use crate::{
    Error, Result,
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
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
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
const DOCUMENTS_FILE: &str = "documents.blocks";
const METADATA_FILE: &str = "documents.meta";
const DOCUMENTS_PER_BLOCK: usize = 64;
const MOMENT_COALESCE_WINDOW_MS: u32 = 20_000;
const METADATA_PREFIX_BYTES: usize = HEADER_BYTES + 8;
const TYPO_RESULT_THRESHOLD: usize = 5;
const DROP_RESULT_THRESHOLD: usize = 1;
const MAX_TOKEN_DROP_CANDIDATES: usize = 20;
const MAX_FUZZY_TERMS_PER_DISTANCE: usize = 4;
const MAX_FUZZY_SPLIT_TERMS: usize = 8;
const MIN_ONE_TYPO_BYTES: usize = 4;
const MIN_TWO_TYPO_BYTES: usize = 7;
const MIN_SPLIT_TERM_CHARS: usize = 2;
const MIN_FUZZY_SPLIT_COMPONENT_CHARS: usize = 3;
const MAX_SPLIT_ALTERNATIVES: usize = 8;
const SPLIT_SCORE_BONUS: f32 = 8.0;
const ENGAGEMENT_RELEVANCE_BOOST: f32 = 0.20;
const VIEW_WEIGHT: f32 = 0.50;
const LIKE_WEIGHT: f32 = 0.35;
const COMMENT_WEIGHT: f32 = 0.15;
const MAX_QUERY_BYTES: usize = 512;
const MAX_FILTER_VALUE_BYTES: usize = 256;
const MAX_PAGE_SIZE: u8 = 250;
const ACTIVE_INCLUDE_FIELDS: &str = "id,video_id,title,text,prev_text,next_text,youtuber_handle,youtuber_name,thumbnail,published_at,duration,t_start_ms,t_end_ms";
const POSITION_SCORE_BOOST: f32 = 0.15;
const PROXIMITY_SCORE_BOOST: f32 = 0.20;
const PROXIMITY_POSITION_BOOST: f32 = 0.05;
const MAX_PROXIMITY_TOKENS: usize = 10;
const MAX_PHRASE_TOKENS: usize = 32;
const MAX_PHRASE_SLOP: u8 = 8;
const GENERATION_FILES: [&str; 9] = [
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

#[derive(Default)]
struct TermBuild {
    text: Vec<BuildPosting>,
    title: Vec<BuildPosting>,
}

#[derive(Clone, Copy)]
struct BuildPosting {
    id: u32,
    position_offset: u32,
    position_length: u16,
    frequency: u16,
}

struct TokenBuild {
    frequency: u16,
    first_position: u32,
    tail: Option<u32>,
}

struct PositionNode {
    position: u32,
    previous: Option<u32>,
}

struct TokenCounts {
    terms: HashMap<String, TokenBuild>,
    tails: Vec<PositionNode>,
}

struct PositionSpoolWriter {
    output: BufWriter<File>,
    position: u32,
    scratch: Vec<u8>,
}

impl PositionSpoolWriter {
    fn create(path: &Path) -> Result<Self> {
        let mut output = BufWriter::new(File::create(path)?);
        let mut header = Vec::with_capacity(HEADER_BYTES);
        write_header(&mut header, POSITION_SPOOL_MAGIC, 0);
        output.write_all(&header)?;
        Ok(Self {
            output,
            position: 0,
            scratch: Vec::new(),
        })
    }

    fn append(&mut self, positions: &[u32]) -> Result<(u32, u16, u16)> {
        let frequency = u16::try_from(positions.len()).map_err(|_| {
            Error::InvalidInput("one term occurs more than 65,535 times in a field".to_string())
        })?;
        if frequency == 0 {
            return Err(Error::InvalidInput(
                "a positional posting must not be empty".to_string(),
            ));
        }
        self.scratch.clear();
        write_varint(positions[0], &mut self.scratch);
        write_remaining_positions(positions, &mut self.scratch)?;
        let length = u16::try_from(self.scratch.len()).map_err(|_| {
            Error::InvalidInput("one encoded position list exceeds 65,535 bytes".to_string())
        })?;
        let offset = self.position;
        self.output.write_all(&self.scratch)?;
        self.position = self
            .position
            .checked_add(u32::from(length))
            .ok_or_else(|| Error::InvalidInput("position spool exceeds 4 GiB".to_string()))?;
        Ok((offset, length, frequency))
    }

    fn finish(mut self) -> Result<()> {
        self.output.flush()?;
        Ok(())
    }
}

impl TokenBuild {
    fn write_positions(&self, tails: &[PositionNode], output: &mut Vec<u32>) -> Result<()> {
        output.clear();
        output.push(self.first_position);
        let mut tail = self.tail;
        while let Some(index) = tail {
            let node = tails.get(index as usize).ok_or_else(|| {
                Error::Corrupt("token position chain is out of bounds".to_string())
            })?;
            output.push(node.position);
            tail = node.previous;
        }
        output[1..].reverse();
        if output.len() != usize::from(self.frequency) {
            return Err(Error::Corrupt(
                "token frequency does not match its positions".to_string(),
            ));
        }
        Ok(())
    }
}

fn append_token_postings(
    terms: &mut HashMap<String, TermBuild>,
    counts: TokenCounts,
    id: u32,
    title: bool,
    spool: &mut PositionSpoolWriter,
    position_scratch: &mut Vec<u32>,
) -> Result<()> {
    let TokenCounts {
        terms: counts,
        tails,
    } = counts;
    for (term, token) in counts {
        token.write_positions(&tails, position_scratch)?;
        let (position_offset, position_length, frequency) = spool.append(position_scratch)?;
        let posting = BuildPosting {
            id,
            position_offset,
            position_length,
            frequency,
        };
        let lists = terms.entry(term).or_default();
        if title {
            lists.title.push(posting);
        } else {
            lists.text.push(posting);
        }
    }
    Ok(())
}

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
    document_with_flags: u32,
    score: f32,
}

impl DocMatch {
    const TEXT_MATCH_FLAG: u32 = 1 << 31;
    const EXACT_MASK_SHIFT: u32 = 27;
    const EXACT_MASK_BITS: u32 = 0b1111 << Self::EXACT_MASK_SHIFT;
    const DOCUMENT_MASK: u32 = (1 << Self::EXACT_MASK_SHIFT) - 1;

    fn new(document: u32, score: f32, matches_text: bool) -> Self {
        Self::with_exact_mask(document, score, matches_text, 1)
    }

    fn with_exact_mask(document: u32, score: f32, matches_text: bool, exact_mask: u8) -> Self {
        debug_assert_eq!(document & !Self::DOCUMENT_MASK, 0);
        Self {
            document_with_flags: document
                | (u32::from(exact_mask & 0b1111) << Self::EXACT_MASK_SHIFT)
                | (u32::from(matches_text) * Self::TEXT_MATCH_FLAG),
            score,
        }
    }

    fn document(self) -> u32 {
        self.document_with_flags & Self::DOCUMENT_MASK
    }

    fn matches_text(self) -> bool {
        self.document_with_flags & Self::TEXT_MATCH_FLAG != 0
    }

    fn set_matches_text(&mut self, matches_text: bool) {
        self.document_with_flags = self.document()
            | (self.document_with_flags & Self::EXACT_MASK_BITS)
            | (u32::from(matches_text) * Self::TEXT_MATCH_FLAG);
    }

    fn exact_mask(self) -> u8 {
        ((self.document_with_flags & Self::EXACT_MASK_BITS) >> Self::EXACT_MASK_SHIFT) as u8
    }

    fn exact_tokens(self) -> u8 {
        self.exact_mask().count_ones() as u8
    }

    fn set_exact_mask(&mut self, exact_mask: u8) {
        self.document_with_flags = self.document()
            | (self.document_with_flags & Self::TEXT_MATCH_FLAG)
            | (u32::from(exact_mask & 0b1111) << Self::EXACT_MASK_SHIFT);
    }
}

const _: () = assert!(DocMatch::DOCUMENT_MASK >= 100_000_000);

struct VideoMatches {
    video: u32,
    score: f32,
    exact_tokens: u8,
    exact_mask: u8,
    moments: MomentCounter,
    documents: Vec<DocMatch>,
}

#[derive(Clone, Copy)]
struct RelevanceRank {
    score: f32,
    exact_tokens: u8,
    exact_mask: u8,
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

struct SplitComponent {
    matches: Vec<DocMatch>,
    highlight: String,
    fuzzy: bool,
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

pub fn build_index(
    chunks_path: &Path,
    channels_path: &Path,
    index_path: &Path,
) -> Result<BuildReport> {
    let started = Instant::now();
    if index_path.join(MANIFEST_FILE).exists() {
        return Err(Error::ExistingIndex(index_path.display().to_string()));
    }
    fs::create_dir_all(index_path)?;

    let documents_path = index_path.join(DOCUMENTS_FILE);
    let mut document_output = BufWriter::new(File::create(&documents_path)?);
    let mut document_header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut document_header, DOCUMENTS_MAGIC, 0);
    document_output.write_all(&document_header)?;
    let position_spool_path = index_path.join(POSITION_SPOOL_FILE);
    let mut position_spool = PositionSpoolWriter::create(&position_spool_path)?;
    let mut position_scratch = Vec::new();

    let mut terms: HashMap<String, TermBuild> = HashMap::new();
    let mut videos = Vec::<VideoMeta>::new();
    let mut video_ids = HashMap::<String, u32>::new();
    let mut document_records = Vec::<DocumentRecord>::new();
    let mut block_records = Vec::<BlockRecord>::new();
    let mut block_bytes = Vec::<u8>::with_capacity(256 * 1024);
    let mut block_document_count = 0_usize;
    let mut total_tokens = 0_u64;

    for line in BufReader::new(File::open(chunks_path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let source: IndexDocument = serde_json::from_str(&line)?;
        let published_at_timestamp = source.published_at_timestamp.or_else(|| {
            source
                .published_at
                .as_deref()
                .and_then(parse_rfc3339_timestamp)
        });
        let video = if let Some(video) = video_ids.get(&source.video_id) {
            let video = *video;
            let metadata = &mut videos[video as usize];
            metadata.view_count = metadata.view_count.max(source.view_count);
            metadata.like_count = metadata.like_count.max(source.like_count);
            metadata.comment_count = metadata.comment_count.max(source.comment_count);
            video
        } else {
            let video = videos.len() as u32;
            let title_counts = token_counts(&source.title)?;
            append_token_postings(
                &mut terms,
                title_counts,
                video,
                true,
                &mut position_spool,
                &mut position_scratch,
            )?;
            videos.push(VideoMeta {
                id: source.video_id.clone(),
                handle: source.youtuber_handle.to_ascii_lowercase(),
                title: source.title,
                published_at_timestamp: published_at_timestamp.unwrap_or_default(),
                view_count: source.view_count,
                like_count: source.like_count,
                comment_count: source.comment_count,
                documents: Vec::new(),
            });
            video_ids.insert(source.video_id, video);
            video
        };

        let document = document_records.len() as u32;
        let text_counts = token_counts(&source.text)?;
        let document_tokens = text_counts
            .terms
            .values()
            .map(|token| u32::from(token.frequency))
            .sum::<u32>();
        total_tokens = total_tokens.saturating_add(u64::from(document_tokens));
        append_token_postings(
            &mut terms,
            text_counts,
            document,
            false,
            &mut position_spool,
            &mut position_scratch,
        )?;

        let line_bytes = line.as_bytes();
        let offset = block_bytes.len() as u32;
        block_bytes.extend_from_slice(line_bytes);
        if let Some(previous_document) = videos[video as usize].documents.last().copied() {
            let previous_start = document_records[previous_document as usize].t_start_ms;
            if source.t_start_ms < previous_start {
                return Err(Error::InvalidInput(format!(
                    "video {} has out-of-order moments: {} follows {}",
                    videos[video as usize].id, source.t_start_ms, previous_start
                )));
            }
        }
        document_records.push(DocumentRecord {
            block: block_records.len() as u32,
            offset,
            length: line_bytes.len() as u32,
            video,
            tokens: document_tokens,
            t_start_ms: source.t_start_ms,
        });
        videos[video as usize].documents.push(document);
        block_document_count += 1;

        if block_document_count == DOCUMENTS_PER_BLOCK {
            flush_document_block(&mut document_output, &mut block_bytes, &mut block_records)?;
            block_document_count = 0;
        }
    }
    if !block_bytes.is_empty() {
        flush_document_block(&mut document_output, &mut block_bytes, &mut block_records)?;
    }
    document_output.flush()?;
    position_spool.finish()?;

    let channels = read_json_lines(channels_path)?;
    serde_json::to_writer(
        BufWriter::new(File::create(index_path.join(CHANNELS_FILE))?),
        &channels,
    )?;
    serde_json::to_writer(
        BufWriter::new(File::create(index_path.join(VIDEOS_FILE))?),
        &videos,
    )?;
    write_metadata(index_path, &document_records, &block_records)?;
    let position_spool = MappedFile::open(&position_spool_path, POSITION_SPOOL_MAGIC)?;
    let term_count = write_postings(index_path, terms, &videos, &position_spool)?;
    drop(position_spool);
    fs::remove_file(&position_spool_path)?;

    let document_count = document_records.len() as u32;
    let average_document_tokens = if document_count == 0 {
        0.0
    } else {
        total_tokens as f32 / document_count as f32
    };
    let manifest = Manifest {
        schema_version: 6,
        documents: document_count,
        videos: videos.len() as u32,
        channels: channels.len() as u32,
        terms: term_count as u32,
        blocks: block_records.len() as u32,
        average_document_tokens,
        max_view_count: videos
            .iter()
            .map(|video| video.view_count)
            .max()
            .unwrap_or(0),
        max_like_count: videos
            .iter()
            .map(|video| video.like_count)
            .max()
            .unwrap_or(0),
        max_comment_count: videos
            .iter()
            .map(|video| video.comment_count)
            .max()
            .unwrap_or(0),
        files: build_manifest_files(index_path)?,
    };
    serde_json::to_writer_pretty(
        BufWriter::new(File::create(index_path.join(MANIFEST_FILE))?),
        &manifest,
    )?;

    Ok(BuildReport {
        documents: u64::from(document_count),
        videos: videos.len(),
        channels: channels.len(),
        terms: term_count,
        index_bytes: directory_size(index_path)?,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

pub fn slice_corpus(
    input_path: &Path,
    output_path: &Path,
    max_videos: usize,
) -> Result<SliceReport> {
    let mut videos = HashSet::new();
    for line in BufReader::new(File::open(input_path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let source: IndexDocument = serde_json::from_str(&line)?;
        videos.insert(source.video_id);
    }
    let mut videos = videos.into_iter().collect::<Vec<_>>();
    videos.sort_unstable_by(|left, right| {
        stable_hash(left)
            .cmp(&stable_hash(right))
            .then_with(|| left.cmp(right))
    });
    videos.truncate(max_videos);
    let selected = videos.into_iter().collect::<HashSet<_>>();

    let mut output = BufWriter::new(File::create(output_path)?);
    let mut documents = 0_u64;
    let mut bytes = 0_u64;
    for line in BufReader::new(File::open(input_path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let source: IndexDocument = serde_json::from_str(&line)?;
        if !selected.contains(&source.video_id) {
            continue;
        }
        output.write_all(line.as_bytes())?;
        output.write_all(b"\n")?;
        documents += 1;
        bytes = bytes.saturating_add(line.len() as u64 + 1);
    }
    output.flush()?;
    Ok(SliceReport {
        documents,
        videos: selected.len(),
        bytes,
    })
}

fn flush_document_block(
    output: &mut BufWriter<File>,
    uncompressed: &mut Vec<u8>,
    blocks: &mut Vec<BlockRecord>,
) -> Result<()> {
    let compressed = zstd::stream::encode_all(uncompressed.as_slice(), 3)?;
    let offset = output.stream_position()?;
    output.write_all(&compressed)?;
    blocks.push(BlockRecord {
        offset,
        compressed_length: compressed.len() as u32,
        uncompressed_length: uncompressed.len() as u32,
    });
    uncompressed.clear();
    Ok(())
}

fn write_metadata(
    index_path: &Path,
    documents: &[DocumentRecord],
    blocks: &[BlockRecord],
) -> Result<()> {
    let mut output = BufWriter::new(File::create(index_path.join(METADATA_FILE))?);
    let mut header = Vec::with_capacity(METADATA_PREFIX_BYTES);
    write_header(&mut header, METADATA_MAGIC, documents.len() as u32);
    header.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    output.write_all(&header)?;
    for document in documents {
        output.write_all(&document.block.to_le_bytes())?;
        output.write_all(&document.offset.to_le_bytes())?;
        output.write_all(&document.length.to_le_bytes())?;
        output.write_all(&document.video.to_le_bytes())?;
        output.write_all(&document.tokens.to_le_bytes())?;
        output.write_all(&document.t_start_ms.to_le_bytes())?;
    }
    for block in blocks {
        output.write_all(&block.offset.to_le_bytes())?;
        output.write_all(&block.compressed_length.to_le_bytes())?;
        output.write_all(&block.uncompressed_length.to_le_bytes())?;
    }
    output.flush()?;
    Ok(())
}

fn write_postings(
    index_path: &Path,
    terms: HashMap<String, TermBuild>,
    videos: &[VideoMeta],
    position_spool: &MappedFile,
) -> Result<usize> {
    let mut terms = terms.into_iter().collect::<Vec<_>>();
    terms.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut postings = BufWriter::new(File::create(index_path.join(POSTINGS_FILE))?);
    let mut postings_header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut postings_header, POSTINGS_MAGIC, terms.len() as u32);
    postings.write_all(&postings_header)?;
    let mut positions = BufWriter::new(File::create(index_path.join(POSITIONS_FILE))?);
    let mut positions_header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut positions_header, POSITIONS_MAGIC, terms.len() as u32);
    positions.write_all(&positions_header)?;
    let mut full_positions = BufWriter::new(File::create(index_path.join(FULL_POSITIONS_FILE))?);
    let mut full_positions_header = Vec::with_capacity(HEADER_BYTES);
    write_header(
        &mut full_positions_header,
        FULL_POSITIONS_MAGIC,
        terms.len() as u32,
    );
    full_positions.write_all(&full_positions_header)?;
    let position_offsets_count = terms
        .len()
        .checked_mul(2)
        .and_then(|count| count.checked_add(1))
        .ok_or_else(|| Error::Corrupt("position offset count overflows".to_string()))?;
    let position_table_bytes = position_offsets_count
        .checked_mul(4)
        .ok_or_else(|| Error::Corrupt("position offset table overflows".to_string()))?;
    positions.write_all(&vec![0_u8; position_table_bytes])?;
    full_positions.write_all(&vec![0_u8; position_table_bytes])?;
    let mut postings_position = HEADER_BYTES as u64;
    let mut positions_position = 0_u32;
    let mut full_positions_position = 0_u32;
    let mut position_offsets = Vec::with_capacity(position_offsets_count);
    let mut full_position_offsets = Vec::with_capacity(position_offsets_count);
    let mut lexicon = Vec::with_capacity(terms.len());
    let mut position_scratch = Vec::with_capacity(5);

    for (term, lists) in terms {
        let text = encode_postings(&lists.text);
        let text_offset = postings_position;
        postings.write_all(&text)?;
        postings_position += text.len() as u64;
        position_offsets.push(positions_position);
        full_position_offsets.push(full_positions_position);
        let (text_first_bytes, text_remaining_bytes) = write_position_lists(
            &lists.text,
            position_spool,
            &mut positions,
            &mut full_positions,
            &mut position_scratch,
        )?;
        positions_position = positions_position
            .checked_add(text_first_bytes)
            .ok_or_else(|| Error::Corrupt("positions file exceeds u32 length".to_string()))?;
        full_positions_position = full_positions_position
            .checked_add(text_remaining_bytes)
            .ok_or_else(|| Error::Corrupt("full positions file exceeds u32 length".to_string()))?;
        let title = encode_postings(&lists.title);
        let fuzzy_title_frequency = lists.title.iter().fold(0_u32, |frequency, posting| {
            frequency.saturating_add(
                videos
                    .get(posting.id as usize)
                    .map_or(0, |video| video.documents.len() as u32),
            )
        });
        let title_offset = postings_position;
        postings.write_all(&title)?;
        postings_position += title.len() as u64;
        position_offsets.push(positions_position);
        full_position_offsets.push(full_positions_position);
        let (title_first_bytes, title_remaining_bytes) = write_position_lists(
            &lists.title,
            position_spool,
            &mut positions,
            &mut full_positions,
            &mut position_scratch,
        )?;
        positions_position = positions_position
            .checked_add(title_first_bytes)
            .ok_or_else(|| Error::Corrupt("positions file exceeds u32 length".to_string()))?;
        full_positions_position = full_positions_position
            .checked_add(title_remaining_bytes)
            .ok_or_else(|| Error::Corrupt("full positions file exceeds u32 length".to_string()))?;
        lexicon.push(LexiconBuildEntry {
            term,
            text_offset,
            text_length: text.len() as u32,
            text_frequency: lists.text.len() as u32,
            title_offset,
            title_length: title.len() as u32,
            title_frequency: lists.title.len() as u32,
            fuzzy_title_frequency,
        });
    }
    position_offsets.push(positions_position);
    full_position_offsets.push(full_positions_position);
    postings.flush()?;
    positions.seek(SeekFrom::Start(HEADER_BYTES as u64))?;
    for offset in position_offsets {
        positions.write_all(&offset.to_le_bytes())?;
    }
    positions.flush()?;
    full_positions.seek(SeekFrom::Start(HEADER_BYTES as u64))?;
    for offset in full_position_offsets {
        full_positions.write_all(&offset.to_le_bytes())?;
    }
    full_positions.flush()?;
    write_fuzzy_index(
        index_path,
        lexicon.iter().enumerate().map(|(index, entry)| {
            (
                index as u32,
                entry.term.as_str(),
                entry.text_frequency,
                entry.fuzzy_title_frequency,
            )
        }),
    )?;
    write_lexicon(index_path, &lexicon)?;
    Ok(lexicon.len())
}

fn write_position_lists(
    postings: &[BuildPosting],
    spool: &MappedFile,
    first_positions: &mut BufWriter<File>,
    remaining_positions: &mut BufWriter<File>,
    scratch: &mut Vec<u8>,
) -> Result<(u32, u32)> {
    let mut first_bytes = 0_u32;
    let mut remaining_bytes = 0_u32;
    for posting in postings {
        let start = HEADER_BYTES
            .checked_add(posting.position_offset as usize)
            .ok_or_else(|| Error::Corrupt("position spool offset overflows".to_string()))?;
        let end = start
            .checked_add(usize::from(posting.position_length))
            .ok_or_else(|| Error::Corrupt("position spool range overflows".to_string()))?;
        let encoded = spool
            .bytes()
            .get(start..end)
            .ok_or_else(|| Error::Corrupt("position spool range is out of bounds".to_string()))?;
        let mut cursor = 0_usize;
        let first = read_varint(encoded, &mut cursor, encoded.len())?;

        scratch.clear();
        write_varint(first, scratch);
        first_positions.write_all(scratch)?;
        first_bytes = first_bytes
            .checked_add(u32::try_from(scratch.len()).map_err(|_| {
                Error::Corrupt("encoded first position exceeds u32 length".to_string())
            })?)
            .ok_or_else(|| Error::Corrupt("first position stream exceeds 4 GiB".to_string()))?;

        let remaining = &encoded[cursor..];
        remaining_positions.write_all(remaining)?;
        remaining_bytes = remaining_bytes
            .checked_add(u32::try_from(remaining.len()).map_err(|_| {
                Error::Corrupt("encoded remaining positions exceed u32 length".to_string())
            })?)
            .ok_or_else(|| Error::Corrupt("remaining position stream exceeds 4 GiB".to_string()))?;
    }
    Ok((first_bytes, remaining_bytes))
}

fn encode_postings(postings: &[BuildPosting]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(postings.len().saturating_mul(3));
    let mut previous = 0_u32;
    for (index, posting) in postings.iter().enumerate() {
        let delta = if index == 0 {
            posting.id
        } else {
            posting.id - previous
        };
        write_varint(delta, &mut encoded);
        write_varint(u32::from(posting.frequency), &mut encoded);
        previous = posting.id;
    }
    encoded
}

fn write_lexicon(index_path: &Path, entries: &[LexiconBuildEntry]) -> Result<()> {
    let mut entry_bytes = Vec::new();
    let mut offsets = Vec::with_capacity(entries.len() + 1);
    for entry in entries {
        offsets.push(entry_bytes.len() as u64);
        let term = entry.term.as_bytes();
        let length = u16::try_from(term.len())
            .map_err(|_| Error::Corrupt("term exceeds u16 length".to_string()))?;
        entry_bytes.extend_from_slice(&length.to_le_bytes());
        entry_bytes.extend_from_slice(term);
        entry_bytes.extend_from_slice(&entry.text_offset.to_le_bytes());
        entry_bytes.extend_from_slice(&entry.text_length.to_le_bytes());
        entry_bytes.extend_from_slice(&entry.text_frequency.to_le_bytes());
        entry_bytes.extend_from_slice(&entry.title_offset.to_le_bytes());
        entry_bytes.extend_from_slice(&entry.title_length.to_le_bytes());
        entry_bytes.extend_from_slice(&entry.title_frequency.to_le_bytes());
    }
    offsets.push(entry_bytes.len() as u64);

    let mut output = BufWriter::new(File::create(index_path.join(LEXICON_FILE))?);
    let mut header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut header, LEXICON_MAGIC, entries.len() as u32);
    output.write_all(&header)?;
    for offset in offsets {
        output.write_all(&offset.to_le_bytes())?;
    }
    output.write_all(&entry_bytes)?;
    output.flush()?;
    Ok(())
}

impl RawSearchEngine {
    pub fn open(index_path: &Path) -> Result<Self> {
        let manifest: Manifest =
            serde_json::from_reader(BufReader::new(File::open(index_path.join(MANIFEST_FILE))?))?;
        if !matches!(manifest.schema_version, 5 | 6) {
            return Err(Error::Corrupt(format!(
                "unsupported schema version {}",
                manifest.schema_version
            )));
        }
        if manifest.schema_version == 6 {
            validate_manifest_files(index_path, &manifest)?;
        }
        let videos: Vec<VideoMeta> =
            serde_json::from_reader(BufReader::new(File::open(index_path.join(VIDEOS_FILE))?))?;
        let channels: Vec<Value> =
            serde_json::from_reader(BufReader::new(File::open(index_path.join(CHANNELS_FILE))?))?;
        let lexicon = Lexicon::open(&index_path.join(LEXICON_FILE))?;
        let fuzzy = FuzzyIndex::open(index_path, lexicon.count)?;
        let postings = MappedFile::open(&index_path.join(POSTINGS_FILE), POSTINGS_MAGIC)?;
        let positions = PositionIndex::open(
            &index_path.join(POSITIONS_FILE),
            POSITIONS_MAGIC,
            lexicon.count,
        )?;
        let full_positions = (manifest.schema_version >= 6)
            .then(|| {
                PositionIndex::open(
                    &index_path.join(FULL_POSITIONS_FILE),
                    FULL_POSITIONS_MAGIC,
                    lexicon.count,
                )
            })
            .transpose()?;
        let documents = MappedFile::open(&index_path.join(DOCUMENTS_FILE), DOCUMENTS_MAGIC)?;
        let metadata = DocumentTable::open(&index_path.join(METADATA_FILE))?;
        let counts_match = manifest.documents as usize == metadata.documents
            && manifest.blocks as usize == metadata.blocks
            && manifest.videos as usize == videos.len()
            && manifest.channels as usize == channels.len()
            && manifest.terms as usize == lexicon.count;
        if !counts_match {
            return Err(Error::Corrupt(
                "manifest counts do not match index files".to_string(),
            ));
        }
        Ok(Self {
            inner: Arc::new(EngineInner {
                manifest,
                videos,
                channels,
                lexicon,
                fuzzy,
                postings,
                positions,
                full_positions,
                documents,
                metadata,
                index_bytes: directory_size(index_path)?,
            }),
        })
    }
}

impl Lexicon {
    fn open(path: &Path) -> Result<Self> {
        let file = MappedFile::open(path, LEXICON_MAGIC)?;
        let count = read_u32(file.bytes(), 8)? as usize;
        let version = read_u32(file.bytes(), 12)?;
        if version != 1 {
            return Err(Error::Corrupt(format!(
                "unsupported lexicon version {version}"
            )));
        }
        let offsets_bytes = (count + 1)
            .checked_mul(8)
            .ok_or_else(|| Error::Corrupt("lexicon offset table overflows".to_string()))?;
        let entries_start = HEADER_BYTES
            .checked_add(offsets_bytes)
            .ok_or_else(|| Error::Corrupt("lexicon header overflows".to_string()))?;
        if entries_start > file.bytes().len() {
            return Err(Error::Corrupt(
                "lexicon offset table is truncated".to_string(),
            ));
        }
        let last = read_u64(file.bytes(), HEADER_BYTES + count * 8)? as usize;
        if entries_start.saturating_add(last) != file.bytes().len() {
            return Err(Error::Corrupt(
                "lexicon entry boundary is invalid".to_string(),
            ));
        }
        Ok(Self {
            file,
            count,
            entries_start,
        })
    }

    fn lookup(&self, term: &str) -> Result<Option<TermEntry>> {
        let mut low = 0_usize;
        let mut high = self.count;
        while low < high {
            let middle = low + (high - low) / 2;
            let (candidate, entry) = self.entry_at(middle)?;
            match candidate.as_bytes().cmp(term.as_bytes()) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return Ok(Some(entry)),
            }
        }
        Ok(None)
    }

    fn entry_at(&self, index: usize) -> Result<(&str, TermEntry)> {
        if index >= self.count {
            return Err(Error::Corrupt("lexicon entry is out of bounds".to_string()));
        }
        let relative = read_u64(self.file.bytes(), HEADER_BYTES + index * 8)? as usize;
        let next = read_u64(self.file.bytes(), HEADER_BYTES + (index + 1) * 8)? as usize;
        let start = self.entries_start.saturating_add(relative);
        let end = self.entries_start.saturating_add(next);
        let entry = self
            .file
            .bytes()
            .get(start..end)
            .ok_or_else(|| Error::Corrupt("lexicon entry extends beyond file".to_string()))?;
        let term_length = read_u16(entry, 0)? as usize;
        let fields = 2 + term_length;
        if fields + 32 != entry.len() {
            return Err(Error::Corrupt(
                "lexicon entry has an invalid length".to_string(),
            ));
        }
        let term = std::str::from_utf8(&entry[2..fields])
            .map_err(|_| Error::Corrupt("lexicon term is not UTF-8".to_string()))?;
        Ok((
            term,
            TermEntry {
                term: index as u32,
                text_offset: read_u64(entry, fields)? as usize,
                text_length: read_u32(entry, fields + 8)? as usize,
                text_frequency: read_u32(entry, fields + 12)?,
                title_offset: read_u64(entry, fields + 16)? as usize,
                title_length: read_u32(entry, fields + 24)? as usize,
                title_frequency: read_u32(entry, fields + 28)?,
            },
        ))
    }
}

impl PositionIndex {
    fn open(path: &Path, magic: &[u8; 8], expected_terms: usize) -> Result<Self> {
        let file = MappedFile::open(path, magic)?;
        let terms = read_u32(file.bytes(), 8)? as usize;
        if terms != expected_terms {
            return Err(Error::Corrupt(
                "position term count does not match lexicon".to_string(),
            ));
        }
        let offsets = terms
            .checked_mul(2)
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| Error::Corrupt("position offset count overflows".to_string()))?;
        let table_bytes = offsets
            .checked_mul(4)
            .ok_or_else(|| Error::Corrupt("position offset table overflows".to_string()))?;
        let data_start = HEADER_BYTES
            .checked_add(table_bytes)
            .ok_or_else(|| Error::Corrupt("position data offset overflows".to_string()))?;
        if data_start > file.bytes().len() {
            return Err(Error::Corrupt(
                "position offset table is truncated".to_string(),
            ));
        }
        let mut previous = 0_u32;
        for index in 0..offsets {
            let offset = read_u32(file.bytes(), HEADER_BYTES + index * 4)?;
            if offset < previous {
                return Err(Error::Corrupt(
                    "position offsets are not monotonic".to_string(),
                ));
            }
            previous = offset;
        }
        if data_start.saturating_add(previous as usize) != file.bytes().len() {
            return Err(Error::Corrupt(
                "position data boundary is invalid".to_string(),
            ));
        }
        Ok(Self {
            file,
            terms,
            data_start,
        })
    }

    fn range(&self, term: u32, title: bool) -> Result<&[u8]> {
        if term as usize >= self.terms {
            return Err(Error::Corrupt(
                "position term is outside the lexicon".to_string(),
            ));
        }
        let index = term as usize * 2 + usize::from(title);
        let start = read_u32(self.file.bytes(), HEADER_BYTES + index * 4)? as usize;
        let end = read_u32(self.file.bytes(), HEADER_BYTES + (index + 1) * 4)? as usize;
        self.file
            .bytes()
            .get(self.data_start + start..self.data_start + end)
            .ok_or_else(|| Error::Corrupt("position range extends beyond file".to_string()))
    }
}

impl<'a> PositionedPostingCursor<'a> {
    fn new(
        postings: &'a [u8],
        first_positions: &'a [u8],
        remaining_positions: &'a [u8],
        expected: u32,
    ) -> Self {
        Self {
            postings,
            posting_cursor: 0,
            first_positions,
            first_cursor: 0,
            remaining_positions: PositionCursor::new(remaining_positions),
            previous_id: 0,
            decoded: 0,
            expected,
        }
    }

    fn next_at_least(&mut self, target: u32, positions: &mut Vec<u32>) -> Result<Option<u32>> {
        loop {
            if self.posting_cursor == self.postings.len() {
                if self.decoded != self.expected || self.first_cursor != self.first_positions.len()
                {
                    return Err(Error::Corrupt(
                        "posting and complete-position streams are misaligned".to_string(),
                    ));
                }
                self.remaining_positions.finish()?;
                positions.clear();
                return Ok(None);
            }

            let delta = read_varint(self.postings, &mut self.posting_cursor, self.postings.len())?;
            if self.decoded > 0 && delta == 0 {
                return Err(Error::Corrupt(
                    "posting identifiers must be strictly increasing".to_string(),
                ));
            }
            let id = self
                .previous_id
                .checked_add(delta)
                .ok_or_else(|| Error::Corrupt("posting identifier overflows u32".to_string()))?;
            let frequency =
                read_varint(self.postings, &mut self.posting_cursor, self.postings.len())?;
            let first_position = read_varint(
                self.first_positions,
                &mut self.first_cursor,
                self.first_positions.len(),
            )?;
            self.previous_id = id;
            self.decoded = self
                .decoded
                .checked_add(1)
                .ok_or_else(|| Error::Corrupt("posting count overflows u32".to_string()))?;
            if self.decoded > self.expected {
                return Err(Error::Corrupt(
                    "posting count exceeds the lexicon frequency".to_string(),
                ));
            }
            if id < target {
                self.remaining_positions.skip(frequency)?;
                continue;
            }
            self.remaining_positions
                .read_into(first_position, frequency, positions)?;
            return Ok(Some(id));
        }
    }
}

impl PhraseCursor<'_> {
    fn advance_to(&mut self, target: u32) -> Result<()> {
        self.id = self.postings.next_at_least(target, &mut self.positions)?;
        Ok(())
    }
}

impl DocumentTable {
    fn open(path: &Path) -> Result<Self> {
        let file = MappedFile::open(path, METADATA_MAGIC)?;
        let documents = read_u32(file.bytes(), 8)? as usize;
        let blocks = read_u32(file.bytes(), HEADER_BYTES)? as usize;
        let document_bytes = documents
            .checked_mul(DOCUMENT_RECORD_BYTES)
            .ok_or_else(|| Error::Corrupt("document metadata overflows".to_string()))?;
        let blocks_start = METADATA_PREFIX_BYTES
            .checked_add(document_bytes)
            .ok_or_else(|| Error::Corrupt("block metadata offset overflows".to_string()))?;
        let expected = blocks_start
            .checked_add(blocks.saturating_mul(BLOCK_RECORD_BYTES))
            .ok_or_else(|| Error::Corrupt("block metadata length overflows".to_string()))?;
        if expected != file.bytes().len() {
            return Err(Error::Corrupt(
                "metadata file has an invalid length".to_string(),
            ));
        }
        Ok(Self {
            file,
            documents,
            blocks,
            blocks_start,
        })
    }

    fn document(&self, id: u32) -> Result<DocumentRecord> {
        if id as usize >= self.documents {
            return Err(Error::Corrupt(format!("document {id} is out of bounds")));
        }
        let offset = METADATA_PREFIX_BYTES + id as usize * DOCUMENT_RECORD_BYTES;
        Ok(DocumentRecord {
            block: read_u32(self.file.bytes(), offset)?,
            offset: read_u32(self.file.bytes(), offset + 4)?,
            length: read_u32(self.file.bytes(), offset + 8)?,
            video: read_u32(self.file.bytes(), offset + 12)?,
            tokens: read_u32(self.file.bytes(), offset + 16)?,
            t_start_ms: read_u32(self.file.bytes(), offset + 20)?,
        })
    }

    fn block(&self, id: u32) -> Result<BlockRecord> {
        if id as usize >= self.blocks {
            return Err(Error::Corrupt(format!("block {id} is out of bounds")));
        }
        let offset = self.blocks_start + id as usize * BLOCK_RECORD_BYTES;
        Ok(BlockRecord {
            offset: read_u64(self.file.bytes(), offset)?,
            compressed_length: read_u32(self.file.bytes(), offset + 8)?,
            uncompressed_length: read_u32(self.file.bytes(), offset + 12)?,
        })
    }
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
        let excluded_documents =
            self.excluded_documents(&parsed.excluded_terms, &parsed.excluded_phrases, fields)?;
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

        let exact_term_matches = self.exact_term_matches(&terms, fields)?;
        let mut strict_matches = intersect_all_borrowed(&exact_term_matches);
        retain_not_excluded(&mut strict_matches, &excluded_documents);
        if self.reaches_result_threshold(params, &strict_matches, TYPO_RESULT_THRESHOLD)? {
            self.apply_exact_proximity(&terms, fields, &mut strict_matches)?;
            return self.format_conjunctive(params, terms, strict_matches);
        }

        // Typesense expands the last token as a prefix and tolerates typos before it
        // starts dropping query tokens. Every expansion remains conjunctive: a
        // document must match one alternative for every retained query token.
        if let Some((matches, highlights)) = self.best_conjunctive_candidate(
            params,
            &terms,
            fields,
            true,
            Some(exact_term_matches),
            &excluded_documents,
        )? {
            return self.format_conjunctive(params, highlights, matches);
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
                None,
                &excluded_documents,
            )? {
                return self.format_conjunctive(params, highlights, matches);
            }
        }
        for dropped in 1..terms.len().min(MAX_TOKEN_DROP_CANDIDATES + 1) {
            if let Some((matches, highlights)) = self.best_conjunctive_candidate(
                params,
                &terms[dropped..],
                fields,
                true,
                None,
                &excluded_documents,
            )? {
                return self.format_conjunctive(params, highlights, matches);
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
                self.phrase_ids_for_field(phrase, false, &targets.documents, slop)?
            } else {
                Vec::new()
            },
            videos: if fields.title {
                self.phrase_ids_for_field(phrase, true, &targets.videos, slop)?
            } else {
                Vec::new()
            },
        })
    }

    fn phrase_ids_for_field(
        &self,
        phrase: &[String],
        title: bool,
        targets: &[u32],
        slop: u8,
    ) -> Result<Vec<u32>> {
        self.evaluate_positions_for_field(phrase, title, targets, |target, cursors| {
            ordered_phrase_positions_match(cursors, slop).then_some(target)
        })
    }

    fn position_windows_for_field(
        &self,
        terms: &[String],
        title: bool,
        targets: &[u32],
    ) -> Result<Vec<(u32, PositionalWindow)>> {
        self.evaluate_positions_for_field(terms, title, targets, |target, cursors| {
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
        videos.sort_unstable();
        videos.dedup();
        Ok(PositionTargets { documents, videos })
    }

    fn apply_exact_proximity(
        &self,
        terms: &[String],
        fields: QueryFields,
        matches: &mut [DocMatch],
    ) -> Result<()> {
        if terms.len() < 2 || matches.is_empty() || self.inner.full_positions.is_none() {
            return Ok(());
        }
        let terms = &terms[..terms.len().min(MAX_PROXIMITY_TOKENS)];
        let targets = self.position_targets(matches)?;
        let text_windows = if fields.text {
            self.position_windows_for_field(terms, false, &targets.documents)?
        } else {
            Vec::new()
        };
        let title_windows = if fields.title {
            self.position_windows_for_field(terms, true, &targets.videos)?
        } else {
            Vec::new()
        };
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
            candidate.score *= text_multiplier.max(title_multiplier);
        }
        Ok(())
    }

    fn evaluate_positions_for_field<T, F>(
        &self,
        terms: &[String],
        title: bool,
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
            let (offset, length, frequency) = if title {
                (
                    entry.title_offset,
                    entry.title_length,
                    entry.title_frequency,
                )
            } else {
                (entry.text_offset, entry.text_length, entry.text_frequency)
            };
            if frequency == 0 {
                return Ok(Vec::new());
            }
            cursors.push(PhraseCursor {
                postings: PositionedPostingCursor::new(
                    self.postings_range(offset, length)?,
                    self.inner.positions.range(entry.term, title)?,
                    full_positions.range(entry.term, title)?,
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

    fn search_single_token(
        &self,
        params: &SearchParams,
        term: &str,
        fields: QueryFields,
        excluded_documents: &[u32],
    ) -> Result<Value> {
        let exact = self.matches_for_term(term, fields)?;
        let mut cache = BlockCache::default();
        let (split, split_terms) = self.split_term_matches(term, fields)?;
        let has_exact_split = !split.is_empty();
        let mut matches = union_sorted(exact, split);
        retain_not_excluded(&mut matches, excluded_documents);
        let mut highlight_terms = Vec::with_capacity(1 + split_terms.len());
        highlight_terms.push(term.to_string());
        highlight_terms.extend(split_terms);
        if self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)? {
            return self.format_matches(params, &highlight_terms, matches, &mut cache);
        }

        let (prefix, prefix_terms) = self.fuzzy_alternatives(term, fields, 0, None, true)?;
        matches = union_sorted(matches, prefix);
        retain_not_excluded(&mut matches, excluded_documents);
        highlight_terms.extend(prefix_terms);
        if bounded_typo_distance(term) == 0
            || self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)?
        {
            return self.format_matches(params, &highlight_terms, matches, &mut cache);
        }
        // Treat a missing separator plus one corrected component as a more
        // specific hypothesis than an unrelated whole-token typo. The path is
        // still guarded by lexicon membership, one exact component, one
        // distance-one component, and a same-document intersection.
        if !has_exact_split {
            let (fuzzy_split, fuzzy_split_terms) =
                self.fuzzy_split_term_matches(term, fields, &mut cache)?;
            matches = union_sorted(matches, fuzzy_split);
            retain_not_excluded(&mut matches, excluded_documents);
            highlight_terms.extend(fuzzy_split_terms);
            if self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)? {
                return self.format_matches(params, &highlight_terms, matches, &mut cache);
            }
        }
        let maximum_distance = bounded_typo_distance(term);
        for distance in 1..=maximum_distance {
            let (fuzzy, fuzzy_terms) =
                self.fuzzy_alternatives(term, fields, distance, Some(distance), false)?;
            highlight_terms.extend(fuzzy_terms);
            matches = union_sorted(matches, fuzzy);
            retain_not_excluded(&mut matches, excluded_documents);
            if distance == maximum_distance
                || self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)?
            {
                break;
            }
        }
        self.format_matches(params, &highlight_terms, matches, &mut cache)
    }

    fn reaches_result_threshold(
        &self,
        params: &SearchParams,
        matches: &[DocMatch],
        threshold: usize,
    ) -> Result<bool> {
        let filter = parse_filter(params.filter_by.as_deref())?;
        if params.group_by.as_deref() == Some("video_id") {
            let mut videos = HashSet::with_capacity(threshold);
            for candidate in matches {
                let record = self.inner.metadata.document(candidate.document())?;
                let video = self
                    .inner
                    .videos
                    .get(record.video as usize)
                    .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
                if video_matches_filter(video, filter) {
                    videos.insert(record.video);
                    if videos.len() >= threshold {
                        return Ok(true);
                    }
                }
            }
            return Ok(false);
        }

        let mut found = 0_usize;
        for candidate in matches {
            let record = self.inner.metadata.document(candidate.document())?;
            let video = self
                .inner
                .videos
                .get(record.video as usize)
                .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
            if video_matches_filter(video, filter) {
                found += 1;
                if found >= threshold {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn split_term_matches(
        &self,
        term: &str,
        fields: QueryFields,
    ) -> Result<(Vec<DocMatch>, Vec<String>)> {
        let boundaries = term
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let character_count = boundaries.len();
        let mut alternatives = Vec::new();
        let mut highlight_terms = Vec::new();

        for (character_index, &byte_index) in boundaries.iter().enumerate().skip(1) {
            if character_index < MIN_SPLIT_TERM_CHARS
                || character_count.saturating_sub(character_index) < MIN_SPLIT_TERM_CHARS
            {
                continue;
            }
            let left = &term[..byte_index];
            let right = &term[byte_index..];
            let left_matches = self.matches_for_split_component(left, fields)?;
            let right_matches = self.matches_for_split_component(right, fields)?;
            if left_matches.is_empty() || right_matches.is_empty() {
                continue;
            }
            let mut matches = intersect_sorted(left_matches, right_matches);
            if matches.is_empty() {
                continue;
            }
            for candidate in &mut matches {
                candidate.score += SPLIT_SCORE_BONUS;
            }
            alternatives.push(matches);
            highlight_terms.push(left.to_string());
            highlight_terms.push(right.to_string());
            if alternatives.len() == MAX_SPLIT_ALTERNATIVES {
                break;
            }
        }

        Ok((union_all(alternatives), highlight_terms))
    }

    fn matches_for_split_component(
        &self,
        term: &str,
        fields: QueryFields,
    ) -> Result<Vec<DocMatch>> {
        let mut alternatives = Vec::with_capacity(2);
        if let Some(entry) = self.inner.lexicon.lookup(term)? {
            alternatives.push(self.matches_for_entry(entry, fields)?);
        }
        if term.len() >= 3 && !term.ends_with('s') {
            let plural = format!("{term}s");
            if let Some(entry) = self.inner.lexicon.lookup(&plural)? {
                alternatives.push(self.matches_for_entry(entry, fields)?);
            }
        }
        Ok(union_all(alternatives))
    }

    fn fuzzy_split_term_matches(
        &self,
        term: &str,
        fields: QueryFields,
        cache: &mut BlockCache,
    ) -> Result<(Vec<DocMatch>, Vec<String>)> {
        let boundaries = term
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let character_count = boundaries.len();
        let mut loose_alternatives = Vec::new();
        let mut loose_highlight_terms = Vec::new();
        let mut phrase_alternatives = Vec::new();
        let mut phrase_highlight_terms = Vec::new();

        for (character_index, &byte_index) in boundaries.iter().enumerate().skip(1) {
            if character_index < MIN_FUZZY_SPLIT_COMPONENT_CHARS
                || character_count.saturating_sub(character_index) < MIN_FUZZY_SPLIT_COMPONENT_CHARS
            {
                continue;
            }
            let left = self.split_component_candidates(&term[..byte_index], fields)?;
            let right = self.split_component_candidates(&term[byte_index..], fields)?;
            for left_candidate in &left {
                for right_candidate in &right {
                    if left_candidate.fuzzy == right_candidate.fuzzy {
                        continue;
                    }
                    let mut matches = intersect_sorted(
                        left_candidate.matches.clone(),
                        right_candidate.matches.clone(),
                    );
                    if matches.is_empty() {
                        continue;
                    }
                    for candidate in &mut matches {
                        // Preserve the corrected two-term query's relative field/term
                        // weights. A smaller base bonus avoids over-rewarding the
                        // correction; adjacent phrase evidence is handled separately.
                        // This keeps `blak + hole` and `black + hloe` from ranking the
                        // same corrected intent differently.
                        candidate.score += SPLIT_SCORE_BONUS * 0.5;
                        candidate.set_exact_mask(0);
                    }
                    let mut phrase_matches = matches.clone();
                    let phrase =
                        format!("{} {}", left_candidate.highlight, right_candidate.highlight);
                    self.retain_phrase_matches(&mut phrase_matches, &phrase, cache)?;
                    if !phrase_matches.is_empty() {
                        phrase_alternatives.push(phrase_matches);
                        phrase_highlight_terms.push(left_candidate.highlight.clone());
                        phrase_highlight_terms.push(right_candidate.highlight.clone());
                    }
                    loose_alternatives.push(matches);
                    loose_highlight_terms.push(left_candidate.highlight.clone());
                    loose_highlight_terms.push(right_candidate.highlight.clone());
                    if loose_alternatives.len() == MAX_SPLIT_ALTERNATIVES {
                        return if phrase_alternatives.is_empty() {
                            Ok((union_all(loose_alternatives), loose_highlight_terms))
                        } else {
                            Ok((union_all(phrase_alternatives), phrase_highlight_terms))
                        };
                    }
                }
            }
        }
        if phrase_alternatives.is_empty() {
            Ok((union_all(loose_alternatives), loose_highlight_terms))
        } else {
            Ok((union_all(phrase_alternatives), phrase_highlight_terms))
        }
    }

    fn split_component_candidates(
        &self,
        term: &str,
        fields: QueryFields,
    ) -> Result<Vec<SplitComponent>> {
        let exact = self.matches_for_split_component(term, fields)?;
        let mut output = Vec::new();
        if !exact.is_empty() {
            output.push(SplitComponent {
                matches: exact,
                highlight: term.to_string(),
                fuzzy: false,
            });
        }
        if term.len() < MIN_FUZZY_SPLIT_COMPONENT_CHARS {
            return Ok(output);
        }

        let candidates = self.inner.fuzzy.candidates(
            term,
            1,
            MAX_FUZZY_SPLIT_TERMS,
            false,
            fields.text,
            fields.title,
        )?;
        output.reserve(candidates.len());
        for candidate in candidates {
            if candidate.exact || candidate.distance != 1 {
                continue;
            }
            let (candidate_term, _) = self.inner.lexicon.entry_at(candidate.term as usize)?;
            // A compact-token split already hypothesizes a missing separator. Avoid also
            // deleting characters from a component: `blackhle` -> `black he` is much
            // broader than the intended insertion correction `black hole`.
            if candidate_term.len() < term.len() {
                continue;
            }
            let matches = self.matches_for_split_component(
                candidate_term,
                QueryFields {
                    prioritize_position: false,
                    ..fields
                },
            )?;
            output.push(SplitComponent {
                matches,
                highlight: candidate_term.to_string(),
                fuzzy: true,
            });
        }
        Ok(output)
    }

    fn best_conjunctive_candidate(
        &self,
        params: &SearchParams,
        terms: &[String],
        fields: QueryFields,
        prefix_last: bool,
        initial_term_matches: Option<Vec<Vec<DocMatch>>>,
        excluded_documents: &[u32],
    ) -> Result<Option<(Vec<DocMatch>, Vec<String>)>> {
        let maximum_distance = terms
            .iter()
            .map(|term| bounded_typo_distance(term))
            .max()
            .unwrap_or(0);
        let mut term_matches = match initial_term_matches {
            Some(matches) => matches,
            None => self.exact_term_matches(terms, fields)?,
        };
        let mut highlight_terms = terms.to_vec();
        if prefix_last {
            let last = terms
                .last()
                .ok_or_else(|| Error::InvalidInput("query has no searchable tokens".to_string()))?;
            let (prefix, prefix_terms) = self.fuzzy_alternatives(last, fields, 0, None, true)?;
            let last_matches = term_matches
                .last_mut()
                .ok_or_else(|| Error::InvalidInput("query has no searchable tokens".to_string()))?;
            *last_matches = union_sorted(std::mem::take(last_matches), prefix);
            highlight_terms.extend(prefix_terms);
        }

        for typo_distance in 0..=maximum_distance {
            if typo_distance > 0 {
                for (term, matches) in terms.iter().zip(&mut term_matches) {
                    if typo_distance > bounded_typo_distance(term) {
                        continue;
                    }
                    let (fuzzy, fuzzy_terms) = self.fuzzy_alternatives(
                        term,
                        fields,
                        typo_distance,
                        Some(typo_distance),
                        false,
                    )?;
                    *matches = union_sorted(std::mem::take(matches), fuzzy);
                    highlight_terms.extend(fuzzy_terms);
                }
            }
            let mut matches = intersect_all_borrowed(&term_matches);
            retain_not_excluded(&mut matches, excluded_documents);
            let enough = self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)?;
            if enough {
                self.apply_exact_proximity(terms, fields, &mut matches)?;
                return Ok(Some((matches, highlight_terms)));
            }
            if typo_distance == maximum_distance {
                return if self.reaches_result_threshold(params, &matches, DROP_RESULT_THRESHOLD)? {
                    self.apply_exact_proximity(terms, fields, &mut matches)?;
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
        self.format_matches(params, &highlight_terms, matches, &mut cache)
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
            .then(|| self.inner.positions.range(entry.term, false))
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
            .then(|| self.inner.positions.range(entry.term, true))
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
                * 6.0
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

    fn format_matches(
        &self,
        params: &SearchParams,
        terms: &[String],
        matches: Vec<DocMatch>,
        cache: &mut BlockCache,
    ) -> Result<Value> {
        if params.group_by.as_deref() != Some("video_id") {
            return self.format_ungrouped(params, terms, matches, cache);
        }

        let hits_per_group = usize::from(params.group_limit.unwrap_or(5).max(1));
        let filter = parse_filter(params.filter_by.as_deref())?;
        let mut groups = HashMap::<u32, VideoMatches>::new();
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
            let group = groups.entry(record.video).or_insert_with(|| VideoMatches {
                video: record.video,
                score: candidate.score,
                exact_tokens: candidate.exact_tokens(),
                exact_mask: candidate.exact_mask(),
                moments: MomentCounter::default(),
                documents: Vec::with_capacity(hits_per_group),
            });
            let candidate_rank = (
                candidate.exact_tokens(),
                candidate.exact_mask(),
                candidate.score,
            );
            let group_rank = (group.exact_tokens, group.exact_mask, group.score);
            if candidate_rank.0 > group_rank.0
                || (candidate_rank.0 == group_rank.0 && candidate_rank.1 > group_rank.1)
                || (candidate_rank.0 == group_rank.0
                    && candidate_rank.1 == group_rank.1
                    && candidate_rank.2 > group_rank.2)
            {
                group.exact_tokens = candidate_rank.0;
                group.exact_mask = candidate_rank.1;
                group.score = candidate_rank.2;
            }
            if candidate.matches_text() {
                group.moments.record(record.t_start_ms);
            }
            retain_best_document(&mut group.documents, candidate, hits_per_group);
        }

        let found = groups.len();
        let total_moments = groups.values().map(visible_group_moments).sum::<usize>();
        let mut groups = groups.into_values().collect::<Vec<_>>();
        self.sort_groups(&mut groups, params.sort_by.as_deref());
        let (offset, limit) = result_window(params);
        let mut grouped_hits = Vec::with_capacity(limit.min(groups.len()));
        for mut group in groups.into_iter().skip(offset).take(limit) {
            let group_found = visible_group_moments(&group);
            group.documents.sort_unstable_by(descending_doc_score);
            let video_id = self.inner.videos[group.video as usize].id.clone();
            let mut hits = Vec::with_capacity(group.documents.len());
            for document in group.documents {
                hits.push(self.hit_value(document.document(), terms, cache)?);
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

    fn format_ungrouped(
        &self,
        params: &SearchParams,
        terms: &[String],
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
                    },
                    right_video,
                    RelevanceRank {
                        score: right.score,
                        exact_tokens: right.exact_tokens(),
                        exact_mask: right.exact_mask(),
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
            hits.push(self.hit_value(candidate.document(), terms, cache)?);
        }
        Ok(json!({
            "found": found,
            "total_moments": total_moments,
            "out_of": self.inner.manifest.documents,
            "hits": hits,
        }))
    }

    fn sort_groups(&self, groups: &mut [VideoMatches], sort_by: Option<&str>) {
        groups.sort_unstable_by(|left, right| {
            let left_video = &self.inner.videos[left.video as usize];
            let right_video = &self.inner.videos[right.video as usize];
            compare_video_sort(
                left_video,
                RelevanceRank {
                    score: left.score,
                    exact_tokens: left.exact_tokens,
                    exact_mask: left.exact_mask,
                },
                right_video,
                RelevanceRank {
                    score: right.score,
                    exact_tokens: right.exact_tokens,
                    exact_mask: right.exact_mask,
                },
                sort_by,
                &self.inner.manifest,
            )
        });
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
                    },
                    right,
                    RelevanceRank {
                        score: 0.0,
                        exact_tokens: 0,
                        exact_mask: 0,
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
                    },
                    right_video,
                    RelevanceRank {
                        score: 0.0,
                        exact_tokens: 0,
                        exact_mask: 0,
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

    fn hit_value(&self, document: u32, terms: &[String], cache: &mut BlockCache) -> Result<Value> {
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
            && let Some(snippet) = highlighted_snippet(text, terms, 360)
        {
            highlights.push(json!({ "field": "text", "snippet": snippet }));
        }
        if let Some(title) = source.get("title").and_then(Value::as_str)
            && let Some(snippet) = highlighted_snippet(title, terms, 180)
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

fn compare_video_sort(
    left: &VideoMeta,
    left_rank: RelevanceRank,
    right: &VideoMeta,
    right_rank: RelevanceRank,
    sort_by: Option<&str>,
    manifest: &Manifest,
) -> Ordering {
    let ordering =
        match sort_by {
            Some("published_at_timestamp:desc") => right
                .published_at_timestamp
                .cmp(&left.published_at_timestamp),
            Some("published_at_timestamp:asc") => left
                .published_at_timestamp
                .cmp(&right.published_at_timestamp),
            Some("view_count:desc") => right.view_count.cmp(&left.view_count),
            Some("like_count:desc") => right.like_count.cmp(&left.like_count),
            Some("comment_count:desc") => right.comment_count.cmp(&left.comment_count),
            Some("engagement_score:desc") => right_rank
                .exact_tokens
                .cmp(&left_rank.exact_tokens)
                .then_with(|| right_rank.exact_mask.cmp(&left_rank.exact_mask))
                .then_with(|| {
                    blended_relevance(right_rank.score, right, manifest)
                        .total_cmp(&blended_relevance(left_rank.score, left, manifest))
                }),
            _ => right_rank
                .exact_tokens
                .cmp(&left_rank.exact_tokens)
                .then_with(|| right_rank.exact_mask.cmp(&left_rank.exact_mask))
                .then_with(|| right_rank.score.total_cmp(&left_rank.score)),
        };
    ordering
        .then_with(|| right_rank.score.total_cmp(&left_rank.score))
        .then_with(|| left.id.cmp(&right.id))
}

fn blended_relevance(relevance: f32, video: &VideoMeta, manifest: &Manifest) -> f32 {
    relevance
        + relevance.max(1.0) * ENGAGEMENT_RELEVANCE_BOOST * normalized_engagement(video, manifest)
}

fn normalized_engagement(video: &VideoMeta, manifest: &Manifest) -> f32 {
    VIEW_WEIGHT * normalized_log(video.view_count, manifest.max_view_count)
        + LIKE_WEIGHT * normalized_log(video.like_count, manifest.max_like_count)
        + COMMENT_WEIGHT * normalized_log(video.comment_count, manifest.max_comment_count)
}

fn normalized_log(value: u64, maximum: u64) -> f32 {
    if maximum == 0 {
        return 0.0;
    }
    (value as f32).ln_1p() / (maximum as f32).ln_1p()
}

fn visible_group_moments(group: &VideoMatches) -> usize {
    group.moments.visible()
}

fn retain_best_document(documents: &mut Vec<DocMatch>, candidate: DocMatch, limit: usize) {
    if documents.len() < limit {
        documents.push(candidate);
        return;
    }
    let Some((worst_index, worst)) = documents.iter().enumerate().min_by(|left, right| {
        left.1
            .exact_tokens()
            .cmp(&right.1.exact_tokens())
            .then_with(|| left.1.exact_mask().cmp(&right.1.exact_mask()))
            .then_with(|| left.1.score.total_cmp(&right.1.score))
    }) else {
        return;
    };
    if candidate.exact_tokens() > worst.exact_tokens()
        || (candidate.exact_tokens() == worst.exact_tokens()
            && candidate.exact_mask() > worst.exact_mask())
        || (candidate.exact_tokens() == worst.exact_tokens()
            && candidate.exact_mask() == worst.exact_mask()
            && candidate.score > worst.score)
    {
        documents[worst_index] = candidate;
    }
}

fn descending_doc_score(left: &DocMatch, right: &DocMatch) -> Ordering {
    right
        .exact_tokens()
        .cmp(&left.exact_tokens())
        .then_with(|| right.exact_mask().cmp(&left.exact_mask()))
        .then_with(|| right.score.total_cmp(&left.score))
        .then_with(|| left.document().cmp(&right.document()))
}

fn intersect_all_borrowed(lists: &[Vec<DocMatch>]) -> Vec<DocMatch> {
    let Some((smallest_index, smallest)) = lists
        .iter()
        .enumerate()
        .min_by_key(|(_, matches)| matches.len())
    else {
        return Vec::new();
    };
    let mut intersection = smallest.clone();
    for (index, list) in lists.iter().enumerate() {
        if index == smallest_index {
            continue;
        }
        intersection = intersect_sorted_borrowed(intersection, list);
        if intersection.is_empty() {
            break;
        }
    }
    intersection
}

fn retain_not_excluded(matches: &mut Vec<DocMatch>, excluded: &[u32]) {
    if excluded.is_empty() {
        return;
    }
    let mut excluded_index = 0_usize;
    matches.retain(|candidate| {
        let document = candidate.document();
        while excluded
            .get(excluded_index)
            .is_some_and(|excluded_document| *excluded_document < document)
        {
            excluded_index += 1;
        }
        excluded.get(excluded_index).copied() != Some(document)
    });
}

fn intersect_sorted_borrowed(left: Vec<DocMatch>, right: &[DocMatch]) -> Vec<DocMatch> {
    let mut output = Vec::with_capacity(left.len().min(right.len()));
    let mut left_index = 0_usize;
    let mut right_index = 0_usize;
    while left_index < left.len() && right_index < right.len() {
        match left[left_index]
            .document()
            .cmp(&right[right_index].document())
        {
            Ordering::Less => left_index += 1,
            Ordering::Greater => right_index += 1,
            Ordering::Equal => {
                let mut combined = DocMatch::new(
                    left[left_index].document(),
                    left[left_index].score + right[right_index].score,
                    left[left_index].matches_text() || right[right_index].matches_text(),
                );
                combined.set_exact_mask(
                    left[left_index].exact_mask() | right[right_index].exact_mask(),
                );
                output.push(combined);
                left_index += 1;
                right_index += 1;
            }
        }
    }
    output
}

fn intersect_sorted(left: Vec<DocMatch>, right: Vec<DocMatch>) -> Vec<DocMatch> {
    let mut output = Vec::with_capacity(left.len().min(right.len()));
    let mut left_index = 0_usize;
    let mut right_index = 0_usize;
    while left_index < left.len() && right_index < right.len() {
        match left[left_index]
            .document()
            .cmp(&right[right_index].document())
        {
            Ordering::Less => left_index += 1,
            Ordering::Greater => right_index += 1,
            Ordering::Equal => {
                let mut combined = DocMatch::new(
                    left[left_index].document(),
                    left[left_index].score + right[right_index].score,
                    left[left_index].matches_text() || right[right_index].matches_text(),
                );
                combined.set_exact_mask(
                    left[left_index].exact_mask() | right[right_index].exact_mask(),
                );
                output.push(combined);
                left_index += 1;
                right_index += 1;
            }
        }
    }
    output
}

fn union_all(mut lists: Vec<Vec<DocMatch>>) -> Vec<DocMatch> {
    if lists.is_empty() {
        return Vec::new();
    }
    let mut union = lists.remove(0);
    for list in lists {
        union = union_sorted(union, list);
    }
    union
}

fn union_sorted_ids(left: Vec<u32>, right: Vec<u32>) -> Vec<u32> {
    let mut output = Vec::with_capacity(left.len().saturating_add(right.len()));
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();
    loop {
        match (left.peek(), right.peek()) {
            (Some(left_value), Some(right_value)) => match left_value.cmp(right_value) {
                Ordering::Less => output.push(left.next().expect("peeked left")),
                Ordering::Greater => output.push(right.next().expect("peeked right")),
                Ordering::Equal => {
                    output.push(left.next().expect("peeked left"));
                    right.next();
                }
            },
            (Some(_), None) => {
                output.extend(left);
                break;
            }
            (None, Some(_)) => {
                output.extend(right);
                break;
            }
            (None, None) => break,
        }
    }
    output
}

fn union_sorted(left: Vec<DocMatch>, right: Vec<DocMatch>) -> Vec<DocMatch> {
    let mut output = Vec::with_capacity(left.len().saturating_add(right.len()));
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();
    loop {
        match (left.peek(), right.peek()) {
            (Some(left_value), Some(right_value)) => {
                match left_value.document().cmp(&right_value.document()) {
                    Ordering::Less => output.push(left.next().expect("peeked left")),
                    Ordering::Greater => output.push(right.next().expect("peeked right")),
                    Ordering::Equal => {
                        let left_value = left.next().expect("peeked left");
                        let right_value = right.next().expect("peeked right");
                        let mut combined = DocMatch::new(
                            left_value.document(),
                            left_value.score + right_value.score,
                            left_value.matches_text() || right_value.matches_text(),
                        );
                        combined.set_exact_mask(left_value.exact_mask() | right_value.exact_mask());
                        output.push(combined);
                    }
                }
            }
            (Some(_), None) => {
                output.extend(left);
                break;
            }
            (None, Some(_)) => {
                output.extend(right);
                break;
            }
            (None, None) => break,
        }
    }
    output
}

fn token_counts(value: &str) -> Result<TokenCounts> {
    let mut counts = TokenCounts {
        terms: HashMap::new(),
        tails: Vec::new(),
    };
    let mut token = String::new();
    let mut position = 0_u32;
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
            continue;
        }
        record_token(&mut counts, &mut token, &mut position)?;
    }
    record_token(&mut counts, &mut token, &mut position)?;
    Ok(counts)
}

fn record_token(counts: &mut TokenCounts, token: &mut String, position: &mut u32) -> Result<()> {
    if token.is_empty() {
        return Ok(());
    }
    let owned = std::mem::take(token);
    match counts.terms.entry(owned) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(TokenBuild {
                frequency: 1,
                first_position: *position,
                tail: None,
            });
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            let tail = u32::try_from(counts.tails.len()).map_err(|_| {
                Error::InvalidInput("one field contains more than 4 billion repeats".to_string())
            })?;
            counts.tails.push(PositionNode {
                position: *position,
                previous: entry.get().tail,
            });
            let entry = entry.get_mut();
            entry.frequency = entry.frequency.checked_add(1).ok_or_else(|| {
                Error::InvalidInput("one term occurs more than 65,535 times in a field".to_string())
            })?;
            entry.tail = Some(tail);
        }
    }
    *position = position
        .checked_add(1)
        .ok_or_else(|| Error::InvalidInput("one field exceeds 4 billion tokens".to_string()))?;
    Ok(())
}

fn parse_query(value: &str) -> Result<ParsedQuery> {
    let mut parser = QueryParser::default();
    let mut segment = String::new();
    let mut quoted = false;
    let mut quote_excluded = false;
    let mut allow_leading_exclusion = true;
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\'
            && characters
                .peek()
                .is_some_and(|next| matches!(next, '"' | '\\'))
        {
            segment.push(characters.next().expect("peeked escaped character"));
            continue;
        }
        if character == '"' {
            if quoted {
                record_quoted_segment(&mut segment, quote_excluded, &mut parser)?;
                quoted = false;
                allow_leading_exclusion = false;
            } else {
                quote_excluded = take_negative_phrase_marker(&mut segment, allow_leading_exclusion);
                record_unquoted_segment(&mut segment, allow_leading_exclusion, &mut parser)?;
                quoted = true;
            }
        } else {
            segment.push(character);
        }
    }
    if quoted {
        return Err(Error::InvalidInput(
            "query contains an unterminated quoted phrase".to_string(),
        ));
    }
    record_unquoted_segment(&mut segment, allow_leading_exclusion, &mut parser)?;
    Ok(ParsedQuery {
        terms: parser.terms,
        phrases: parser.phrases,
        excluded_terms: parser.excluded_terms,
        excluded_phrases: parser.excluded_phrases,
    })
}

fn record_quoted_segment(
    segment: &mut String,
    excluded: bool,
    parser: &mut QueryParser,
) -> Result<()> {
    let tokens = ordered_tokens(segment);
    segment.clear();
    if tokens.is_empty() {
        return Err(Error::InvalidInput(
            "quoted phrase must contain at least one searchable token".to_string(),
        ));
    }
    if tokens.len() > MAX_PHRASE_TOKENS {
        return Err(Error::InvalidInput(format!(
            "quoted phrase exceeds {MAX_PHRASE_TOKENS} tokens"
        )));
    }
    if excluded {
        if tokens.len() == 1 {
            let token = tokens.into_iter().next().expect("one quoted token");
            if parser.excluded_seen.insert(token.clone()) {
                parser.excluded_terms.push(token);
            }
        } else if !parser.excluded_phrases.contains(&tokens) {
            parser.excluded_phrases.push(tokens);
        }
    } else {
        for token in &tokens {
            if parser.seen.insert(token.clone()) {
                parser.terms.push(token.clone());
            }
        }
        parser.phrases.push(tokens);
    }
    Ok(())
}

fn record_unquoted_segment(
    segment: &mut String,
    allow_leading_exclusion: bool,
    parser: &mut QueryParser,
) -> Result<()> {
    for (token, excluded) in signed_tokens(segment, allow_leading_exclusion)? {
        if excluded {
            if parser.excluded_seen.insert(token.clone()) {
                parser.excluded_terms.push(token);
            }
        } else if parser.seen.insert(token.clone()) {
            parser.terms.push(token);
        }
    }
    segment.clear();
    Ok(())
}

fn take_negative_phrase_marker(segment: &mut String, allow_leading_exclusion: bool) -> bool {
    let Some(marker_index) = segment.len().checked_sub(1) else {
        return false;
    };
    if segment.as_bytes()[marker_index] != b'-' {
        return false;
    }
    let valid = if marker_index == 0 {
        allow_leading_exclusion
    } else {
        segment[..marker_index]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    };
    if valid {
        segment.truncate(marker_index);
    }
    valid
}

fn signed_tokens(value: &str, allow_leading_exclusion: bool) -> Result<Vec<(String, bool)>> {
    let mut output = Vec::new();
    let mut token = String::new();
    let mut excluded = false;
    let mut can_exclude = allow_leading_exclusion;
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
            can_exclude = false;
        } else if character == '-' && token.is_empty() && can_exclude {
            excluded = true;
            can_exclude = false;
        } else {
            if excluded && token.is_empty() {
                return Err(Error::InvalidInput(
                    "negative marker must directly prefix a searchable token".to_string(),
                ));
            }
            if !token.is_empty() {
                output.push((std::mem::take(&mut token), excluded));
                excluded = false;
            }
            can_exclude = character.is_whitespace();
        }
    }
    if excluded && token.is_empty() {
        return Err(Error::InvalidInput(
            "negative marker must directly prefix a searchable token".to_string(),
        ));
    }
    if !token.is_empty() {
        output.push((token, excluded));
    }
    Ok(output)
}

fn ordered_tokens(value: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut token = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
        } else if !token.is_empty() {
            output.push(std::mem::take(&mut token));
        }
    }
    if !token.is_empty() {
        output.push(token);
    }
    output
}

fn ordered_phrase_positions_match(cursors: &[PhraseCursor<'_>], slop: u8) -> bool {
    let Some(first) = cursors.first() else {
        return false;
    };
    first.positions.iter().copied().any(|start| {
        let mut previous = start;
        let mut used_slop = 0_u32;
        for cursor in cursors.iter().skip(1) {
            let next_index = cursor
                .positions
                .partition_point(|position| *position <= previous);
            let Some(&next) = cursor.positions.get(next_index) else {
                return false;
            };
            used_slop = used_slop.saturating_add(next - previous - 1);
            if used_slop > u32::from(slop) {
                return false;
            }
            previous = next;
        }
        true
    })
}

fn minimum_position_window(cursors: &[PhraseCursor<'_>]) -> Option<PositionalWindow> {
    if cursors.len() < 2 || cursors.iter().any(|cursor| cursor.positions.is_empty()) {
        return None;
    }
    let mut indices = vec![0_usize; cursors.len()];
    let mut best: Option<PositionalWindow> = None;
    loop {
        let mut minimum = (0_usize, u32::MAX);
        let mut maximum = 0_u32;
        for (cursor_index, (cursor, &position_index)) in cursors.iter().zip(&indices).enumerate() {
            let position = cursor.positions[position_index];
            if position < minimum.1 {
                minimum = (cursor_index, position);
            }
            maximum = maximum.max(position);
        }
        let window = PositionalWindow {
            displacement: maximum - minimum.1,
            max_offset: maximum,
        };
        if best.is_none_or(|current| {
            window.displacement < current.displacement
                || (window.displacement == current.displacement
                    && window.max_offset < current.max_offset)
        }) {
            best = Some(window);
        }
        if window.displacement == cursors.len().saturating_sub(1) as u32 {
            break;
        }
        let minimum_index = &mut indices[minimum.0];
        *minimum_index += 1;
        if *minimum_index == cursors[minimum.0].positions.len() {
            break;
        }
    }
    best
}

fn positional_window(windows: &[(u32, PositionalWindow)], id: u32) -> Option<PositionalWindow> {
    windows
        .binary_search_by_key(&id, |(candidate, _)| *candidate)
        .ok()
        .map(|index| windows[index].1)
}

fn proximity_multiplier(window: PositionalWindow, tokens: usize, prioritize_position: bool) -> f32 {
    const TYPESENSE_WINDOW: u32 = 10;
    if window.displacement > TYPESENSE_WINDOW {
        return 1.0;
    }
    let minimum_displacement = tokens.saturating_sub(1) as u32;
    let extra_gap = window.displacement.saturating_sub(minimum_displacement);
    let proximity = PROXIMITY_SCORE_BOOST / (extra_gap as f32 + 1.0).sqrt();
    let position = if prioritize_position {
        PROXIMITY_POSITION_BOOST / (window.max_offset as f32 + 1.0).sqrt()
    } else {
        0.0
    };
    1.0 + proximity + position
}

fn validate_search_params(params: &SearchParams) -> Result<()> {
    validate_query_and_window(params)?;
    query_fields(params.query_by.as_deref(), false)?;
    parse_filter(params.filter_by.as_deref())?;
    if let Some(group_by) = params.group_by.as_deref()
        && group_by != "video_id"
    {
        return Err(Error::InvalidInput(format!(
            "unsupported group_by field: {group_by}"
        )));
    }
    if params.group_limit == Some(0) {
        return Err(Error::InvalidInput(
            "group_limit must be at least 1".to_string(),
        ));
    }
    if params
        .group_limit
        .is_some_and(|value| value > MAX_PAGE_SIZE)
    {
        return Err(Error::InvalidInput(format!(
            "group_limit cannot exceed {MAX_PAGE_SIZE}"
        )));
    }
    if let Some(sort_by) = params.sort_by.as_deref()
        && !matches!(
            sort_by,
            "published_at_timestamp:desc"
                | "published_at_timestamp:asc"
                | "view_count:desc"
                | "like_count:desc"
                | "comment_count:desc"
                | "engagement_score:desc"
        )
    {
        return Err(Error::InvalidInput(format!(
            "unsupported sort_by expression: {sort_by}"
        )));
    }

    if let Some(weights) = params.query_by_weights.as_deref() {
        let fields = params.query_by.as_deref().unwrap_or("text,title");
        let expected = match fields {
            "text,title" => "4,6",
            "title,text" => "6,4",
            "text" => "4",
            "title" => "6",
            _ => {
                return Err(Error::InvalidInput(
                    "query_by_weights requires an exact supported query_by order".to_string(),
                ));
            }
        };
        if weights != expected {
            return Err(Error::InvalidInput(format!(
                "query_by_weights must be {expected} for query_by={fields}"
            )));
        }
    }
    if params
        .text_match_type
        .as_deref()
        .is_some_and(|value| value != "sum_score")
    {
        return Err(Error::InvalidInput(
            "text_match_type must be sum_score".to_string(),
        ));
    }
    if params.prioritize_exact_match == Some(false) {
        return Err(Error::InvalidInput(
            "prioritize_exact_match=false is unsupported".to_string(),
        ));
    }
    if params.prioritize_token_position == Some(false) {
        return Err(Error::InvalidInput(
            "prioritize_token_position=false is unsupported".to_string(),
        ));
    }
    if params
        .phrase_slop
        .is_some_and(|slop| slop > MAX_PHRASE_SLOP)
    {
        return Err(Error::InvalidInput(format!(
            "phrase_slop cannot exceed {MAX_PHRASE_SLOP}"
        )));
    }
    if params.typo_tokens_threshold.is_some_and(|value| value != 5) {
        return Err(Error::InvalidInput(
            "typo_tokens_threshold must be 5".to_string(),
        ));
    }
    if params.drop_tokens_threshold.is_some_and(|value| value != 1) {
        return Err(Error::InvalidInput(
            "drop_tokens_threshold must be 1".to_string(),
        ));
    }
    if params
        .drop_tokens_mode
        .as_deref()
        .is_some_and(|value| value != "right_to_left")
    {
        return Err(Error::InvalidInput(
            "drop_tokens_mode must be right_to_left".to_string(),
        ));
    }
    if params
        .split_join_tokens
        .as_deref()
        .is_some_and(|value| value != "always")
    {
        return Err(Error::InvalidInput(
            "split_join_tokens must be always".to_string(),
        ));
    }
    if let Some(fields) = params.highlight_fields.as_deref() {
        query_fields(Some(fields), false)?;
    }
    if params
        .include_fields
        .as_deref()
        .is_some_and(|fields| fields != ACTIVE_INCLUDE_FIELDS)
    {
        return Err(Error::InvalidInput(
            "unsupported include_fields projection".to_string(),
        ));
    }
    Ok(())
}

fn validate_query_and_window(params: &SearchParams) -> Result<()> {
    let query = params.q.trim();
    if query.is_empty() {
        return Err(Error::InvalidInput("q cannot be empty".to_string()));
    }
    if params.q.len() > MAX_QUERY_BYTES {
        return Err(Error::InvalidInput(format!(
            "q exceeds {MAX_QUERY_BYTES} bytes"
        )));
    }

    for (name, value) in [("per_page", params.per_page), ("limit", params.limit)] {
        if value.is_some_and(|value| value > MAX_PAGE_SIZE) {
            return Err(Error::InvalidInput(format!(
                "{name} cannot exceed {MAX_PAGE_SIZE}"
            )));
        }
    }
    if params.limit.is_some() && (params.page.is_some() || params.per_page.is_some()) {
        return Err(Error::InvalidInput(
            "limit/offset cannot be combined with page/per_page".to_string(),
        ));
    }
    if params.offset.is_some() && params.limit.is_none() {
        return Err(Error::InvalidInput("offset requires limit".to_string()));
    }
    Ok(())
}

fn validate_channel_search_params(params: &SearchParams) -> Result<()> {
    validate_query_and_window(params)?;
    if params
        .query_by
        .as_deref()
        .is_some_and(|fields| fields != "name,handle")
    {
        return Err(Error::InvalidInput(
            "channel query_by must be name,handle".to_string(),
        ));
    }
    for (name, present) in [
        ("query_by_weights", params.query_by_weights.is_some()),
        ("filter_by", params.filter_by.is_some()),
        ("group_by", params.group_by.is_some()),
        ("group_limit", params.group_limit.is_some()),
        ("sort_by", params.sort_by.is_some()),
        ("text_match_type", params.text_match_type.is_some()),
        (
            "prioritize_exact_match",
            params.prioritize_exact_match.is_some(),
        ),
        (
            "prioritize_token_position",
            params.prioritize_token_position.is_some(),
        ),
        ("phrase_slop", params.phrase_slop.is_some()),
        (
            "typo_tokens_threshold",
            params.typo_tokens_threshold.is_some(),
        ),
        (
            "drop_tokens_threshold",
            params.drop_tokens_threshold.is_some(),
        ),
        ("drop_tokens_mode", params.drop_tokens_mode.is_some()),
        ("split_join_tokens", params.split_join_tokens.is_some()),
        ("highlight_fields", params.highlight_fields.is_some()),
        ("include_fields", params.include_fields.is_some()),
    ] {
        if present {
            return Err(Error::InvalidInput(format!(
                "unsupported channel search parameter: {name}"
            )));
        }
    }
    Ok(())
}

fn query_fields(query_by: Option<&str>, prioritize_position: bool) -> Result<QueryFields> {
    let requested = query_by.unwrap_or("text,title");
    let mut text = false;
    let mut title = false;
    for field in requested.split(',').map(str::trim) {
        match field {
            "text" if !text => text = true,
            "title" if !title => title = true,
            "text" | "title" => {
                return Err(Error::InvalidInput(format!(
                    "query_by contains duplicate field: {field}"
                )));
            }
            "" => {
                return Err(Error::InvalidInput(
                    "query_by cannot contain an empty field".to_string(),
                ));
            }
            unsupported => {
                return Err(Error::InvalidInput(format!(
                    "unsupported query_by field: {unsupported}"
                )));
            }
        }
    }
    Ok(QueryFields {
        text,
        title,
        prioritize_position,
        exact_mask: 1,
    })
}

fn inverse_document_frequency(documents: u32, frequency: u32) -> f32 {
    let documents = documents as f32;
    let frequency = frequency as f32;
    (1.0 + (documents - frequency + 0.5) / (frequency + 0.5)).ln()
}

fn bm25(frequency: u32, document_tokens: u32, average_tokens: f32) -> f32 {
    const K1: f32 = 1.2;
    const B: f32 = 0.75;
    let frequency = frequency as f32;
    let normalized_length = if average_tokens > 0.0 {
        document_tokens as f32 / average_tokens
    } else {
        1.0
    };
    frequency * (K1 + 1.0) / (frequency + K1 * (1.0 - B + B * normalized_length))
}

fn token_position_multiplier(first_position: u32, enabled: bool) -> f32 {
    if !enabled {
        return 1.0;
    }
    1.0 + POSITION_SCORE_BOOST / (first_position as f32 + 1.0).sqrt()
}

fn result_window(params: &SearchParams) -> (usize, usize) {
    if let Some(limit) = params.limit {
        return (
            params.offset.unwrap_or_default() as usize,
            usize::from(limit),
        );
    }
    let limit = usize::from(params.per_page.unwrap_or(10));
    let page = params.page.unwrap_or(1).max(1) as usize;
    ((page - 1).saturating_mul(limit), limit)
}

fn parse_filter(filter: Option<&str>) -> Result<Option<(&str, &str)>> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    let (field, value) = filter
        .split_once(":=")
        .ok_or_else(|| Error::InvalidInput("filter_by must use an exact := filter".to_string()))?;
    let field = field.trim();
    if !matches!(field, "video_id" | "youtuber_handle") {
        return Err(Error::InvalidInput(format!(
            "unsupported filter_by field: {field}"
        )));
    }
    let value = value.trim();
    if value.contains(":=")
        || value.contains("&&")
        || value.contains("||")
        || value.chars().any(|character| {
            matches!(
                character,
                '[' | ']' | '(' | ')' | '{' | '}' | ',' | '<' | '>'
            )
        })
    {
        return Err(Error::InvalidInput(
            "compound filter_by expressions are unsupported".to_string(),
        ));
    }
    let value = match (value.strip_prefix('`'), value.strip_suffix('`')) {
        (Some(without_prefix), Some(_)) => without_prefix
            .strip_suffix('`')
            .ok_or_else(|| Error::InvalidInput("filter_by has malformed backticks".to_string()))?,
        (None, None) => value,
        _ => {
            return Err(Error::InvalidInput(
                "filter_by has malformed backticks".to_string(),
            ));
        }
    };
    if value.contains('`') {
        return Err(Error::InvalidInput(
            "filter_by has malformed backticks".to_string(),
        ));
    }
    if value.is_empty() {
        return Err(Error::InvalidInput(
            "filter_by value cannot be empty".to_string(),
        ));
    }
    if value.len() > MAX_FILTER_VALUE_BYTES {
        return Err(Error::InvalidInput(format!(
            "filter_by value exceeds {MAX_FILTER_VALUE_BYTES} bytes"
        )));
    }
    Ok(Some((field, value)))
}

fn video_matches_filter(video: &VideoMeta, filter: Option<(&str, &str)>) -> bool {
    let Some((field, value)) = filter else {
        return true;
    };
    match field {
        "video_id" => video.id == value,
        "youtuber_handle" => video.handle.eq_ignore_ascii_case(value),
        _ => true,
    }
}

fn bounded_typo_distance(term: &str) -> u8 {
    if term.len() < MIN_ONE_TYPO_BYTES {
        0
    } else if term.len() < MIN_TWO_TYPO_BYTES {
        1
    } else {
        2
    }
}

fn highlighted_snippet(value: &str, terms: &[String], maximum_chars: usize) -> Option<String> {
    let lowercase = value.to_ascii_lowercase();
    let (start, term_length) = terms
        .iter()
        .filter_map(|term| lowercase.find(term).map(|start| (start, term.len())))
        .min_by_key(|(start, _)| *start)?;
    let mut snippet_start = start.saturating_sub(maximum_chars / 3);
    while !value.is_char_boundary(snippet_start) {
        snippet_start += 1;
    }
    let mut snippet_end = (snippet_start + maximum_chars).min(value.len());
    while !value.is_char_boundary(snippet_end) {
        snippet_end -= 1;
    }
    let mark_start = start.max(snippet_start);
    let mark_end = (start + term_length).min(snippet_end);
    let mut output = String::with_capacity(snippet_end - snippet_start + 13);
    escape_html(&value[snippet_start..mark_start], &mut output);
    output.push_str("<mark>");
    escape_html(&value[mark_start..mark_end], &mut output);
    output.push_str("</mark>");
    escape_html(&value[mark_end..snippet_end], &mut output);
    Some(output)
}

fn escape_html(value: &str, output: &mut String) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(character),
        }
    }
}

