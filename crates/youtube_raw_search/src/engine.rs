use crate::{
    Error, Result,
    cache_io::{ColdReader, ColdWriter},
    channel_ids_equal,
    format::{
        BLOCK_RECORD_BYTES, DOCUMENT_RECORD_BYTES, DOCUMENTS_MAGIC, FULL_POSITIONS_MAGIC,
        HEADER_BYTES, LEXICON_MAGIC, METADATA_MAGIC, MappedFile, POSITION_SPOOL_MAGIC,
        POSITIONS_MAGIC, POSTINGS_MAGIC, read_u16, read_u32, read_u64, read_varint, write_header,
        write_varint,
    },
    fuzzy::{FUZZY_FILE, FuzzyCandidate, FuzzyIndex, write_index as write_fuzzy_index},
    positions::{PositionCursor, write_remaining_positions},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use smallvec::SmallVec;
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use youtube_index_store::{ChannelDocument, ChunkStats};
use youtube_search_service::{BackendError, SearchBackend, SearchParams};
use youtube_time::parse_rfc3339_timestamp;

const MANIFEST_FILE: &str = "manifest.json";
const CHANNELS_FILE: &str = "channels.json";
const VIDEOS_FILE: &str = "videos.json";
const LEXICON_FILE: &str = "lexicon.bin";
const POSTINGS_FILE: &str = "postings.bin";
const POSITIONS_FILE: &str = "positions.bin";
const FULL_POSITIONS_FILE: &str = "positions.full.bin";
const POSITION_SPOOL_FILE: &str = ".positions.spool";
const POSTING_SPOOL_DIRECTORY: &str = ".postings.spool";
const BIGRAM_SPOOL_DIRECTORY: &str = ".bigrams.spool";
const POSTING_SHARDS: usize = 256;
const MAX_POSTING_SHARD_BYTES: u64 = 32 * 1024 * 1024;
// Pair records have substantially more decoded map overhead than unigram
// records. Multi-key leaves are capped at 3 MiB so their decoded maps fit the
// 32 MiB run budget. A single pair can be larger than that at corpus scale;
// those leaves are coalesced and encoded through a constant-memory stream.
const MAX_BIGRAM_SHARD_BYTES: u64 = 3 * 1024 * 1024;
const MIN_BIGRAM_TERM_DOCUMENTS: u32 = 50_000;
const DOCUMENT_RECORD_SPOOL_FILE: &str = ".documents.meta.spool";
const DOCUMENTS_FILE: &str = "documents.blocks";
const METADATA_FILE: &str = "documents.meta";
const DOCUMENTS_PER_BLOCK: usize = 64;
const MOMENT_COALESCE_WINDOW_MS: u32 = 20_000;
const METADATA_PREFIX_BYTES: usize = HEADER_BYTES + 8;
const TYPO_RESULT_THRESHOLD: usize = 5;
const DROP_RESULT_THRESHOLD: usize = 1;
const MAX_TOKEN_DROP_CANDIDATES: usize = 20;
const MAX_FUZZY_TERMS_PER_DISTANCE: usize = 4;
const MIN_ONE_TYPO_BYTES: usize = 4;
const MIN_TWO_TYPO_BYTES: usize = 7;
const ENGAGEMENT_RELEVANCE_BOOST: f32 = 0.20;
const VIEW_WEIGHT: f32 = 0.50;
const LIKE_WEIGHT: f32 = 0.35;
const COMMENT_WEIGHT: f32 = 0.15;
const MAX_QUERY_BYTES: usize = 512;
const MAX_FILTER_VALUE_BYTES: usize = 256;
const MAX_PAGE_SIZE: u8 = 250;
const ACTIVE_INCLUDE_FIELDS: &str = "id,video_id,title,text,prev_text,next_text,youtuber_handle,youtuber_name,thumbnail,published_at,duration,t_start_ms,t_end_ms";
const TEXT_QUERY_WEIGHT: f32 = 4.0;
const TITLE_QUERY_WEIGHT: f32 = 6.0;
const POSITION_SCORE_BOOST: f32 = 0.15;
const PROXIMITY_SCORE_BOOST: f32 = 0.20;
const PROXIMITY_POSITION_BOOST: f32 = 0.05;
const ORDERED_PHRASE_MULTIPLIER: f32 = 1.25;
const MAX_PROXIMITY_TOKENS: usize = 10;
const MAX_DIRECT_POSITION_TARGETS: usize = 128;
// Relevance refinement must not turn a broad conjunction into several complete
// positional posting scans. Coverage and BM25 remain exact above this ceiling.
const MAX_EXACT_PROXIMITY_MATCHES: usize = 50_000;
const MAX_PHRASE_TOKENS: usize = 32;
const MAX_PHRASE_SLOP: u8 = 8;
const GENERATION_FILES_SCHEMA_6: [&str; 9] = [
    CHANNELS_FILE,
    VIDEOS_FILE,
    LEXICON_FILE,
    POSTINGS_FILE,
    POSITIONS_FILE,
    FULL_POSITIONS_FILE,
    DOCUMENTS_FILE,
    METADATA_FILE,
    FUZZY_FILE,
];
const GENERATION_FILES_SCHEMA_7: [&str; 11] = [
    CHANNELS_FILE,
    VIDEOS_FILE,
    LEXICON_FILE,
    POSTINGS_FILE,
    POSITIONS_FILE,
    FULL_POSITIONS_FILE,
    DOCUMENTS_FILE,
    METADATA_FILE,
    FUZZY_FILE,
    bigram::TEXT_BIGRAM_FILE,
    bigram::TITLE_BIGRAM_FILE,
];

#[derive(Clone, Debug, Serialize)]
pub struct BuildReport {
    pub documents: u64,
    pub videos: usize,
    pub channels: usize,
    pub terms: usize,
    pub index_bytes: u64,
    pub elapsed_ms: u128,
}

#[derive(Clone, Debug, Serialize)]
pub struct SliceReport {
    pub documents: u64,
    pub videos: usize,
    pub bytes: u64,
}

#[derive(Deserialize)]
struct IndexDocument {
    video_id: String,
    #[serde(default)]
    youtuber_id: String,
    title: String,
    text: String,
    youtuber_handle: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    published_at_timestamp: Option<i64>,
    #[serde(default)]
    view_count: u64,
    #[serde(default)]
    like_count: u64,
    #[serde(default)]
    comment_count: u64,
    #[serde(default)]
    t_start_ms: u32,
}

#[derive(Clone, Deserialize, Serialize)]
struct VideoMeta {
    id: String,
    #[serde(default)]
    youtuber_id: String,
    handle: String,
    title: String,
    published_at_timestamp: i64,
    #[serde(default)]
    view_count: u64,
    #[serde(default)]
    like_count: u64,
    #[serde(default)]
    comment_count: u64,
    documents: Vec<u32>,
}

#[derive(Deserialize, Serialize)]
struct Manifest {
    schema_version: u8,
    documents: u32,
    videos: u32,
    channels: u32,
    terms: u32,
    blocks: u32,
    average_document_tokens: f32,
    max_view_count: u64,
    max_like_count: u64,
    max_comment_count: u64,
    #[serde(default)]
    files: BTreeMap<String, ManifestFile>,
}

#[derive(Deserialize, Serialize)]
struct ManifestFile {
    bytes: u64,
    blake3: String,
}

#[derive(Clone, Copy)]
enum ManifestValidation {
    Full,
    VerifiedByBuilder,
}

mod bigram;
mod builder;
mod conjunction;
mod query_support;
mod single_token;
mod storage;
#[cfg(test)]
mod tests;

#[cfg(test)]
use builder::BuildPosting;
pub(crate) use builder::build_index_from_lines_and_open;
pub use builder::{build_index, slice_corpus};
use query_support::*;
#[cfg(test)]
use single_token::{
    MAX_COMPACT_SPLIT_POSTING_WORK, MAX_COMPACT_SPLIT_PROXIMITY_MATCHES, SplitWorkBudget,
    compact_split_proximity_is_bounded,
};

#[derive(Clone, Copy)]
struct DocumentRecord {
    block: u32,
    offset: u32,
    length: u32,
    video: u32,
    tokens: u32,
    t_start_ms: u32,
}

#[derive(Clone, Copy)]
struct SearchDocumentRecord {
    video: u32,
    tokens: u32,
    t_start_ms: u32,
}

#[derive(Clone, Copy)]
struct BlockRecord {
    offset: u64,
    compressed_length: u32,
    uncompressed_length: u32,
}

struct LexiconBuildEntry {
    term: String,
    text_offset: u64,
    text_length: u32,
    text_frequency: u32,
    title_offset: u64,
    title_length: u32,
    title_frequency: u32,
    fuzzy_title_frequency: u32,
}

#[derive(Clone)]
pub struct RawSearchEngine {
    inner: Arc<EngineInner>,
}

struct EngineInner {
    manifest: Manifest,
    videos: Vec<VideoMeta>,
    channels: Vec<Value>,
    lexicon: Lexicon,
    fuzzy: FuzzyIndex,
    postings: MappedFile,
    positions: PositionIndex,
    full_positions: Option<PositionIndex>,
    text_bigrams: Option<bigram::BigramIndex>,
    title_bigrams: Option<bigram::BigramIndex>,
    documents: MappedFile,
    metadata: DocumentTable,
    index_bytes: u64,
}

struct Lexicon {
    file: MappedFile,
    count: usize,
    entries_start: usize,
}

#[derive(Clone, Copy)]
struct TermEntry {
    term: u32,
    text_offset: usize,
    text_length: usize,
    text_frequency: u32,
    title_offset: usize,
    title_length: usize,
    title_frequency: u32,
}

struct PositionIndex {
    file: MappedFile,
    terms: usize,
    data_start: usize,
}

struct PositionedPostingCursor<'a> {
    postings: &'a [u8],
    posting_cursor: usize,
    first_positions: &'a [u8],
    first_cursor: usize,
    remaining_positions: PositionCursor<'a>,
    previous_id: u32,
    decoded: u32,
    expected: u32,
}