fn read_json_lines(path: &Path) -> Result<Vec<Value>> {
    let mut values = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if !line.trim().is_empty() {
            values.push(serde_json::from_str(&line)?);
        }
    }
    Ok(values)
}

fn directory_size(path: &Path) -> Result<u64> {
    let mut pending = vec![PathBuf::from(path)];
    let mut bytes = 0_u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    Ok(bytes)
}

fn build_manifest_files(index_path: &Path) -> Result<BTreeMap<String, ManifestFile>> {
    let mut files = BTreeMap::new();
    for name in GENERATION_FILES {
        let path = index_path.join(name);
        let bytes = fs::metadata(&path)?.len();
        files.insert(
            name.to_string(),
            ManifestFile {
                bytes,
                blake3: checksum_file(&path)?,
            },
        );
    }
    Ok(files)
}

fn validate_manifest_files(index_path: &Path, manifest: &Manifest) -> Result<()> {
    if manifest.files.len() != GENERATION_FILES.len() {
        return Err(Error::Corrupt(format!(
            "manifest records {} generation files, expected {}",
            manifest.files.len(),
            GENERATION_FILES.len()
        )));
    }
    for name in GENERATION_FILES {
        let expected = manifest
            .files
            .get(name)
            .ok_or_else(|| Error::Corrupt(format!("manifest is missing generation file {name}")))?;
        let path = index_path.join(name);
        let actual_bytes = fs::metadata(&path)?.len();
        if actual_bytes != expected.bytes {
            return Err(Error::Corrupt(format!(
                "generation file {name} is {actual_bytes} bytes, expected {}",
                expected.bytes
            )));
        }
        let actual_checksum = checksum_file(&path)?;
        if actual_checksum != expected.blake3 {
            return Err(Error::Corrupt(format!(
                "generation file {name} failed its BLAKE3 checksum"
            )));
        }
    }
    Ok(())
}