struct PhraseCursor<'a> {
    postings: PositionedPostingCursor<'a>,
    id: Option<u32>,
    positions: Vec<u32>,
}

struct ParsedQuery {
    terms: Vec<String>,
    phrases: Vec<Vec<String>>,
    excluded_terms: Vec<String>,
    excluded_phrases: Vec<Vec<String>>,
}

#[derive(Default)]
struct QueryParser {
    seen: HashSet<String>,
    excluded_seen: HashSet<String>,
    terms: Vec<String>,
    phrases: Vec<Vec<String>>,
    excluded_terms: Vec<String>,
    excluded_phrases: Vec<Vec<String>>,
}

struct PhraseFieldMatches {
    documents: Vec<u32>,
    videos: Vec<u32>,
}

struct PositionTargets {
    documents: Vec<u32>,
    videos: Vec<u32>,
}

#[derive(Clone, Copy)]
struct SearchConstraints<'a> {
    filter: Option<(&'a str, &'a str)>,
    excluded_documents: &'a [u32],
}

struct InitialConjunction {
    matches: Vec<DocMatch>,
    highlights: HighlightGroups,
}

type HighlightGroups = Vec<Vec<String>>;

struct CandidateSearch<'a> {
    constraints: SearchConstraints<'a>,
    initial: Option<InitialConjunction>,
}

#[derive(Clone, Copy)]
struct PositionalWindow {
    displacement: u32,
    max_offset: u32,
}

struct DocumentTable {
    file: MappedFile,
    documents: usize,
    blocks: usize,
    blocks_start: usize,
}

#[derive(Clone, Copy)]
struct DocMatch {
    document: u32,
    score: f32,
    flags: u8,
}

struct VideoMatches {
    video: u32,
    score: f32,
    exact_tokens: u8,
    exact_mask: u8,
    ordered_phrase: bool,
    moments: MomentCounter,
    documents: SmallVec<[DocMatch; 3]>,
}

#[derive(Clone, Copy)]
struct RelevanceRank {
    score: f32,
    exact_tokens: u8,
    exact_mask: u8,
    ordered_phrase: bool,
}

#[derive(Default)]
struct MomentCounter {
    clusters: usize,
    cluster_start_ms: Option<u32>,
    last_start_ms: Option<u32>,
}

impl MomentCounter {
    fn record(&mut self, start_ms: u32) {
        debug_assert!(self.last_start_ms.is_none_or(|last| last <= start_ms));
        self.last_start_ms = Some(start_ms);
        if self
            .cluster_start_ms
            .is_none_or(|cluster| start_ms.saturating_sub(cluster) > MOMENT_COALESCE_WINDOW_MS)
        {
            self.clusters += 1;
            self.cluster_start_ms = Some(start_ms);
        }
    }

    fn visible(&self) -> usize {
        self.clusters.max(1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum IndexedField {
    Text = 0,
    Title = 1,
}

impl IndexedField {
    const fn archive_index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy)]
struct QueryFields {
    text: bool,
    title: bool,
    prioritize_position: bool,
    exact_mask: u8,
}

#[derive(Default)]
struct BlockCache {
    blocks: HashMap<u32, Vec<u8>>,
}

impl SearchBackend for RawSearchEngine {
    fn name(&self) -> &'static str {
        "raw-inverted-index"
    }

    fn search(&self, params: &SearchParams) -> std::result::Result<Value, BackendError> {
        RawSearchEngine::search(self, params).map_err(|error| match error {
            Error::InvalidInput(message) => BackendError::InvalidInput(message),
            error => BackendError::Internal(error.to_string()),
        })
    }

    fn search_channels(&self, params: &SearchParams) -> std::result::Result<Value, BackendError> {
        RawSearchEngine::search_channels(self, params).map_err(|error| match error {
            Error::InvalidInput(message) => BackendError::InvalidInput(message),
            error => BackendError::Internal(error.to_string()),
        })
    }

    fn documents(&self) -> u64 {
        u64::from(self.inner.manifest.documents)
    }

    fn index_bytes(&self) -> u64 {
        self.inner.index_bytes
    }
}

impl RawSearchEngine {
    pub fn document_count(&self) -> u64 {
        u64::from(self.inner.manifest.documents)
    }

    pub fn video_count(&self) -> usize {
        self.inner.videos.len()
    }

    pub fn channel_count(&self) -> usize {
        self.inner.channels.len()
    }

    pub fn term_count(&self) -> usize {
        self.inner.lexicon.count
    }

    pub fn index_size(&self) -> u64 {
        self.inner.index_bytes
    }

    pub fn video_chunk_count(&self, video_id: &str) -> u64 {
        self.inner
            .videos
            .iter()
            .find(|video| video.id == video_id)
            .map_or(0, |video| video.documents.len() as u64)
    }

    pub fn indexed_video_ids(&self, channel_id: &str) -> Vec<String> {
        let mut video_ids = self
            .video_chunk_counts(channel_id)
            .into_iter()
            .map(|(video_id, _)| video_id)
            .collect::<Vec<_>>();
        video_ids.sort_unstable();
        video_ids
    }

    pub fn video_chunk_counts(&self, channel_id: &str) -> Vec<(String, u64)> {
        self.inner
            .videos
            .iter()
            .filter(|video| video.belongs_to(channel_id))
            .map(|video| (video.id.clone(), video.documents.len() as u64))
            .collect()
    }

    pub fn channel_stats(&self, channel_id: &str) -> ChunkStats {
        self.inner
            .videos
            .iter()
            .filter(|video| video.belongs_to(channel_id))
            .fold(ChunkStats::default(), |mut stats, video| {
                stats.indexed_videos = stats.indexed_videos.saturating_add(1);
                stats.indexed_chunks = stats
                    .indexed_chunks
                    .saturating_add(video.documents.len() as u64);
                stats
            })
    }

    pub fn channel_document(&self, channel_id: &str) -> Result<Option<ChannelDocument>> {
        self.inner
            .channels
            .iter()
            .find(|channel| channel.get("id").and_then(Value::as_str) == Some(channel_id))
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(Error::Json)
    }

    pub fn search(&self, params: &SearchParams) -> Result<Value> {
        validate_search_params(params)?;
        let parsed = parse_query(&params.q)?;
        if params.phrase_slop.is_some() && parsed.phrases.is_empty() {
            return Err(Error::InvalidInput(
                "phrase_slop requires at least one quoted phrase".to_string(),
            ));
        }
        let fields = query_fields(
            params.query_by.as_deref(),
            params.prioritize_token_position.unwrap_or(true),
        )?;
        let filter = parse_filter(params.filter_by.as_deref())?;
        let excluded_documents =
            self.excluded_documents(&parsed.excluded_terms, &parsed.excluded_phrases, fields)?;
        let constraints = SearchConstraints {
            filter,
            excluded_documents: &excluded_documents,
        };
        if params.q.trim() == "*" || parsed.terms.is_empty() {
            return self.search_all(params, &excluded_documents);
        }
        if !parsed.phrases.is_empty() {
            return self.search_quoted(params, parsed, fields, &excluded_documents);
        }
        let terms = parsed.terms;

        if terms.len() == 1 {
            return self.search_single_token(params, &terms[0], fields, &excluded_documents);
        }

        // Most successful multi-token searches should never materialize the
        // posting list for a high-frequency token. Start with the rarest token
        // and decode later terms only for surviving document identifiers.
        let mut block_cache = BlockCache::default();
        if filter.is_none()
            && excluded_documents.is_empty()
            && self.exact_terms_are_broad(&terms)?
            && let Some(response) =
                self.try_stream_broad_grouped(params, &terms, fields, &mut block_cache)?
        {
            return Ok(response);
        }
        let (mut fast_matches, fast_highlights) =
            self.exact_prefix_conjunction(&terms, fields, true, filter, &mut block_cache)?;
        retain_not_excluded(&mut fast_matches, &excluded_documents);
        let fast_enough =
            self.reaches_result_threshold(params, &fast_matches, TYPO_RESULT_THRESHOLD)?;
        if fast_enough {
            self.apply_exact_proximity(&terms, fields, &mut fast_matches, &mut block_cache)?;
            return self.format_matches(params, &fast_highlights, fast_matches, &mut block_cache);
        }

        // Typesense includes the last-token prefix in the ordinary query even
        // when the exact conjunction already clears the typo threshold. Typos
        // remain threshold-gated inside this bounded candidate progression.
        // Every expansion remains conjunctive: a document must match one
        // alternative for every retained query token.
        if let Some((matches, highlights)) = self.best_conjunctive_candidate(
            params,
            &terms,
            fields,
            true,
            CandidateSearch {
                constraints,
                initial: Some(InitialConjunction {
                    matches: fast_matches,
                    highlights: fast_highlights,
                }),
            },
            &mut block_cache,
        )? {
            return self.format_matches(params, &highlights, matches, &mut block_cache);
        }

        // Match Typesense's default right_to_left progression. Prefer the longest
        // retained prefix, then try retained suffixes where prefix search remains
        // useful. Bound both ladders so adversarial long queries cannot multiply
        // fuzzy work without limit.
        for retained in (1..terms.len()).rev().take(MAX_TOKEN_DROP_CANDIDATES) {
            if let Some((matches, highlights)) = self.best_conjunctive_candidate(
                params,
                &terms[..retained],
                fields,
                false,
                CandidateSearch {
                    constraints,
                    initial: None,
                },
                &mut block_cache,
            )? {
                return self.format_matches(params, &highlights, matches, &mut block_cache);
            }
        }
        for dropped in 1..terms.len().min(MAX_TOKEN_DROP_CANDIDATES + 1) {
            if let Some((matches, highlights)) = self.best_conjunctive_candidate(
                params,
                &terms[dropped..],
                fields,
                true,
                CandidateSearch {
                    constraints,
                    initial: None,
                },
                &mut block_cache,
            )? {
                return self.format_matches(params, &highlights, matches, &mut block_cache);
            }
        }

        self.format_conjunctive(params, terms, Vec::new())
    }

    fn search_quoted(
        &self,
        params: &SearchParams,
        parsed: ParsedQuery,
        fields: QueryFields,
        excluded_documents: &[u32],
    ) -> Result<Value> {
        if self.inner.full_positions.is_none() {
            return Err(Error::InvalidInput(
                "quoted phrase search requires a schema v6 index".to_string(),
            ));
        }
        let exact_term_matches = self.exact_term_matches(&parsed.terms, fields)?;
        let mut matches = intersect_all_borrowed(&exact_term_matches);
        retain_not_excluded(&mut matches, excluded_documents);
        let phrase_slop = params.phrase_slop.unwrap_or(0);
        for phrase in &parsed.phrases {
            let targets = self.position_targets(&matches)?;
            let phrase_matches =
                self.phrase_field_matches(phrase, fields, &targets, phrase_slop)?;
            matches = self.retain_position_phrase_matches(matches, &phrase_matches)?;
        }
        self.format_conjunctive(params, parsed.terms, matches)
    }

    fn retain_position_phrase_matches(
        &self,
        matches: Vec<DocMatch>,
        phrase_matches: &PhraseFieldMatches,
    ) -> Result<Vec<DocMatch>> {
        let mut retained = Vec::with_capacity(matches.len());
        for candidate in matches {
            let text_phrase = candidate.matches_text()
                && phrase_matches
                    .documents
                    .binary_search(&candidate.document())
                    .is_ok();
            let record = self.inner.metadata.document(candidate.document())?;
            let title_phrase = phrase_matches.videos.binary_search(&record.video).is_ok();
            if text_phrase || title_phrase {
                retained.push(candidate);
            }
        }
        Ok(retained)
    }

    fn phrase_field_matches(
        &self,
        phrase: &[String],
        fields: QueryFields,
        targets: &PositionTargets,
        slop: u8,
    ) -> Result<PhraseFieldMatches> {
        Ok(PhraseFieldMatches {
            documents: if fields.text {
                self.phrase_ids_for_field(phrase, IndexedField::Text, &targets.documents, slop)?
            } else {
                Vec::new()
            },
            videos: if fields.title {
                self.phrase_ids_for_field(phrase, IndexedField::Title, &targets.videos, slop)?
            } else {
                Vec::new()
            },
        })
    }

    fn phrase_ids_for_field(
        &self,
        phrase: &[String],
        field: IndexedField,
        targets: &[u32],
        slop: u8,
    ) -> Result<Vec<u32>> {
        self.evaluate_positions_for_field(phrase, field, targets, |target, cursors| {
            ordered_phrase_positions_match(cursors, slop).then_some(target)
        })
    }

    fn position_windows_for_field(
        &self,
        terms: &[String],
        field: IndexedField,
        targets: &[u32],
    ) -> Result<Vec<(u32, PositionalWindow)>> {
        self.evaluate_positions_for_field(terms, field, targets, |target, cursors| {
            minimum_position_window(cursors).map(|window| (target, window))
        })
    }