fn checksum_file(path: &Path) -> Result<String> {
    let mut input = BufReader::new(File::open(path)?);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::OpenOptions, io::Write};
    use tempfile::TempDir;

    fn test_engine() -> (TempDir, RawSearchEngine) {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"id":"a-1","video_id":"a","title":"Rust performance","text":"a compact inverted search index about relativity","youtuber_handle":"@one","youtuber_name":"One","t_start_ms":1000,"t_end_ms":2000,"published_at_timestamp":20}),
            json!({"id":"a-2","video_id":"a","title":"Rust performance","text":"memory mapped search is fast","youtuber_handle":"@one","youtuber_name":"One","t_start_ms":30000,"t_end_ms":31000,"published_at_timestamp":20}),
            json!({"id":"b-1","video_id":"b","title":"JavaScript runtimes","text":"Bun starts quickly","youtuber_handle":"@Two","youtuber_name":"Two","t_start_ms":1000,"t_end_ms":2000,"published_at_timestamp":10}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        let mut channels = File::create(&channels_path).expect("channels");
        writeln!(
            channels,
            "{}",
            json!({"id":"one","name":"One","handle":"@one"})
        )
        .expect("write channel");
        build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
        let engine = RawSearchEngine::open(&index_path).expect("open raw index");
        (root, engine)
    }

    fn blackhole_engine() -> (TempDir, RawSearchEngine) {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"id":"literal-1","video_id":"literal","title":"Compiler blackhole identifier","text":"the blackhole optimization marker","youtuber_handle":"@other","youtuber_name":"Other","published_at":"1970-01-01T00:00:30Z"}),
            json!({"id":"veritasium-1","video_id":"veritasium","title":"A Black Hole Is Not Black","text":"a black hole bends spacetime","youtuber_handle":"@Veritasium","youtuber_name":"Veritasium","published_at":"1970-01-01T00:00:10Z"}),
            json!({"id":"vsauce-1","video_id":"vsauce","title":"Questions About Space","text":"what happens near black holes","youtuber_handle":"@Vsauce","youtuber_name":"Vsauce","published_at":"1970-01-01T00:00:20Z"}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        let mut channels = File::create(&channels_path).expect("channels");
        for channel in [
            json!({"id":"veritasium","name":"Veritasium","handle":"@veritasium"}),
            json!({"id":"vsauce","name":"Vsauce","handle":"@vsauce"}),
            json!({"id":"other","name":"Other","handle":"@other"}),
        ] {
            writeln!(channels, "{channel}").expect("write channel");
        }
        build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
        let engine = RawSearchEngine::open(&index_path).expect("open raw index");
        (root, engine)
    }

    fn weighted_blackhole_engine() -> (TempDir, RawSearchEngine) {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"id":"black-heavy-1","video_id":"black-heavy","title":"black black black black black hole","text":"space","youtuber_handle":"@one","youtuber_name":"One"}),
            json!({"id":"hole-heavy-1","video_id":"hole-heavy","title":"black hole hole hole hole","text":"space","youtuber_handle":"@two","youtuber_name":"Two"}),
            json!({"id":"blank-distractor-1","video_id":"blank-distractor","title":"blank blank blank blank tape with a hole","text":"space","youtuber_handle":"@three","youtuber_name":"Three"}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        File::create(&channels_path).expect("channels");
        build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
        let engine = RawSearchEngine::open(&index_path).expect("open raw index");
        (root, engine)
    }

    fn engagement_engine() -> (TempDir, RawSearchEngine) {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"id":"small-1","video_id":"small","title":"Space systems","text":"orbital mechanics explained","youtuber_handle":"@one","youtuber_name":"One","view_count":100,"like_count":20,"comment_count":2}),
            json!({"id":"large-1","video_id":"large","title":"Space systems","text":"orbital mechanics explained","youtuber_handle":"@two","youtuber_name":"Two","view_count":1_000_000,"like_count":100_000,"comment_count":10_000}),
            json!({"id":"exact-1","video_id":"exact","title":"Orbital mechanics","text":"an introduction","youtuber_handle":"@three","youtuber_name":"Three","view_count":1,"like_count":1,"comment_count":1}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        File::create(&channels_path).expect("channels");
        build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
        let engine = RawSearchEngine::open(&index_path).expect("open raw index");
        (root, engine)
    }

    fn position_engine() -> (TempDir, RawSearchEngine) {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"id":"early-1","video_id":"early","title":"Same","text":"target alpha beta gamma","youtuber_handle":"@one","youtuber_name":"One"}),
            json!({"id":"late-1","video_id":"late","title":"Same","text":"alpha beta gamma target","youtuber_handle":"@two","youtuber_name":"Two"}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        File::create(&channels_path).expect("channels");
        build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
        let engine = RawSearchEngine::open(&index_path).expect("open raw index");
        (root, engine)
    }

    fn token_drop_engine() -> (TempDir, RawSearchEngine) {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"id":"prefix-pair-1","video_id":"prefix-pair","title":"Alpha beta","text":"paired terms","youtuber_handle":"@one","youtuber_name":"One"}),
            json!({"id":"suffix-pair-1","video_id":"suffix-pair","title":"Beta gamma","text":"paired terms","youtuber_handle":"@two","youtuber_name":"Two"}),
            json!({"id":"single-1","video_id":"single","title":"Alpha alone","text":"one term only","youtuber_handle":"@three","youtuber_name":"Three"}),
            json!({"id":"exact-relativity-1","video_id":"exact-relativity","title":"Relativity explained","text":"a clear account","youtuber_handle":"@one","youtuber_name":"One"}),
            json!({"id":"fuzzy-relativity-1","video_id":"fuzzy-relativity","title":"Relatively explained explained explained explained","text":"a repeated account","youtuber_handle":"@two","youtuber_name":"Two"}),
            json!({"id":"early-exact-1","video_id":"early-exact","title":"Planet oribt","text":"an account","youtuber_handle":"@one","youtuber_name":"One"}),
            json!({"id":"late-exact-1","video_id":"late-exact","title":"Planat orbit orbit orbit","text":"a repeated account","youtuber_handle":"@two","youtuber_name":"Two"}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        File::create(&channels_path).expect("channels");
        build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
        let engine = RawSearchEngine::open(&index_path).expect("open raw index");
        (root, engine)
    }

    fn grouped_query(query: &str) -> SearchParams {
        SearchParams {
            q: query.to_string(),
            query_by: Some("text,title".to_string()),
            filter_by: None,
            group_by: Some("video_id".to_string()),
            group_limit: Some(3),
            per_page: None,
            page: None,
            offset: Some(0),
            limit: Some(10),
            sort_by: None,
            use_cache: None,
            cache_ttl: None,
            ..SearchParams::default()
        }
    }

    #[test]
    fn raw_index_groups_chunks_and_highlights_matches() {
        let (_root, engine) = test_engine();
        let response = engine
            .search(&grouped_query("memory mapped"))
            .expect("search");
        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "a");
        assert!(
            response["grouped_hits"][0]["hits"][0]["highlights"][0]["snippet"]
                .as_str()
                .expect("snippet")
                .contains("<mark>")
        );
    }

    #[test]
    fn schema_six_reconstructs_complete_positions_from_aligned_sidecars() {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        writeln!(
            File::create(&chunks_path).expect("chunks"),
            "{}",
            json!({
                "video_id":"a",
                "title":"Positions",
                "text":"one echo two echo echo",
                "youtuber_handle":"@one"
            })
        )
        .expect("write chunk");
        File::create(&channels_path).expect("channels");
        build_index(&chunks_path, &channels_path, &index_path).expect("build");
        let engine = RawSearchEngine::open(&index_path).expect("open");

        let entry = engine
            .inner
            .lexicon
            .lookup("echo")
            .expect("lookup")
            .expect("echo term");
        let posting_bytes = engine
            .postings_range(entry.text_offset, entry.text_length)
            .expect("posting range");
        let mut posting_cursor = 0;
        assert_eq!(
            read_varint(posting_bytes, &mut posting_cursor, posting_bytes.len()).expect("document"),
            0
        );
        let frequency = read_varint(posting_bytes, &mut posting_cursor, posting_bytes.len())
            .expect("frequency");
        assert_eq!(posting_cursor, posting_bytes.len());

        let first_bytes = engine
            .inner
            .positions
            .range(entry.term, false)
            .expect("first positions");
        let mut first_cursor = 0;
        let first =
            read_varint(first_bytes, &mut first_cursor, first_bytes.len()).expect("first position");
        assert_eq!(first_cursor, first_bytes.len());
        let remaining = engine
            .inner
            .full_positions
            .as_ref()
            .expect("schema 6 positions")
            .range(entry.term, false)
            .expect("remaining positions");
        let mut positions = Vec::new();
        let mut cursor = crate::positions::PositionCursor::new(remaining);
        cursor
            .read_into(first, frequency, &mut positions)
            .expect("decode positions");
        cursor.finish().expect("consume position range");
        assert_eq!(positions, [1, 3, 4]);
    }

    #[test]
    fn query_parser_preserves_phrase_order_and_duplicate_tokens() {
        let parsed = parse_query(r#"outside "black black hole" tail"#).expect("parse query");
        assert_eq!(parsed.terms, ["outside", "black", "hole", "tail"]);
        assert_eq!(parsed.phrases, [["black", "black", "hole"]]);
        assert!(parsed.excluded_terms.is_empty());
        assert!(parsed.excluded_phrases.is_empty());

        let escaped = parse_query(r#"say \"hello\""#).expect("escaped quotes");
        assert_eq!(escaped.terms, ["say", "hello"]);
        assert!(escaped.phrases.is_empty());
        assert!(escaped.excluded_terms.is_empty());
        assert!(escaped.excluded_phrases.is_empty());

        let excluded = parse_query("alpha -beta x-ray -beta").expect("negative term");
        assert_eq!(excluded.terms, ["alpha", "x", "ray"]);
        assert_eq!(excluded.excluded_terms, ["beta"]);
        assert!(excluded.excluded_phrases.is_empty());

        let excluded_phrase =
            parse_query(r#"outside -"black black hole" -"tail""#).expect("negative phrases");
        assert_eq!(excluded_phrase.terms, ["outside"]);
        assert_eq!(excluded_phrase.excluded_terms, ["tail"]);
        assert_eq!(
            excluded_phrase.excluded_phrases,
            [["black", "black", "hole"]]
        );

        let adjacent = parse_query(r#""alpha"-beta"#).expect("adjacent hyphen");
        assert_eq!(adjacent.terms, ["alpha", "beta"]);
        assert!(adjacent.excluded_terms.is_empty());

        assert!(parse_query(r#""unterminated"#).is_err());
        assert!(parse_query(r#"empty "" phrase"#).is_err());
        for invalid in ["alpha - beta", "alpha -", "--alpha", r#"alpha - "beta""#] {
            assert!(parse_query(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn quoted_phrases_require_order_adjacency_and_one_field() {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"video_id":"adjacent","title":"A","text":"black hole forms in deep space","youtuber_handle":"@one"}),
            json!({"video_id":"gapped","title":"B","text":"black deep hole","youtuber_handle":"@one"}),
            json!({"video_id":"reversed","title":"C","text":"hole black","youtuber_handle":"@one"}),
            json!({"video_id":"duplicate","title":"D","text":"black black hole","youtuber_handle":"@one"}),
            json!({"video_id":"title","title":"Black Hole Lecture","text":"astronomy","youtuber_handle":"@one"}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        File::create(&channels_path).expect("channels");
        build_index(&chunks_path, &channels_path, &index_path).expect("build");
        let engine = RawSearchEngine::open(&index_path).expect("open");

        let ids = |query: &str, query_by: &str, slop: Option<u8>| {
            let mut params = grouped_query(query);
            params.query_by = Some(query_by.to_string());
            params.phrase_slop = slop;
            let response = engine.search(&params).expect("phrase search");
            response["grouped_hits"]
                .as_array()
                .expect("groups")
                .iter()
                .map(|group| {
                    group["group_key"][0]
                        .as_str()
                        .expect("group key")
                        .to_string()
                })
                .collect::<HashSet<_>>()
        };

        assert_eq!(
            ids(r#""black hole""#, "text", None),
            HashSet::from(["adjacent".to_string(), "duplicate".to_string()])
        );
        assert_eq!(
            ids(r#""black black""#, "text", None),
            HashSet::from(["duplicate".to_string()])
        );
        assert_eq!(
            ids(r#""hole black""#, "text", None),
            HashSet::from(["reversed".to_string()])
        );
        assert_eq!(
            ids(r#""black hole" deep"#, "text", None),
            HashSet::from(["adjacent".to_string()])
        );
        assert_eq!(
            ids(r#""black hole""#, "title", None),
            HashSet::from(["title".to_string()])
        );
        assert_eq!(
            ids(r#""black hole""#, "text", Some(1)),
            HashSet::from([
                "adjacent".to_string(),
                "gapped".to_string(),
                "duplicate".to_string(),
            ])
        );
        assert_eq!(
            ids(r#""black hole" -deep"#, "text", None),
            HashSet::from(["duplicate".to_string()])
        );
        assert_eq!(
            ids(r#"black -"deep space""#, "text", None),
            HashSet::from([
                "duplicate".to_string(),
                "gapped".to_string(),
                "reversed".to_string(),
            ])
        );
        assert_eq!(
            ids(r#""black hole" -"deep space""#, "text", None),
            HashSet::from(["duplicate".to_string()])
        );
        assert_eq!(
            ids(r#""black hole" -"deep hole""#, "text", Some(1)),
            HashSet::from(["adjacent".to_string(), "duplicate".to_string()])
        );
        assert_eq!(
            ids(r#"black -"black black""#, "text", None),
            HashSet::from([
                "adjacent".to_string(),
                "gapped".to_string(),
                "reversed".to_string(),
            ])
        );
        assert!(ids(r#"black -"black hole""#, "title", None).is_empty());
        let without_deep = HashSet::from(["duplicate".to_string(), "reversed".to_string()]);
        assert_eq!(ids("bla -deep", "text", None), without_deep);
        assert_eq!(ids("blak -deep", "text", None), without_deep);
        assert_eq!(ids("black missing -deep", "text", None), without_deep);
        assert!(ids("black -deap", "text", None).contains("adjacent"));
        assert!(ids("black -lecture", "title", None).is_empty());
        assert!(ids("black hole", "text", None).contains("gapped"));
        for query in ["black hole", "*", r#"-"black hole""#] {
            assert!(
                engine
                    .search(&SearchParams {
                        q: query.to_string(),
                        phrase_slop: Some(1),
                        ..SearchParams::default()
                    })
                    .is_err()
            );
        }
        for query in ["-deep", "* -deep"] {
            assert_eq!(
                ids(query, "text", None),
                HashSet::from([
                    "duplicate".to_string(),
                    "reversed".to_string(),
                    "title".to_string(),
                ])
            );
        }
        for query in [r#"-"black hole""#, r#"* -"black hole""#] {
            assert_eq!(
                ids(query, "text", None),
                HashSet::from([
                    "gapped".to_string(),
                    "reversed".to_string(),
                    "title".to_string(),
                ])
            );
        }
    }

    #[test]
    fn complete_positions_prefer_the_tightest_bounded_token_window() {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let index_path = root.path().join("index");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for document in [
            json!({"video_id":"wide","title":"Wide","text":"alpha x beta x x x x","youtuber_handle":"@one"}),
            json!({"video_id":"tight","title":"Tight","text":"alpha x x beta x alpha beta","youtuber_handle":"@one"}),
        ] {
            writeln!(chunks, "{document}").expect("write chunk");
        }
        File::create(&channels_path).expect("channels");
        build_index(&chunks_path, &channels_path, &index_path).expect("build");
        let engine = RawSearchEngine::open(&index_path).expect("open");
        let mut query = grouped_query("alpha beta");
        query.query_by = Some("text".to_string());
        let response = engine.search(&query).expect("proximity search");
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "tight");

        let exact = proximity_multiplier(
            PositionalWindow {
                displacement: 1,
                max_offset: 1,
            },
            2,
            true,
        );
        let gapped = proximity_multiplier(
            PositionalWindow {
                displacement: 2,
                max_offset: 2,
            },
            2,
            true,
        );
        assert!(exact > gapped && exact <= 1.25);
        assert_eq!(
            proximity_multiplier(
                PositionalWindow {
                    displacement: 11,
                    max_offset: 11,
                },
                2,
                true,
            ),
            1.0
        );
    }

    #[test]
    fn grouped_totals_include_all_moments_beyond_the_return_window() {
        let (_root, engine) = test_engine();
        let mut query = grouped_query("search");
        query.query_by = Some("text".to_string());
        query.group_limit = Some(1);
        let response = engine.search(&query).expect("search");

        assert_eq!(response["found"], 1);
        assert_eq!(response["total_moments"], 2);
        assert_eq!(response["grouped_hits"][0]["found"], 2);
        assert_eq!(
            response["grouped_hits"][0]["hits"]
                .as_array()
                .expect("hits")
                .len(),
            1
        );
    }

    #[test]
    fn match_provenance_does_not_increase_the_hot_candidate_size() {
        assert_eq!(std::mem::size_of::<DocMatch>(), 8);
        // The extra four bytes are build-only spool coordinates. Query candidates stay compact.
        assert_eq!(std::mem::size_of::<BuildPosting>(), 12);
        let mut transcript = DocMatch::new(42, 1.0, true);
        assert_eq!(transcript.document(), 42);
        assert!(transcript.matches_text());
        assert_eq!(transcript.exact_tokens(), 1);
        transcript.set_exact_mask(0b1000);
        let mut fuzzy = DocMatch::new(42, 2.0, true);
        fuzzy.set_exact_mask(0);
        let union = union_sorted(vec![transcript], vec![fuzzy]);
        assert_eq!(union[0].exact_tokens(), 1);
        assert_eq!(union[0].exact_mask(), 0b1000);

        let mut later_exact = transcript;
        later_exact.set_exact_mask(0b0100);
        let two_exact = intersect_sorted(vec![transcript], vec![later_exact]);
        assert_eq!(two_exact[0].exact_tokens(), 2);
        assert_eq!(two_exact[0].exact_mask(), 0b1100);
        let one_exact = intersect_sorted(vec![transcript], vec![fuzzy]);
        assert_eq!(one_exact[0].exact_tokens(), 1);
    }

    #[test]
    fn sorted_exclusion_union_deduplicates_without_reordering() {
        assert_eq!(
            union_sorted_ids(vec![1, 3, 7], vec![2, 3, 8]),
            [1, 2, 3, 7, 8]
        );
        assert_eq!(union_sorted_ids(Vec::new(), vec![4, 9]), [4, 9]);
    }

    #[test]
    fn first_token_position_breaks_equal_relevance_ties() {
        let (_root, engine) = position_engine();
        let mut query = grouped_query("target");
        query.query_by = Some("text".to_string());
        let response = engine.search(&query).expect("position search");
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "early");
        assert!(token_position_multiplier(0, true) > token_position_multiplier(3, true));
        assert_eq!(token_position_multiplier(0, false), 1.0);
    }

    #[test]
    fn moment_totals_use_the_same_twenty_second_coalescing_window_as_the_api() {
        let mut moments = MomentCounter::default();
        for start in [1_000, 21_000, 21_001, 41_001] {
            moments.record(start);
        }
        assert_eq!(moments.visible(), 2);
    }

    #[test]
    fn title_only_matches_count_once_per_video() {
        let (_root, engine) = test_engine();
        let mut query = grouped_query("performance");
        query.group_limit = Some(1);
        let response = engine.search(&query).expect("search");

        assert_eq!(response["found"], 1);
        assert_eq!(response["total_moments"], 1);
        assert_eq!(response["grouped_hits"][0]["found"], 1);
    }

    #[test]
    fn filters_and_date_sort_match_the_shared_contract() {
        let (_root, engine) = test_engine();
        let mut query = grouped_query("*");
        query.filter_by = Some("youtuber_handle:=@TWO".to_string());
        query.sort_by = Some("published_at_timestamp:desc".to_string());
        let response = engine.search(&query).expect("search");
        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "b");
    }

    #[test]
    fn unsupported_search_contracts_fail_closed() {
        let (_root, engine) = test_engine();
        for (field, value, expected) in [
            ("query_by", "text,description", "unsupported query_by field"),
            ("query_by", "text,text", "duplicate field"),
            (
                "filter_by",
                "published_at:=2026",
                "unsupported filter_by field",
            ),
            ("group_by", "youtuber_handle", "unsupported group_by field"),
            (
                "sort_by",
                "published_at:desc",
                "unsupported sort_by expression",
            ),
        ] {
            let mut query = grouped_query("search");
            match field {
                "query_by" => query.query_by = Some(value.to_string()),
                "filter_by" => query.filter_by = Some(value.to_string()),
                "group_by" => query.group_by = Some(value.to_string()),
                "sort_by" => query.sort_by = Some(value.to_string()),
                _ => unreachable!(),
            }
            let error = engine.search(&query).expect_err("invalid request");
            assert!(error.to_string().contains(expected), "error={error}");
        }

        let mut malformed_filter = grouped_query("search");
        malformed_filter.filter_by = Some("video_id:abc".to_string());
        assert!(engine.search(&malformed_filter).is_err());
        for filter in [
            "video_id:=abc && youtuber_handle:=@one",
            "video_id:=[abc,def]",
            "video_id:=abc:=def",
            "video_id:=`abc",
            "video_id:=abc`",
        ] {
            malformed_filter.filter_by = Some(filter.to_string());
            assert!(
                engine.search(&malformed_filter).is_err(),
                "accepted {filter}"
            );
        }

        let mut quoted_filter = grouped_query("*");
        quoted_filter.filter_by = Some("video_id:=`a`".to_string());
        assert_eq!(
            engine.search(&quoted_filter).expect("quoted filter")["found"],
            1
        );

        let mut conflicting_window = grouped_query("search");
        conflicting_window.per_page = Some(10);
        assert!(engine.search(&conflicting_window).is_err());

        let mut empty = grouped_query("  ");
        empty.limit = Some(10);
        assert!(engine.search(&empty).is_err());
    }

    #[test]
    fn active_typesense_parameters_are_validated_and_supported() {
        let (_root, engine) = test_engine();
        let mut query = grouped_query("search");
        query.query_by_weights = Some("4,6".to_string());
        query.text_match_type = Some("sum_score".to_string());
        query.prioritize_exact_match = Some(true);
        query.prioritize_token_position = Some(true);
        query.typo_tokens_threshold = Some(5);
        query.drop_tokens_threshold = Some(1);
        query.drop_tokens_mode = Some("right_to_left".to_string());
        query.split_join_tokens = Some("always".to_string());
        query.highlight_fields = Some("text,title".to_string());
        query.include_fields = Some(ACTIVE_INCLUDE_FIELDS.to_string());
        assert_eq!(engine.search(&query).expect("active contract")["found"], 1);

        query.typo_tokens_threshold = Some(1);
        assert!(engine.search(&query).is_err());

        query.typo_tokens_threshold = Some(5);
        query.drop_tokens_threshold = Some(0);
        assert!(engine.search(&query).is_err());

        query.drop_tokens_threshold = Some(1);
        query.drop_tokens_mode = Some("left_to_right".to_string());
        assert!(engine.search(&query).is_err());
    }

    #[test]
    fn typo_search_is_a_guarded_fallback() {
        let (_root, engine) = test_engine();
        let response = engine.search(&grouped_query("invertd")).expect("search");
        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "a");
    }

    #[test]
    fn prefix_transposition_and_two_edit_search_share_the_end_to_end_contract() {
        let (_root, engine) = test_engine();
        for query in ["relat", "relaitvity", "relativtyy"] {
            let response = engine.search(&grouped_query(query)).expect("search");
            assert_eq!(response["found"], 1, "query={query}");
            assert_eq!(
                response["grouped_hits"][0]["group_key"][0], "a",
                "query={query}"
            );
            assert!(
                response["grouped_hits"][0]["hits"][0]["highlights"]
                    .as_array()
                    .expect("highlights")
                    .iter()
                    .any(|highlight| highlight["snippet"]
                        .as_str()
                        .is_some_and(|snippet| snippet.contains("<mark>"))),
                "query={query}"
            );
        }
    }

    #[test]
    fn multi_token_typo_and_last_token_prefix_preserve_strict_intersection() {
        let (_root, engine) = test_engine();
        for query in ["memory map", "memory maped"] {
            let response = engine.search(&grouped_query(query)).expect("search");
            assert_eq!(response["found"], 1, "query={query}");
            assert_eq!(
                response["grouped_hits"][0]["group_key"][0], "a",
                "query={query}"
            );
        }
    }

    #[test]
    fn token_drop_prefers_the_longest_right_to_left_conjunction() {
        let (_root, engine) = token_drop_engine();
        let response = engine
            .search(&grouped_query("alpha beta missing"))
            .expect("token-drop search");

        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "prefix-pair");
    }

    #[test]
    fn token_drop_switches_direction_only_after_prefixes_are_exhausted() {
        let (_root, engine) = token_drop_engine();
        let response = engine
            .search(&grouped_query("missing beta gamma"))
            .expect("token-drop search");

        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "suffix-pair");
    }

    #[test]
    fn token_drop_threshold_applies_the_active_filter() {
        let (_root, engine) = token_drop_engine();
        let mut query = grouped_query("alpha beta gamma");
        query.filter_by = Some("youtuber_handle:=@two".to_string());
        let response = engine.search(&query).expect("filtered token-drop search");

        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "suffix-pair");
    }

    #[test]
    fn typo_expansion_stays_conjunctive_inside_a_retained_subset() {
        let (_root, engine) = token_drop_engine();
        let response = engine
            .search(&grouped_query("alpha betta missing"))
            .expect("fuzzy token-drop search");

        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "prefix-pair");
    }

    #[test]
    fn fully_exact_matches_rank_before_higher_frequency_typo_matches() {
        let (_root, engine) = token_drop_engine();
        let response = engine
            .search(&grouped_query("relativity explained"))
            .expect("exact-priority search");

        assert_eq!(response["found"], 2);
        assert_eq!(
            response["grouped_hits"][0]["group_key"][0],
            "exact-relativity"
        );
    }

    #[test]
    fn earlier_exact_query_tokens_break_equal_exact_count_ties() {
        let (_root, engine) = token_drop_engine();
        let response = engine
            .search(&grouped_query("planet orbit missing"))
            .expect("query-order exact-priority search");

        assert_eq!(response["found"], 2);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "early-exact");
    }

    #[test]
    fn blackhole_split_is_consistent_globally_and_per_creator() {
        let (_root, engine) = blackhole_engine();
        let global = engine.search(&grouped_query("blackhole")).expect("global");
        assert_eq!(global["found"], 3);
        let global_ids = global["grouped_hits"]
            .as_array()
            .expect("groups")
            .iter()
            .map(|group| group["group_key"][0].as_str().expect("video id"))
            .collect::<HashSet<_>>();
        assert_eq!(
            global_ids,
            HashSet::from(["literal", "veritasium", "vsauce"])
        );
        for (handle, expected) in [("@VERITASIUM", "veritasium"), ("@vsauce", "vsauce")] {
            let mut query = grouped_query("blackhole");
            query.filter_by = Some(format!("youtuber_handle:={handle}"));
            let response = engine.search(&query).expect("creator search");
            assert_eq!(response["found"], 1);
            assert_eq!(response["grouped_hits"][0]["group_key"][0], expected);
            assert!(
                response["grouped_hits"][0]["hits"][0]["highlights"][0]["snippet"]
                    .as_str()
                    .expect("split highlight")
                    .contains("<mark>")
            );
        }
    }

    #[test]
    fn typo_corrected_compact_split_recovers_black_hole_results() {
        let (_root, engine) = blackhole_engine();
        for query in ["blackhloe", "blakhole", "blackhle"] {
            let response = engine.search(&grouped_query(query)).expect("search");
            let ids = response["grouped_hits"]
                .as_array()
                .expect("groups")
                .iter()
                .map(|group| group["group_key"][0].as_str().expect("video id"))
                .collect::<HashSet<_>>();
            assert!(ids.contains("veritasium"), "query={query}");
            assert!(ids.contains("vsauce"), "query={query}");
        }
    }

    #[test]
    fn typo_corrected_split_preserves_corrected_query_ranking() {
        let (_root, engine) = weighted_blackhole_engine();
        let exact = engine.search(&grouped_query("blackhole")).expect("exact");
        let expected = exact["grouped_hits"][0]["group_key"][0]
            .as_str()
            .expect("exact top result");
        for query in ["blackhloe", "blakhole", "blackhle"] {
            let response = engine.search(&grouped_query(query)).expect("search");
            assert_eq!(
                response["grouped_hits"][0]["group_key"][0], expected,
                "query={query}"
            );
        }
    }

    #[test]
    fn relevance_and_explicit_date_sorts_are_distinct_and_deterministic() {
        let (_root, engine) = blackhole_engine();
        let relevant = engine
            .search(&grouped_query("blackhole"))
            .expect("relevance");
        assert_eq!(relevant["grouped_hits"][0]["group_key"][0], "veritasium");

        let mut newest = grouped_query("blackhole");
        newest.sort_by = Some("published_at_timestamp:desc".to_string());
        let newest = engine.search(&newest).expect("newest");
        assert_eq!(newest["grouped_hits"][0]["group_key"][0], "literal");

        let mut oldest = grouped_query("blackhole");
        oldest.sort_by = Some("published_at_timestamp:asc".to_string());
        let oldest = engine.search(&oldest).expect("oldest");
        assert_eq!(oldest["grouped_hits"][0]["group_key"][0], "veritasium");
    }

    #[test]
    fn engagement_sorts_are_explicit_bounded_and_return_the_source_counts() {
        let (_root, engine) = engagement_engine();

        for sort in [
            "engagement_score:desc",
            "view_count:desc",
            "like_count:desc",
            "comment_count:desc",
        ] {
            let mut query = grouped_query("systems explained");
            query.sort_by = Some(sort.to_string());
            let response = engine.search(&query).expect("engagement search");
            assert_eq!(response["grouped_hits"][0]["group_key"][0], "large");
            assert_eq!(
                response["grouped_hits"][0]["hits"][0]["document"]["view_count"],
                1_000_000
            );
            assert_eq!(
                response["grouped_hits"][0]["hits"][0]["document"]["like_count"],
                100_000
            );
            assert_eq!(
                response["grouped_hits"][0]["hits"][0]["document"]["comment_count"],
                10_000
            );
        }

        let relevant = engine
            .search(&grouped_query("orbital mechanics"))
            .expect("relevance search");
        assert_eq!(relevant["grouped_hits"][0]["group_key"][0], "exact");
    }

    #[test]
    fn engagement_normalization_is_bounded_and_handles_missing_metrics() {
        let manifest = Manifest {
            schema_version: 5,
            documents: 0,
            videos: 0,
            channels: 0,
            terms: 0,
            blocks: 0,
            average_document_tokens: 0.0,
            max_view_count: 1_000,
            max_like_count: 100,
            max_comment_count: 10,
            files: BTreeMap::new(),
        };
        let video = VideoMeta {
            id: "video".to_string(),
            handle: "@creator".to_string(),
            title: "Video".to_string(),
            published_at_timestamp: 0,
            view_count: 1_000,
            like_count: 100,
            comment_count: 10,
            documents: Vec::new(),
        };
        assert!((normalized_engagement(&video, &manifest) - 1.0).abs() < f32::EPSILON);
        assert_eq!(normalized_log(0, 0), 0.0);
        assert_eq!(blended_relevance(10.0, &video, &manifest), 12.0);
    }

    #[test]
    fn pagination_and_query_fields_preserve_the_search_contract() {
        let (_root, engine) = blackhole_engine();
        let mut first = grouped_query("blackhole");
        first.limit = None;
        first.offset = None;
        first.per_page = Some(1);
        first.page = Some(1);
        let mut second = first.clone();
        second.page = Some(2);
        let first = engine.search(&first).expect("first page");
        let second = engine.search(&second).expect("second page");
        assert_eq!(first["found"], 3);
        assert_eq!(second["found"], 3);
        assert_eq!(first["total_moments"], 3);
        assert_eq!(second["total_moments"], 3);
        assert_ne!(
            first["grouped_hits"][0]["group_key"][0],
            second["grouped_hits"][0]["group_key"][0]
        );

        let mut title_only = grouped_query("blackhole");
        title_only.query_by = Some("title".to_string());
        let title_only = engine.search(&title_only).expect("title only");
        assert_eq!(title_only["found"], 2);

        let mut ungrouped = grouped_query("blackhole");
        ungrouped.group_by = None;
        let ungrouped = engine.search(&ungrouped).expect("ungrouped");
        assert_eq!(ungrouped["found"], 3);
        assert_eq!(ungrouped["hits"].as_array().expect("hits").len(), 3);
    }

    #[test]
    fn wildcard_video_filter_group_limit_and_channel_search_are_supported() {
        let (_root, engine) = blackhole_engine();
        let mut wildcard = grouped_query("*");
        wildcard.filter_by = Some("video_id:=vsauce".to_string());
        wildcard.group_limit = Some(1);
        let wildcard = engine.search(&wildcard).expect("wildcard");
        assert_eq!(wildcard["found"], 1);
        assert_eq!(wildcard["grouped_hits"][0]["group_key"][0], "vsauce");
        assert_eq!(
            wildcard["grouped_hits"][0]["hits"]
                .as_array()
                .expect("hits")
                .len(),
            1
        );

        let channels = engine
            .search_channels(&SearchParams {
                q: "sauce".to_string(),
                query_by: Some("name,handle".to_string()),
                filter_by: None,
                group_by: None,
                group_limit: None,
                per_page: Some(1),
                page: Some(1),
                offset: None,
                limit: None,
                sort_by: None,
                use_cache: None,
                cache_ttl: None,
                ..SearchParams::default()
            })
            .expect("channel search");
        assert_eq!(channels["found"], 1);
        assert_eq!(channels["hits"][0]["document"]["handle"], "@vsauce");

        let mut invalid_channel = SearchParams {
            q: "sauce".to_string(),
            query_by: Some("text,title".to_string()),
            ..SearchParams::default()
        };
        assert!(engine.search_channels(&invalid_channel).is_err());
        invalid_channel.query_by = Some("name,handle".to_string());
        invalid_channel.sort_by = Some("name:asc".to_string());
        assert!(engine.search_channels(&invalid_channel).is_err());
    }

    #[test]
    fn empty_corpus_builds_and_searches_without_special_cases() {
        let root = TempDir::new().expect("temp dir");
        let chunks = root.path().join("chunks.jsonl");
        let channels = root.path().join("channels.jsonl");
        File::create(&chunks).expect("chunks");
        File::create(&channels).expect("channels");
        let index = root.path().join("index");
        let report = build_index(&chunks, &channels, &index).expect("build empty index");
        assert_eq!(report.documents, 0);
        assert_eq!(report.videos, 0);
        let engine = RawSearchEngine::open(&index).expect("open empty index");
        assert_eq!(
            engine
                .search(&grouped_query("blackhole"))
                .expect("empty search")["found"],
            0
        );
        assert_eq!(
            engine.search(&grouped_query("*")).expect("empty wildcard")["found"],
            0
        );
    }

    #[test]
    fn cloned_engines_return_stable_results_under_concurrency() {
        let (_root, engine) = blackhole_engine();
        let expected = engine
            .search(&grouped_query("blackhole"))
            .expect("expected");
        let workers = (0..8)
            .map(|_| {
                let engine = engine.clone();
                let expected = expected.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        assert_eq!(
                            engine.search(&grouped_query("blackhole")).expect("search"),
                            expected
                        );
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().expect("worker");
        }
    }

    #[test]
    fn non_contiguous_video_input_preserves_group_membership() {
        let root = TempDir::new().expect("temp dir");
        let chunks_path = root.path().join("chunks.jsonl");
        let channels_path = root.path().join("channels.jsonl");
        let mut chunks = File::create(&chunks_path).expect("chunks");
        for video in ["a", "b", "a"] {
            writeln!(
                chunks,
                "{}",
                json!({"video_id":video,"title":"t","text":"x","youtuber_handle":"@one"})
            )
            .expect("write");
        }
        File::create(&channels_path).expect("channels");
        let index_path = root.path().join("index");
        build_index(&chunks_path, &channels_path, &index_path).expect("build");
        let engine = RawSearchEngine::open(&index_path).expect("open");
        let response = engine.search(&grouped_query("x")).expect("search");
        assert_eq!(response["found"], 2);
        let a_group = response["grouped_hits"]
            .as_array()
            .expect("groups")
            .iter()
            .find(|group| group["group_key"][0] == "a")
            .expect("a group");
        assert_eq!(a_group["hits"].as_array().expect("hits").len(), 2);
    }

    #[test]
    fn typo_distance_uses_typesense_length_gates() {
        assert_eq!(bounded_typo_distance("abc"), 0);
        assert_eq!(bounded_typo_distance("abcd"), 1);
        assert_eq!(bounded_typo_distance("abcdef"), 1);
        assert_eq!(bounded_typo_distance("abcdefg"), 2);
    }

    #[test]
    fn corrupt_file_magic_is_rejected_before_search() {
        let (root, engine) = test_engine();
        drop(engine);
        let mut lexicon = OpenOptions::new()
            .write(true)
            .open(root.path().join("index").join(LEXICON_FILE))
            .expect("lexicon");
        lexicon.write_all(b"BROKEN!!").expect("corrupt magic");
        let error = match RawSearchEngine::open(&root.path().join("index")) {
            Ok(_) => panic!("corrupt index must fail"),
            Err(error) => error,
        };
        assert!(matches!(error, Error::Corrupt(_)));
    }

    #[test]
    fn truncated_fuzzy_index_is_rejected_at_open() {
        let (root, engine) = test_engine();
        drop(engine);
        let fuzzy_path = root.path().join("index").join(crate::fuzzy::FUZZY_FILE);
        OpenOptions::new()
            .write(true)
            .open(&fuzzy_path)
            .expect("fuzzy index")
            .set_len((HEADER_BYTES + 1) as u64)
            .expect("truncate fuzzy index");
        assert!(matches!(
            RawSearchEngine::open(&root.path().join("index")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn truncated_postings_are_rejected_at_open() {
        let (root, engine) = test_engine();
        drop(engine);
        let postings_path = root.path().join("index").join(POSTINGS_FILE);
        OpenOptions::new()
            .write(true)
            .open(&postings_path)
            .expect("postings")
            .set_len(HEADER_BYTES as u64)
            .expect("truncate postings");
        assert!(matches!(
            RawSearchEngine::open(&root.path().join("index")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn truncated_positions_are_rejected_at_open() {
        let (root, engine) = test_engine();
        drop(engine);
        let positions_path = root.path().join("index").join(POSITIONS_FILE);
        OpenOptions::new()
            .write(true)
            .open(&positions_path)
            .expect("positions")
            .set_len(HEADER_BYTES as u64)
            .expect("truncate positions");
        assert!(matches!(
            RawSearchEngine::open(&root.path().join("index")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn truncated_full_positions_are_rejected_at_open() {
        let (root, engine) = test_engine();
        drop(engine);
        let positions_path = root.path().join("index").join(FULL_POSITIONS_FILE);
        OpenOptions::new()
            .write(true)
            .open(&positions_path)
            .expect("full positions")
            .set_len(HEADER_BYTES as u64)
            .expect("truncate full positions");
        assert!(matches!(
            RawSearchEngine::open(&root.path().join("index")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn same_length_generation_corruption_is_rejected_at_open() {
        let (root, engine) = test_engine();
        drop(engine);
        let postings_path = root.path().join("index").join(POSTINGS_FILE);
        let mut postings = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&postings_path)
            .expect("postings");
        postings
            .seek(SeekFrom::Start(HEADER_BYTES as u64))
            .expect("seek posting");
        let mut byte = [0_u8; 1];
        postings.read_exact(&mut byte).expect("read posting");
        byte[0] ^= 0x01;
        postings
            .seek(SeekFrom::Start(HEADER_BYTES as u64))
            .expect("seek posting again");
        postings.write_all(&byte).expect("corrupt posting");
        postings.flush().expect("flush corruption");
        assert!(matches!(
            RawSearchEngine::open(&root.path().join("index")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn truncated_document_blocks_are_rejected_at_open() {
        let (root, engine) = test_engine();
        drop(engine);
        let documents_path = root.path().join("index").join(DOCUMENTS_FILE);
        OpenOptions::new()
            .write(true)
            .open(&documents_path)
            .expect("documents")
            .set_len(HEADER_BYTES as u64)
            .expect("truncate documents");
        assert!(matches!(
            RawSearchEngine::open(&root.path().join("index")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn schema_five_reader_is_a_response_compatible_rollback() {
        let (root, engine) = test_engine();
        let expected = ["memory", "bun", "*"]
            .into_iter()
            .map(|query| {
                (
                    query,
                    engine
                        .search(&grouped_query(query))
                        .expect("schema 6 search"),
                )
            })
            .collect::<Vec<_>>();
        drop(engine);

        let index_path = root.path().join("index");
        let manifest_path = index_path.join(MANIFEST_FILE);
        let mut manifest: Value =
            serde_json::from_reader(File::open(&manifest_path).expect("manifest"))
                .expect("manifest JSON");
        manifest["schema_version"] = json!(5);
        manifest
            .as_object_mut()
            .expect("manifest object")
            .remove("files");
        serde_json::to_writer_pretty(
            File::create(&manifest_path).expect("rewrite manifest"),
            &manifest,
        )
        .expect("write schema 5 manifest");
        fs::remove_file(index_path.join(FULL_POSITIONS_FILE)).expect("remove v6 sidecar");

        let rollback = RawSearchEngine::open(&index_path).expect("open schema 5 rollback");
        for (query, expected) in expected {
            assert_eq!(
                rollback
                    .search(&grouped_query(query))
                    .expect("schema 5 search"),
                expected,
                "schema 5 response changed for {query}"
            );
        }
        assert!(matches!(
            rollback.search(&grouped_query(r#""memory mapped""#)),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            rollback.search(&grouped_query(r#"bun -"memory mapped""#)),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(
            rollback
                .search(&grouped_query(r#"bun -"memory""#))
                .expect("single-token negative quote"),
            rollback
                .search(&grouped_query("bun -memory"))
                .expect("negative term")
        );
    }

    #[test]
    fn manifest_channel_count_mismatch_is_rejected_at_open() {
        let (root, engine) = test_engine();
        drop(engine);
        let manifest_path = root.path().join("index").join(MANIFEST_FILE);
        let mut manifest: Value =
            serde_json::from_reader(File::open(&manifest_path).expect("manifest"))
                .expect("manifest JSON");
        manifest["channels"] = json!(99);
        serde_json::to_writer(
            File::create(&manifest_path).expect("rewrite manifest"),
            &manifest,
        )
        .expect("write manifest");
        assert!(matches!(
            RawSearchEngine::open(&root.path().join("index")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn completed_index_is_never_overwritten() {
        let (root, engine) = test_engine();
        drop(engine);
        let error = build_index(
            &root.path().join("chunks.jsonl"),
            &root.path().join("channels.jsonl"),
            &root.path().join("index"),
        )
        .expect_err("completed index must not be overwritten");
        assert!(matches!(error, Error::ExistingIndex(_)));
    }

    #[test]
    fn stable_slice_is_nested_and_input_order_independent() {
        let root = TempDir::new().expect("temp dir");
        let input = root.path().join("input.jsonl");
        let small = root.path().join("small.jsonl");
        let large = root.path().join("large.jsonl");
        let mut source = File::create(&input).expect("input");
        for video in ["c", "a", "d", "b", "a"] {
            writeln!(
                source,
                "{}",
                json!({"video_id":video,"title":"t","text":"x","youtuber_handle":"@one"})
            )
            .expect("write");
        }
        let small_report = slice_corpus(&input, &small, 2).expect("small slice");
        let large_report = slice_corpus(&input, &large, 3).expect("large slice");
        assert_eq!(small_report.videos, 2);
        assert_eq!(large_report.videos, 3);
        let small_ids = read_video_ids(&small);
        let large_ids = read_video_ids(&large);
        assert!(small_ids.is_subset(&large_ids));
    }

    fn read_video_ids(path: &Path) -> HashSet<String> {
        BufReader::new(File::open(path).expect("slice"))
            .lines()
            .map(|line| {
                serde_json::from_str::<IndexDocument>(&line.expect("line"))
                    .expect("document")
                    .video_id
            })
            .collect()
    }
}