    fn position_targets(&self, matches: &[DocMatch]) -> Result<PositionTargets> {
        let mut documents = Vec::with_capacity(matches.len());
        let mut videos = Vec::with_capacity(matches.len());
        for candidate in matches {
            if candidate.matches_text() {
                documents.push(candidate.document());
            }
            videos.push(self.inner.metadata.document(candidate.document())?.video);
        }
        documents.sort_unstable();
        documents.dedup();
        videos.sort_unstable();
        videos.dedup();
        Ok(PositionTargets { documents, videos })
    }

    fn apply_exact_proximity(
        &self,
        terms: &[String],
        fields: QueryFields,
        matches: &mut [DocMatch],
        cache: &mut BlockCache,
    ) -> Result<()> {
        if terms.len() < 2
            || matches.is_empty()
            || !exact_proximity_is_bounded(matches.len())
            || self.inner.full_positions.is_none()
        {
            return Ok(());
        }
        if matches.len() <= MAX_DIRECT_POSITION_TARGETS {
            return self.apply_exact_proximity_from_documents(terms, fields, matches, cache);
        }
        if self.try_apply_exact_bigram(terms, fields, matches)? {
            // Schema 7 makes exact adjacency and its earliest offset authoritative for
            // large two-token result sets. Non-adjacent candidates retain their BM25
            // score instead of faulting the complete positional streams back into RAM.
            return Ok(());
        }
        self.apply_exact_proximity_from_postings(terms, fields, matches)
    }

    fn try_apply_exact_bigram(
        &self,
        terms: &[String],
        fields: QueryFields,
        matches: &mut [DocMatch],
    ) -> Result<bool> {
        if terms.len() != 2 {
            return Ok(false);
        }
        let (Some(text_index), Some(title_index)) =
            (&self.inner.text_bigrams, &self.inner.title_bigrams)
        else {
            return Ok(false);
        };
        let Some(left) = self.inner.lexicon.lookup(&terms[0])? else {
            return Ok(false);
        };
        let Some(right) = self.inner.lexicon.lookup(&terms[1])? else {
            return Ok(false);
        };
        let key = bigram::BigramKey::new(left.term, right.term);
        // The sidecar records the complete eligible-term set independently of
        // observed pairs. For a covered pair, a missing key is therefore proof
        // of no exact adjacency, not a reason to switch ranking algorithms.
        if !text_index.covers(key)? || !title_index.covers(key)? {
            return Ok(false);
        }
        let targets = self.position_targets(matches)?;
        let mut text = Vec::new();
        let mut title = Vec::new();
        if fields.text && !targets.documents.is_empty() {
            text_index.lookup_targets_into(key, &targets.documents, &mut text)?;
        }
        if fields.title && !targets.videos.is_empty() {
            title_index.lookup_targets_into(key, &targets.videos, &mut title)?;
        }
        apply_bigram_proximity(
            matches,
            &self.inner.metadata,
            &text,
            &title,
            fields.prioritize_position,
        )?;
        Ok(true)
    }

    fn apply_exact_proximity_from_postings(
        &self,
        terms: &[String],
        fields: QueryFields,
        matches: &mut [DocMatch],
    ) -> Result<()> {
        let terms = &terms[..terms.len().min(MAX_PROXIMITY_TOKENS)];
        let targets = self.position_targets(matches)?;
        let text_windows = if fields.text {
            self.position_windows_for_field(terms, IndexedField::Text, &targets.documents)?
        } else {
            Vec::new()
        };
        let title_windows = if fields.title {
            self.position_windows_for_field(terms, IndexedField::Title, &targets.videos)?
        } else {
            Vec::new()
        };
        let ordered_phrases = self.phrase_field_matches(terms, fields, &targets, 0)?;
        for candidate in matches {
            let text_multiplier = candidate
                .matches_text()
                .then(|| positional_window(&text_windows, candidate.document()))
                .flatten()
                .map(|window| proximity_multiplier(window, terms.len(), fields.prioritize_position))
                .unwrap_or(1.0);
            let record = self.inner.metadata.document(candidate.document())?;
            let title_multiplier = positional_window(&title_windows, record.video)
                .map(|window| proximity_multiplier(window, terms.len(), fields.prioritize_position))
                .unwrap_or(1.0);
            let ordered = ordered_phrases
                .documents
                .binary_search(&candidate.document())
                .is_ok()
                || ordered_phrases.videos.binary_search(&record.video).is_ok();
            candidate.score *= text_multiplier.max(title_multiplier)
                * if ordered {
                    ORDERED_PHRASE_MULTIPLIER
                } else {
                    1.0
                };
            candidate.set_ordered_phrase(ordered);
        }
        Ok(())
    }

    fn apply_exact_proximity_from_documents(
        &self,
        terms: &[String],
        fields: QueryFields,
        matches: &mut [DocMatch],
        cache: &mut BlockCache,
    ) -> Result<()> {
        let terms = &terms[..terms.len().min(MAX_PROXIMITY_TOKENS)];
        for candidate in matches {
            let record = self.inner.metadata.document(candidate.document())?;
            let video = self
                .inner
                .videos
                .get(record.video as usize)
                .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
            let document = self.document_value(candidate.document(), cache)?;
            let text_summary = if fields.text && candidate.matches_text() {
                document
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|value| positional_summary(value, terms))
            } else {
                None
            };
            let title_summary = fields
                .title
                .then(|| positional_summary(&video.title, terms));
            let text_multiplier = text_summary
                .and_then(|summary| summary.window)
                .map(|window| proximity_multiplier(window, terms.len(), fields.prioritize_position))
                .unwrap_or(1.0);
            let title_multiplier = title_summary
                .and_then(|summary| summary.window)
                .map(|window| proximity_multiplier(window, terms.len(), fields.prioritize_position))
                .unwrap_or(1.0);
            let ordered = text_summary.is_some_and(|summary| summary.ordered)
                || title_summary.is_some_and(|summary| summary.ordered);
            candidate.score *= text_multiplier.max(title_multiplier)
                * if ordered {
                    ORDERED_PHRASE_MULTIPLIER
                } else {
                    1.0
                };
            candidate.set_ordered_phrase(ordered);
        }
        Ok(())
    }

    fn evaluate_positions_for_field<T, F>(
        &self,
        terms: &[String],
        field: IndexedField,
        targets: &[u32],
        mut evaluate: F,
    ) -> Result<Vec<T>>
    where
        F: FnMut(u32, &[PhraseCursor<'_>]) -> Option<T>,
    {
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let full_positions = self.inner.full_positions.as_ref().ok_or_else(|| {
            Error::InvalidInput("phrase search requires complete positions".to_string())
        })?;
        let mut cursors = Vec::with_capacity(terms.len());
        for term in terms {
            let Some(entry) = self.inner.lexicon.lookup(term)? else {
                return Ok(Vec::new());
            };
            let (offset, length, frequency) = match field {
                IndexedField::Text => (entry.text_offset, entry.text_length, entry.text_frequency),
                IndexedField::Title => (
                    entry.title_offset,
                    entry.title_length,
                    entry.title_frequency,
                ),
            };
            if frequency == 0 {
                return Ok(Vec::new());
            }
            cursors.push(PhraseCursor {
                postings: PositionedPostingCursor::new(
                    self.postings_range(offset, length)?,
                    self.inner.positions.range(entry.term, field)?,
                    full_positions.range(entry.term, field)?,
                    frequency,
                ),
                id: None,
                positions: Vec::new(),
            });
        }
        let mut output = Vec::new();
        for &target in targets {
            for cursor in &mut cursors {
                if cursor.id.is_none_or(|id| id < target) {
                    cursor.advance_to(target)?;
                }
                if cursor.id.is_none() {
                    return Ok(output);
                }
            }
            if cursors.iter().any(|cursor| cursor.id != Some(target)) {
                continue;
            }
            if let Some(value) = evaluate(target, &cursors) {
                output.push(value);
            }
        }
        Ok(output)
    }

    fn best_conjunctive_candidate(
        &self,
        params: &SearchParams,
        terms: &[String],
        fields: QueryFields,
        prefix_last: bool,
        mut search: CandidateSearch<'_>,
        block_cache: &mut BlockCache,
    ) -> Result<Option<(Vec<DocMatch>, HighlightGroups)>> {
        let maximum_distance = terms
            .iter()
            .map(|term| bounded_typo_distance(term))
            .max()
            .unwrap_or(0);
        for typo_distance in 0..=maximum_distance {
            let (mut matches, highlight_terms) = if typo_distance == 0 {
                match search.initial.take() {
                    Some(initial) => (initial.matches, initial.highlights),
                    None => {
                        self.fuzzy_prefix_conjunction(
                            terms,
                            fields,
                            prefix_last,
                            0,
                            search.constraints.filter,
                            block_cache,
                        )
                    }?,
                }
            } else {
                self.fuzzy_prefix_conjunction(
                    terms,
                    fields,
                    prefix_last,
                    typo_distance,
                    search.constraints.filter,
                    block_cache,
                )?
            };
            retain_not_excluded(&mut matches, search.constraints.excluded_documents);
            let enough = self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)?;
            if enough {
                self.apply_exact_proximity(terms, fields, &mut matches, block_cache)?;
                return Ok(Some((matches, highlight_terms)));
            }
            if typo_distance == maximum_distance {
                let has_complete_exact_match = matches
                    .iter()
                    .any(|candidate| candidate.all_retained_exact());
                let has_sparse_result =
                    self.reaches_result_threshold(params, &matches, DROP_RESULT_THRESHOLD)?;
                return if has_complete_exact_match && has_sparse_result {
                    self.apply_exact_proximity(terms, fields, &mut matches, block_cache)?;
                    Ok(Some((matches, highlight_terms)))
                } else {
                    Ok(None)
                };
            }
        }
        Ok(None)
    }

    fn exact_term_matches(
        &self,
        terms: &[String],
        fields: QueryFields,
    ) -> Result<Vec<Vec<DocMatch>>> {
        let mut term_matches = Vec::with_capacity(terms.len());
        for (index, term) in terms.iter().enumerate() {
            let exact_mask = 1 << (3 - index.min(3));
            term_matches.push(self.matches_for_term(
                term,
                QueryFields {
                    exact_mask,
                    ..fields
                },
            )?);
        }
        Ok(term_matches)
    }

    fn excluded_documents(
        &self,
        terms: &[String],
        phrases: &[Vec<String>],
        fields: QueryFields,
    ) -> Result<Vec<u32>> {
        let mut excluded = if terms.is_empty() {
            Vec::new()
        } else {
            let mut lists = Vec::with_capacity(terms.len());
            for term in terms {
                lists.push(self.matches_for_term(
                    term,
                    QueryFields {
                        prioritize_position: false,
                        exact_mask: 0,
                        ..fields
                    },
                )?);
            }
            union_all(lists)
                .into_iter()
                .map(DocMatch::document)
                .collect()
        };
        if !phrases.is_empty() && self.inner.full_positions.is_none() {
            return Err(Error::InvalidInput(
                "negative phrase search requires a schema v6 index".to_string(),
            ));
        }
        for phrase in phrases {
            excluded = union_sorted_ids(excluded, self.excluded_phrase_documents(phrase, fields)?);
        }
        Ok(excluded)
    }

    fn excluded_phrase_documents(
        &self,
        phrase: &[String],
        fields: QueryFields,
    ) -> Result<Vec<u32>> {
        let mut term_matches = Vec::with_capacity(phrase.len());
        for (index, term) in phrase.iter().enumerate() {
            if phrase[..index].contains(term) {
                continue;
            }
            term_matches.push(self.matches_for_term(
                term,
                QueryFields {
                    exact_mask: 0,
                    ..fields
                },
            )?);
        }
        let candidates = intersect_all_borrowed(&term_matches);
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let targets = self.position_targets(&candidates)?;
        let phrase_matches = self.phrase_field_matches(phrase, fields, &targets, 0)?;
        Ok(self
            .retain_position_phrase_matches(candidates, &phrase_matches)?
            .into_iter()
            .map(DocMatch::document)
            .collect())
    }

    fn format_conjunctive(
        &self,
        params: &SearchParams,
        highlight_terms: Vec<String>,
        matches: Vec<DocMatch>,
    ) -> Result<Value> {
        let mut cache = BlockCache::default();
        let highlight_groups = highlight_terms
            .into_iter()
            .map(|term| vec![term])
            .collect::<HighlightGroups>();
        self.format_matches(params, &highlight_groups, matches, &mut cache)
    }

    fn matches_for_term(&self, term: &str, fields: QueryFields) -> Result<Vec<DocMatch>> {
        let Some(entry) = self.inner.lexicon.lookup(term)? else {
            return Ok(Vec::new());
        };
        self.matches_for_entry(entry, fields)
    }

    fn matches_for_entry(&self, entry: TermEntry, fields: QueryFields) -> Result<Vec<DocMatch>> {
        let average_documents_per_video = self
            .inner
            .manifest
            .documents
            .checked_div(self.inner.manifest.videos)
            .unwrap_or(0);
        let effective_title_documents = entry
            .title_frequency
            .saturating_mul(average_documents_per_video);
        let effective_frequency = entry
            .text_frequency
            .saturating_add(effective_title_documents)
            .min(self.inner.manifest.documents);
        let idf =
            inverse_document_frequency(self.inner.manifest.documents, effective_frequency.max(1));
        let text = if fields.text {
            self.decode_text_postings(entry, idf, fields.prioritize_position, fields.exact_mask)?
        } else {
            Vec::new()
        };
        let title = if fields.title {
            self.decode_title_postings(entry, idf, fields.prioritize_position, fields.exact_mask)?
        } else {
            Vec::new()
        };
        Ok(union_sorted(text, title))
    }

    fn decode_text_postings(
        &self,
        entry: TermEntry,
        idf: f32,
        prioritize_position: bool,
        exact_mask: u8,
    ) -> Result<Vec<DocMatch>> {
        let bytes = self.postings_range(entry.text_offset, entry.text_length)?;
        let position_bytes = prioritize_position
            .then(|| self.inner.positions.range(entry.term, IndexedField::Text))
            .transpose()?;
        let mut matches = Vec::with_capacity(entry.text_frequency as usize);
        let mut cursor = 0_usize;
        let mut position_cursor = 0_usize;
        let mut document = 0_u32;
        while cursor < bytes.len() {
            document = document.saturating_add(read_varint(bytes, &mut cursor, bytes.len())?);
            let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
            let first_position = position_bytes
                .map(|positions| read_varint(positions, &mut position_cursor, positions.len()))
                .transpose()?
                .unwrap_or_default();
            let record = self.inner.metadata.document(document)?;
            let score = bm25(
                frequency,
                record.tokens,
                self.inner.manifest.average_document_tokens,
            ) * idf
                * TEXT_QUERY_WEIGHT
                * token_position_multiplier(first_position, prioritize_position);
            matches.push(DocMatch::with_exact_mask(document, score, true, exact_mask));
        }
        if matches.len() != entry.text_frequency as usize
            || position_bytes.is_some_and(|positions| position_cursor != positions.len())
        {
            return Err(Error::Corrupt(
                "text posting/position count does not match lexicon".to_string(),
            ));
        }
        Ok(matches)
    }

    fn decode_title_postings(
        &self,
        entry: TermEntry,
        idf: f32,
        prioritize_position: bool,
        exact_mask: u8,
    ) -> Result<Vec<DocMatch>> {
        let bytes = self.postings_range(entry.title_offset, entry.title_length)?;
        let position_bytes = prioritize_position
            .then(|| self.inner.positions.range(entry.term, IndexedField::Title))
            .transpose()?;
        let average_docs = self
            .inner
            .manifest
            .documents
            .checked_div(self.inner.manifest.videos)
            .unwrap_or(0);
        let mut matches =
            Vec::with_capacity((entry.title_frequency.saturating_mul(average_docs)) as usize);
        let mut cursor = 0_usize;
        let mut position_cursor = 0_usize;
        let mut video_id = 0_u32;
        let mut postings = 0_u32;
        while cursor < bytes.len() {
            video_id = video_id.saturating_add(read_varint(bytes, &mut cursor, bytes.len())?);
            let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
            let first_position = position_bytes
                .map(|positions| read_varint(positions, &mut position_cursor, positions.len()))
                .transpose()?
                .unwrap_or_default();
            let video = self.inner.videos.get(video_id as usize).ok_or_else(|| {
                Error::Corrupt("title posting video is out of bounds".to_string())
            })?;
            let score = idf
                * TITLE_QUERY_WEIGHT
                * (1.0 + (frequency as f32).ln())
                * token_position_multiplier(first_position, prioritize_position);
            for &document in &video.documents {
                matches.push(DocMatch::with_exact_mask(
                    document, score, false, exact_mask,
                ));
            }
            postings += 1;
        }
        if postings != entry.title_frequency
            || position_bytes.is_some_and(|positions| position_cursor != positions.len())
        {
            return Err(Error::Corrupt(
                "title posting/position count does not match lexicon".to_string(),
            ));
        }
        Ok(matches)
    }

    fn postings_range(&self, offset: usize, length: usize) -> Result<&[u8]> {
        let end = offset
            .checked_add(length)
            .ok_or_else(|| Error::Corrupt("postings range overflows".to_string()))?;
        self.inner
            .postings
            .bytes()
            .get(offset..end)
            .ok_or_else(|| Error::Corrupt("postings range extends beyond file".to_string()))
    }

    fn fuzzy_alternatives(
        &self,
        term: &str,
        fields: QueryFields,
        maximum_distance: u8,
        exact_distance: Option<u8>,
        prefix_only: bool,
    ) -> Result<(Vec<DocMatch>, Vec<String>)> {
        let mut alternatives = Vec::new();
        let mut highlight_terms = Vec::new();
        let candidates = self.inner.fuzzy.candidates(
            term,
            maximum_distance,
            MAX_FUZZY_TERMS_PER_DISTANCE,
            true,
            fields.text,
            fields.title,
        )?;
        for FuzzyCandidate {
            term: term_id,
            distance,
            exact,
        } in candidates
        {
            if exact
                || (prefix_only && distance != 0)
                || (!prefix_only && exact_distance.is_some_and(|expected| distance != expected))
            {
                continue;
            }
            let (candidate, entry) = self.inner.lexicon.entry_at(term_id as usize)?;
            // Edit distance already supplies the fallback's relevance demotion.
            // Avoid decoding a second posting stream for every fuzzy candidate.
            let mut matches = self.matches_for_entry(
                entry,
                QueryFields {
                    prioritize_position: false,
                    exact_mask: 0,
                    ..fields
                },
            )?;
            let score_multiplier = match distance {
                0 => 0.9,
                1 => 0.65,
                _ => 0.4,
            };
            for matched in &mut matches {
                matched.score *= score_multiplier;
            }
            alternatives.push(matches);
            highlight_terms.push(candidate.to_string());
        }
        Ok((union_all(alternatives), highlight_terms))
    }

    fn retain_phrase_matches(
        &self,
        matches: &mut Vec<DocMatch>,
        phrase: &str,
        cache: &mut BlockCache,
    ) -> Result<()> {
        let phrase = phrase.to_ascii_lowercase();
        let mut retained = Vec::with_capacity(matches.len());
        for candidate in matches.drain(..) {
            let document = self.document_value(candidate.document(), cache)?;
            let title_matches = document
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|value| value.to_ascii_lowercase().contains(&phrase));
            let text_matches = document
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|value| value.to_ascii_lowercase().contains(&phrase));
            if title_matches || text_matches {
                let mut candidate = candidate;
                candidate.score += 8.0;
                candidate.set_matches_text(text_matches);
                retained.push(candidate);
            }
        }
        *matches = retained;
        Ok(())
    }

    fn try_stream_broad_grouped(
        &self,
        params: &SearchParams,
        terms: &[String],
        fields: QueryFields,
        cache: &mut BlockCache,
    ) -> Result<Option<Value>> {
        if params.group_by.as_deref() != Some("video_id") || terms.len() != 2 {
            return Ok(None);
        }
        let (mut plans, highlights) = self.conjunction_plans(terms, fields, true, 0)?;
        plans.sort_unstable_by_key(|plan| plan.estimated_documents);
        if !conjunction::is_broad_two_plan(&plans) {
            return Ok(None);
        }

        let hits_per_group = usize::from(params.group_limit.unwrap_or(5).max(1));
        let full_exact_mask = plans
            .iter()
            .flat_map(|plan| &plan.alternatives)
            .fold(0_u8, |mask, alternative| {
                mask | alternative.fields.exact_mask
            });
        let mut groups = self.empty_group_slots();
        let mut documents = 0_usize;
        self.visit_two_plan_conjunction(&plans, |candidate, record| {
            documents = documents.saturating_add(1);
            self.record_group_match_values(
                &mut groups,
                candidate,
                record.video,
                record.t_start_ms,
                hits_per_group,
                Some(full_exact_mask),
            )
        })?;
        let found = groups.iter().flatten().count();
        if documents <= MAX_EXACT_PROXIMITY_MATCHES || found < TYPO_RESULT_THRESHOLD {
            return Ok(None);
        }
        self.format_group_slots(params, &highlights, groups, cache)
            .map(Some)
    }

    fn empty_group_slots(&self) -> Vec<Option<VideoMatches>> {
        let mut groups = Vec::new();
        groups.resize_with(self.inner.videos.len(), || None);
        groups
    }

    fn record_group_match(
        &self,
        groups: &mut [Option<VideoMatches>],
        candidate: DocMatch,
        record: DocumentRecord,
        hits_per_group: usize,
    ) -> Result<()> {
        self.record_group_match_values(
            groups,
            candidate,
            record.video,
            record.t_start_ms,
            hits_per_group,
            None,
        )
    }

    fn record_group_match_values(
        &self,
        groups: &mut [Option<VideoMatches>],
        candidate: DocMatch,
        video: u32,
        t_start_ms: u32,
        hits_per_group: usize,
        dominant_exact_mask: Option<u8>,
    ) -> Result<()> {
        let slot = groups
            .get_mut(video as usize)
            .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
        let group = slot.get_or_insert_with(|| VideoMatches {
            video,
            score: candidate.score,
            exact_tokens: candidate.exact_tokens(),
            exact_mask: candidate.exact_mask(),
            ordered_phrase: candidate.ordered_phrase(),
            moments: MomentCounter::default(),
            documents: SmallVec::new(),
        });
        if candidate.matches_text() {
            group.moments.record(t_start_ms);
        }
        if dominant_exact_mask.is_some_and(|mask| {
            candidate.exact_mask() != mask
                && group.exact_mask == mask
                && group.documents.len() == hits_per_group
                && group
                    .documents
                    .last()
                    .is_some_and(|document| document.exact_mask() == mask)
        }) {
            return Ok(());
        }
        let candidate_rank = (
            candidate.exact_tokens(),
            candidate.exact_mask(),
            candidate.ordered_phrase(),
            candidate.score,
        );
        let group_rank = (
            group.exact_tokens,
            group.exact_mask,
            group.ordered_phrase,
            group.score,
        );
        if candidate_rank.0 > group_rank.0
            || (candidate_rank.0 == group_rank.0 && candidate_rank.1 > group_rank.1)
            || (candidate_rank.0 == group_rank.0
                && candidate_rank.1 == group_rank.1
                && candidate_rank.2
                && !group_rank.2)
            || (candidate_rank.0 == group_rank.0
                && candidate_rank.1 == group_rank.1
                && candidate_rank.2 == group_rank.2
                && candidate_rank.3 > group_rank.3)
        {
            group.exact_tokens = candidate_rank.0;
            group.exact_mask = candidate_rank.1;
            group.ordered_phrase = candidate_rank.2;
            group.score = candidate_rank.3;
        }
        retain_best_document(&mut group.documents, candidate, hits_per_group);
        Ok(())
    }

    fn format_group_slots(
        &self,
        params: &SearchParams,
        highlight_groups: &[Vec<String>],
        groups: Vec<Option<VideoMatches>>,
        cache: &mut BlockCache,
    ) -> Result<Value> {
        let found = groups.iter().flatten().count();
        let total_moments = groups
            .iter()
            .flatten()
            .map(visible_group_moments)
            .sum::<usize>();
        let mut groups = groups.into_iter().flatten().collect::<Vec<_>>();
        let (offset, limit) = result_window(params);
        self.sort_group_window(
            &mut groups,
            params.sort_by.as_deref(),
            offset.saturating_add(limit),
        );
        let mut grouped_hits = Vec::with_capacity(limit.min(groups.len()));
        for mut group in groups.into_iter().skip(offset).take(limit) {
            let group_found = visible_group_moments(&group);
            group.documents.sort_unstable_by(descending_doc_score);
            let video_id = self.inner.videos[group.video as usize].id.clone();
            let mut hits = Vec::with_capacity(group.documents.len());
            for document in group.documents {
                hits.push(self.hit_value(document.document(), highlight_groups, cache)?);
            }
            grouped_hits.push(json!({
                "group_key": [video_id],
                "found": group_found,
                "hits": hits,
            }));
        }
        Ok(json!({
            "found": found,
            "total_moments": total_moments,
            "out_of": self.inner.manifest.documents,
            "grouped_hits": grouped_hits,
        }))
    }

    fn format_matches(
        &self,
        params: &SearchParams,
        highlight_groups: &[Vec<String>],
        matches: Vec<DocMatch>,
        cache: &mut BlockCache,
    ) -> Result<Value> {
        if params.group_by.as_deref() != Some("video_id") {
            return self.format_ungrouped(params, highlight_groups, matches, cache);
        }

        let hits_per_group = usize::from(params.group_limit.unwrap_or(5).max(1));
        let filter = parse_filter(params.filter_by.as_deref())?;
        // Video identifiers are dense generation-local integers. A direct slot
        // table avoids hashing every matching transcript chunk on broad queries.
        let mut groups = self.empty_group_slots();
        for candidate in matches {
            let record = self.inner.metadata.document(candidate.document())?;
            let Some(video) = self.inner.videos.get(record.video as usize) else {
                return Err(Error::Corrupt(
                    "document video is out of bounds".to_string(),
                ));
            };
            if !video_matches_filter(video, filter) {
                continue;
            }
            self.record_group_match(&mut groups, candidate, record, hits_per_group)?;
        }
        self.format_group_slots(params, highlight_groups, groups, cache)
    }

    fn format_ungrouped(
        &self,
        params: &SearchParams,
        highlight_groups: &[Vec<String>],
        matches: Vec<DocMatch>,
        cache: &mut BlockCache,
    ) -> Result<Value> {
        let filter = parse_filter(params.filter_by.as_deref())?;
        let mut filtered = Vec::with_capacity(matches.len());
        let mut moment_counts = HashMap::<u32, MomentCounter>::new();
        for candidate in matches {
            let record = self.inner.metadata.document(candidate.document())?;
            let video = self
                .inner
                .videos
                .get(record.video as usize)
                .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
            if video_matches_filter(video, filter) {
                if candidate.matches_text() {
                    moment_counts
                        .entry(record.video)
                        .or_default()
                        .record(record.t_start_ms);
                } else {
                    moment_counts.entry(record.video).or_default();
                }
                filtered.push(candidate);
            }
        }
        let total_moments = moment_counts
            .values()
            .map(MomentCounter::visible)
            .sum::<usize>();
        let mut matches = filtered;
        matches.sort_unstable_by(|left, right| {
            let left_video = self.video_for_document(left.document());
            let right_video = self.video_for_document(right.document());
            match (left_video, right_video) {
                (Some(left_video), Some(right_video)) => compare_video_sort(
                    left_video,
                    RelevanceRank {
                        score: left.score,
                        exact_tokens: left.exact_tokens(),
                        exact_mask: left.exact_mask(),
                        ordered_phrase: left.ordered_phrase(),
                    },
                    right_video,
                    RelevanceRank {
                        score: right.score,
                        exact_tokens: right.exact_tokens(),
                        exact_mask: right.exact_mask(),
                        ordered_phrase: right.ordered_phrase(),
                    },
                    params.sort_by.as_deref(),
                    &self.inner.manifest,
                )
                .then_with(|| left.document().cmp(&right.document())),
                _ => left.document().cmp(&right.document()),
            }
        });
        let found = matches.len();
        let (offset, limit) = result_window(params);
        let mut hits = Vec::with_capacity(limit.min(matches.len()));
        for candidate in matches.into_iter().skip(offset).take(limit) {
            hits.push(self.hit_value(candidate.document(), highlight_groups, cache)?);
        }
        Ok(json!({
            "found": found,
            "total_moments": total_moments,
            "out_of": self.inner.manifest.documents,
            "hits": hits,
        }))
    }

    fn sort_group_window(
        &self,
        groups: &mut [VideoMatches],
        sort_by: Option<&str>,
        required: usize,
    ) {
        let compare = |left: &VideoMatches, right: &VideoMatches| {
            let left_video = &self.inner.videos[left.video as usize];
            let right_video = &self.inner.videos[right.video as usize];
            compare_video_sort(
                left_video,
                RelevanceRank {
                    score: left.score,
                    exact_tokens: left.exact_tokens,
                    exact_mask: left.exact_mask,
                    ordered_phrase: left.ordered_phrase,
                },
                right_video,
                RelevanceRank {
                    score: right.score,
                    exact_tokens: right.exact_tokens,
                    exact_mask: right.exact_mask,
                    ordered_phrase: right.ordered_phrase,
                },
                sort_by,
                &self.inner.manifest,
            )
        };
        let required = required.min(groups.len());
        if required == 0 {
            return;
        }
        if required < groups.len() {
            groups.select_nth_unstable_by(required, &compare);
        }
        groups[..required].sort_unstable_by(compare);
    }

    fn search_all(&self, params: &SearchParams, excluded_documents: &[u32]) -> Result<Value> {
        let filter = parse_filter(params.filter_by.as_deref())?;
        let (offset, limit) = result_window(params);
        let mut cache = BlockCache::default();
        if params.group_by.as_deref() == Some("video_id") {
            let mut videos = self
                .inner
                .videos
                .iter()
                .enumerate()
                .filter(|(_, video)| {
                    video_matches_filter(video, filter)
                        && video
                            .documents
                            .iter()
                            .any(|document| excluded_documents.binary_search(document).is_err())
                })
                .map(|(index, _)| index as u32)
                .collect::<Vec<_>>();
            videos.sort_unstable_by(|left, right| {
                let left = &self.inner.videos[*left as usize];
                let right = &self.inner.videos[*right as usize];
                compare_video_sort(
                    left,
                    RelevanceRank {
                        score: 0.0,
                        exact_tokens: 0,
                        exact_mask: 0,
                        ordered_phrase: false,
                    },
                    right,
                    RelevanceRank {
                        score: 0.0,
                        exact_tokens: 0,
                        exact_mask: 0,
                        ordered_phrase: false,
                    },
                    params
                        .sort_by
                        .as_deref()
                        .or(Some("published_at_timestamp:desc")),
                    &self.inner.manifest,
                )
            });
            let found = videos.len();
            let hits_per_group = usize::from(params.group_limit.unwrap_or(5).max(1));
            let mut grouped_hits = Vec::with_capacity(limit.min(videos.len()));
            for video_id in videos.into_iter().skip(offset).take(limit) {
                let video = &self.inner.videos[video_id as usize];
                let mut hits = Vec::with_capacity(hits_per_group);
                for &document in video
                    .documents
                    .iter()
                    .filter(|document| excluded_documents.binary_search(document).is_err())
                    .take(hits_per_group)
                {
                    hits.push(self.hit_value(document, &[], &mut cache)?);
                }
                grouped_hits.push(json!({
                    "group_key": [video.id],
                    "found": 1,
                    "hits": hits,
                }));
            }
            return Ok(json!({
                "found": found,
                "total_moments": found,
                "out_of": self.inner.manifest.documents,
                "grouped_hits": grouped_hits,
            }));
        }

        let mut documents = (0..self.inner.manifest.documents)
            .filter(|document| {
                excluded_documents.binary_search(document).is_err()
                    && self
                        .inner
                        .metadata
                        .document(*document)
                        .ok()
                        .and_then(|record| self.inner.videos.get(record.video as usize))
                        .is_some_and(|video| video_matches_filter(video, filter))
            })
            .collect::<Vec<_>>();
        documents.sort_unstable_by(|left, right| {
            let left_video = self.video_for_document(*left);
            let right_video = self.video_for_document(*right);
            match (left_video, right_video) {
                (Some(left_video), Some(right_video)) => compare_video_sort(
                    left_video,
                    RelevanceRank {
                        score: 0.0,
                        exact_tokens: 0,
                        exact_mask: 0,
                        ordered_phrase: false,
                    },
                    right_video,
                    RelevanceRank {
                        score: 0.0,
                        exact_tokens: 0,
                        exact_mask: 0,
                        ordered_phrase: false,
                    },
                    params
                        .sort_by
                        .as_deref()
                        .or(Some("published_at_timestamp:desc")),
                    &self.inner.manifest,
                )
                .then_with(|| left.cmp(right)),
                _ => left.cmp(right),
            }
        });
        let found = documents.len();
        let mut hits = Vec::with_capacity(limit.min(documents.len()));
        for document in documents.into_iter().skip(offset).take(limit) {
            hits.push(self.hit_value(document, &[], &mut cache)?);
        }
        Ok(json!({
            "found": found,
            "total_moments": found,
            "out_of": self.inner.manifest.documents,
            "hits": hits,
        }))
    }

    fn video_for_document(&self, document: u32) -> Option<&VideoMeta> {
        self.inner
            .metadata
            .document(document)
            .ok()
            .and_then(|record| self.inner.videos.get(record.video as usize))
    }

    fn hit_value(
        &self,
        document: u32,
        highlight_groups: &[Vec<String>],
        cache: &mut BlockCache,
    ) -> Result<Value> {
        let mut source = self.document_value(document, cache)?;
        if let Some(video) = self.video_for_document(document)
            && let Some(object) = source.as_object_mut()
        {
            object.insert("view_count".to_string(), json!(video.view_count));
            object.insert("like_count".to_string(), json!(video.like_count));
            object.insert("comment_count".to_string(), json!(video.comment_count));
        }
        let mut highlights = Vec::with_capacity(2);
        if let Some(text) = source.get("text").and_then(Value::as_str)
            && let Some(snippet) =
                highlighted_snippet(text, &matched_highlight_terms(text, highlight_groups), 360)
        {
            highlights.push(json!({ "field": "text", "snippet": snippet }));
        }
        if let Some(title) = source.get("title").and_then(Value::as_str)
            && let Some(snippet) = highlighted_snippet(
                title,
                &matched_highlight_terms(title, highlight_groups),
                180,
            )
        {
            highlights.push(json!({ "field": "title", "snippet": snippet }));
        }
        Ok(json!({ "document": source, "highlights": highlights }))
    }

    fn document_value(&self, document: u32, cache: &mut BlockCache) -> Result<Value> {
        let record = self.inner.metadata.document(document)?;
        if let std::collections::hash_map::Entry::Vacant(entry) = cache.blocks.entry(record.block) {
            let block = self.inner.metadata.block(record.block)?;
            let start = block.offset as usize;
            let end = start
                .checked_add(block.compressed_length as usize)
                .ok_or_else(|| Error::Corrupt("document block range overflows".to_string()))?;
            let compressed = self
                .inner
                .documents
                .bytes()
                .get(start..end)
                .ok_or_else(|| Error::Corrupt("document block extends beyond file".to_string()))?;
            let decoded = zstd::stream::decode_all(compressed)?;
            if decoded.len() != block.uncompressed_length as usize {
                return Err(Error::Corrupt(
                    "document block length does not match metadata".to_string(),
                ));
            }
            entry.insert(decoded);
        }
        let block = &cache.blocks[&record.block];
        let start = record.offset as usize;
        let end = start
            .checked_add(record.length as usize)
            .ok_or_else(|| Error::Corrupt("document range overflows".to_string()))?;
        let bytes = block
            .get(start..end)
            .ok_or_else(|| Error::Corrupt("document extends beyond decoded block".to_string()))?;
        Ok(serde_json::from_slice(bytes)?)
    }

    pub fn search_channels(&self, params: &SearchParams) -> Result<Value> {
        validate_channel_search_params(params)?;
        let query = params.q.trim().to_ascii_lowercase();
        let mut channels = self
            .inner
            .channels
            .iter()
            .filter(|channel| {
                query == "*"
                    || ["name", "handle"].iter().any(|field| {
                        channel
                            .get(field)
                            .and_then(Value::as_str)
                            .is_some_and(|value| value.to_ascii_lowercase().contains(&query))
                    })
            })
            .cloned()
            .collect::<Vec<_>>();
        channels.sort_by(|left, right| {
            left.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(right.get("name").and_then(Value::as_str).unwrap_or(""))
        });
        let found = channels.len();
        let (offset, limit) = result_window(params);
        let hits = channels
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|document| json!({ "document": document }))
            .collect::<Vec<_>>();
        Ok(json!({ "found": found, "out_of": found, "hits": hits }))
    }
}

const fn exact_proximity_is_bounded(matches: usize) -> bool {
    matches <= MAX_EXACT_PROXIMITY_MATCHES
}

impl VideoMeta {
    fn belongs_to(&self, channel_id: &str) -> bool {
        channel_ids_equal(&self.youtuber_id, channel_id)
            || (self.youtuber_id.is_empty()
                && self
                    .handle
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(channel_id))
    }
}
