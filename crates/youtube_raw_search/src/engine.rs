use crate::{
    Error, Result,
    format::{
        BLOCK_RECORD_BYTES, DOCUMENT_RECORD_BYTES, DOCUMENTS_MAGIC, HEADER_BYTES, LEXICON_MAGIC,
        METADATA_MAGIC, MappedFile, POSTINGS_MAGIC, read_u16, read_u32, read_u64, read_varint,
        write_header, write_varint,
    },
    fuzzy::{FuzzyCandidate, FuzzyIndex, write_index as write_fuzzy_index},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Seek, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use youtube_search_service::{SearchBackend, SearchParams};
use youtube_time::parse_rfc3339_timestamp;

const MANIFEST_FILE: &str = "manifest.json";
const CHANNELS_FILE: &str = "channels.json";
const VIDEOS_FILE: &str = "videos.json";
const LEXICON_FILE: &str = "lexicon.bin";
const POSTINGS_FILE: &str = "postings.bin";
const DOCUMENTS_FILE: &str = "documents.blocks";
const METADATA_FILE: &str = "documents.meta";
const DOCUMENTS_PER_BLOCK: usize = 64;
const METADATA_PREFIX_BYTES: usize = HEADER_BYTES + 8;
const TYPO_RESULT_THRESHOLD: usize = 5;
const MAX_FUZZY_TERMS_PER_DISTANCE: usize = 4;
const MAX_FUZZY_SPLIT_TERMS: usize = 8;
const MIN_ONE_TYPO_BYTES: usize = 4;
const MIN_TWO_TYPO_BYTES: usize = 7;
const MIN_SPLIT_TERM_CHARS: usize = 2;
const MIN_FUZZY_SPLIT_COMPONENT_CHARS: usize = 3;
const MAX_SPLIT_ALTERNATIVES: usize = 8;
const SPLIT_SCORE_BONUS: f32 = 8.0;

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
}

#[derive(Clone, Deserialize, Serialize)]
struct VideoMeta {
    id: String,
    handle: String,
    title: String,
    published_at_timestamp: i64,
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
}

#[derive(Default)]
struct TermBuild {
    text: Vec<BuildPosting>,
    title: Vec<BuildPosting>,
}

#[derive(Clone, Copy)]
struct BuildPosting {
    id: u32,
    frequency: u16,
}

#[derive(Clone, Copy)]
struct DocumentRecord {
    block: u32,
    offset: u32,
    length: u32,
    video: u32,
    tokens: u32,
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
    text_offset: usize,
    text_length: usize,
    text_frequency: u32,
    title_offset: usize,
    title_length: usize,
    title_frequency: u32,
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
}

struct VideoMatches {
    video: u32,
    score: f32,
    documents: Vec<DocMatch>,
}

struct SplitComponent {
    matches: Vec<DocMatch>,
    highlight: String,
    fuzzy: bool,
}

#[derive(Clone, Copy)]
enum QueryMode {
    Strict,
    LeadingPhrase,
    Exact,
    Fuzzy,
}

#[derive(Clone, Copy)]
struct QueryFields {
    text: bool,
    title: bool,
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
            *video
        } else {
            let video = videos.len() as u32;
            let title_counts = token_counts(&source.title);
            for (term, frequency) in title_counts {
                terms.entry(term).or_default().title.push(BuildPosting {
                    id: video,
                    frequency,
                });
            }
            videos.push(VideoMeta {
                id: source.video_id.clone(),
                handle: source.youtuber_handle.to_ascii_lowercase(),
                title: source.title,
                published_at_timestamp: published_at_timestamp.unwrap_or_default(),
                documents: Vec::new(),
            });
            video_ids.insert(source.video_id, video);
            video
        };

        let document = document_records.len() as u32;
        let text_counts = token_counts(&source.text);
        let document_tokens = text_counts
            .values()
            .map(|frequency| u32::from(*frequency))
            .sum::<u32>();
        total_tokens = total_tokens.saturating_add(u64::from(document_tokens));
        for (term, frequency) in text_counts {
            terms.entry(term).or_default().text.push(BuildPosting {
                id: document,
                frequency,
            });
        }

        let line_bytes = line.as_bytes();
        let offset = block_bytes.len() as u32;
        block_bytes.extend_from_slice(line_bytes);
        document_records.push(DocumentRecord {
            block: block_records.len() as u32,
            offset,
            length: line_bytes.len() as u32,
            video,
            tokens: document_tokens,
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
    let term_count = write_postings(index_path, terms, &videos)?;

    let document_count = document_records.len() as u32;
    let average_document_tokens = if document_count == 0 {
        0.0
    } else {
        total_tokens as f32 / document_count as f32
    };
    let manifest = Manifest {
        schema_version: 2,
        documents: document_count,
        videos: videos.len() as u32,
        channels: channels.len() as u32,
        terms: term_count as u32,
        blocks: block_records.len() as u32,
        average_document_tokens,
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
) -> Result<usize> {
    let mut terms = terms.into_iter().collect::<Vec<_>>();
    terms.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let mut postings = BufWriter::new(File::create(index_path.join(POSTINGS_FILE))?);
    let mut header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut header, POSTINGS_MAGIC, terms.len() as u32);
    postings.write_all(&header)?;
    let mut position = HEADER_BYTES as u64;
    let mut lexicon = Vec::with_capacity(terms.len());

    for (term, lists) in terms {
        let text = encode_postings(&lists.text);
        let text_offset = position;
        postings.write_all(&text)?;
        position += text.len() as u64;
        let title = encode_postings(&lists.title);
        let fuzzy_title_frequency = lists.title.iter().fold(0_u32, |frequency, posting| {
            frequency.saturating_add(
                videos
                    .get(posting.id as usize)
                    .map_or(0, |video| video.documents.len() as u32),
            )
        });
        let title_offset = position;
        postings.write_all(&title)?;
        position += title.len() as u64;
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
    postings.flush()?;
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
        if manifest.schema_version != 2 {
            return Err(Error::Corrupt(format!(
                "unsupported schema version {}",
                manifest.schema_version
            )));
        }
        let videos: Vec<VideoMeta> =
            serde_json::from_reader(BufReader::new(File::open(index_path.join(VIDEOS_FILE))?))?;
        let channels: Vec<Value> =
            serde_json::from_reader(BufReader::new(File::open(index_path.join(CHANNELS_FILE))?))?;
        let lexicon = Lexicon::open(&index_path.join(LEXICON_FILE))?;
        let fuzzy = FuzzyIndex::open(index_path, lexicon.count)?;
        let postings = MappedFile::open(&index_path.join(POSTINGS_FILE), POSTINGS_MAGIC)?;
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

    fn search(&self, params: &SearchParams) -> std::result::Result<Value, String> {
        RawSearchEngine::search(self, params).map_err(|error| error.to_string())
    }

    fn search_channels(&self, params: &SearchParams) -> Value {
        RawSearchEngine::search_channels(self, params)
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
        let terms = unique_tokens(&params.q);
        if params.q.trim() == "*" || terms.is_empty() {
            return self.search_all(params);
        }
        let fields = query_fields(params.query_by.as_deref());

        if terms.len() == 1 {
            return self.search_single_token(params, &terms[0], fields);
        }

        if terms.len() > 1 {
            let strict = self.search_mode(params, &terms, fields, QueryMode::Strict)?;
            if response_found(&strict) as usize >= TYPO_RESULT_THRESHOLD {
                return Ok(strict);
            }
        }
        if terms.len() > 2 {
            let leading =
                self.search_mode(params, &terms[..2], fields, QueryMode::LeadingPhrase)?;
            if response_found(&leading) >= 3 {
                return Ok(leading);
            }
        }
        let exact = self.search_mode(params, &terms, fields, QueryMode::Exact)?;
        let query_is_short = params.q.trim().len() < 5;
        if query_is_short || response_found(&exact) as usize >= TYPO_RESULT_THRESHOLD {
            return Ok(exact);
        }
        self.search_mode(params, &terms, fields, QueryMode::Fuzzy)
    }

    fn search_single_token(
        &self,
        params: &SearchParams,
        term: &str,
        fields: QueryFields,
    ) -> Result<Value> {
        let exact = self.matches_for_term(term, fields)?;
        let mut cache = BlockCache::default();
        let (split, split_terms) = self.split_term_matches(term, fields)?;
        let has_exact_split = !split.is_empty();
        let mut matches = union_sorted(exact, split);
        let mut highlight_terms = Vec::with_capacity(1 + split_terms.len());
        highlight_terms.push(term.to_string());
        highlight_terms.extend(split_terms);
        if self.reaches_result_threshold(params, &matches, TYPO_RESULT_THRESHOLD)? {
            return self.format_matches(params, &highlight_terms, matches, &mut cache);
        }

        let (prefix, prefix_terms) = self.fuzzy_alternatives(term, fields, 0, None, true)?;
        matches = union_sorted(matches, prefix);
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
        let filter = parse_filter(params.filter_by.as_deref());
        if params.group_by.as_deref() == Some("video_id") {
            let mut videos = HashSet::with_capacity(threshold);
            for candidate in matches {
                let record = self.inner.metadata.document(candidate.document)?;
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
            let record = self.inner.metadata.document(candidate.document)?;
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
            let matches = self.matches_for_split_component(candidate_term, fields)?;
            output.push(SplitComponent {
                matches,
                highlight: candidate_term.to_string(),
                fuzzy: true,
            });
        }
        Ok(output)
    }

    fn search_mode(
        &self,
        params: &SearchParams,
        terms: &[String],
        fields: QueryFields,
        mode: QueryMode,
    ) -> Result<Value> {
        let mut term_matches = Vec::with_capacity(terms.len());
        let mut highlight_terms = terms.to_vec();
        for (index, term) in terms.iter().enumerate() {
            let mut matches = self.matches_for_term(term, fields)?;
            if index + 1 == terms.len()
                && !matches!(mode, QueryMode::Strict | QueryMode::LeadingPhrase)
            {
                let (prefix, prefix_terms) =
                    self.fuzzy_alternatives(term, fields, 0, None, true)?;
                matches = union_sorted(matches, prefix);
                highlight_terms.extend(prefix_terms);
            }
            if matches!(mode, QueryMode::Fuzzy) {
                for distance in 1..=bounded_typo_distance(term) {
                    let (fuzzy, fuzzy_terms) =
                        self.fuzzy_alternatives(term, fields, distance, Some(distance), false)?;
                    matches = union_sorted(matches, fuzzy);
                    highlight_terms.extend(fuzzy_terms);
                }
            }
            term_matches.push(matches);
        }
        let require_every_term = matches!(mode, QueryMode::Strict | QueryMode::LeadingPhrase);
        let mut matches = if require_every_term {
            intersect_all(term_matches)
        } else {
            union_all(term_matches)
        };
        let mut cache = BlockCache::default();
        if matches!(mode, QueryMode::LeadingPhrase) {
            let phrase = terms.join(" ");
            self.retain_phrase_matches(&mut matches, &phrase, &mut cache)?;
        }
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
            self.decode_text_postings(entry, idf)?
        } else {
            Vec::new()
        };
        let title = if fields.title {
            self.decode_title_postings(entry, idf)?
        } else {
            Vec::new()
        };
        Ok(union_sorted(text, title))
    }

    fn decode_text_postings(&self, entry: TermEntry, idf: f32) -> Result<Vec<DocMatch>> {
        let bytes = self.postings_range(entry.text_offset, entry.text_length)?;
        let mut matches = Vec::with_capacity(entry.text_frequency as usize);
        let mut cursor = 0_usize;
        let mut document = 0_u32;
        while cursor < bytes.len() {
            document = document.saturating_add(read_varint(bytes, &mut cursor, bytes.len())?);
            let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
            let record = self.inner.metadata.document(document)?;
            let score = bm25(
                frequency,
                record.tokens,
                self.inner.manifest.average_document_tokens,
            ) * idf;
            matches.push(DocMatch { document, score });
        }
        if matches.len() != entry.text_frequency as usize {
            return Err(Error::Corrupt(
                "text posting count does not match lexicon".to_string(),
            ));
        }
        Ok(matches)
    }

    fn decode_title_postings(&self, entry: TermEntry, idf: f32) -> Result<Vec<DocMatch>> {
        let bytes = self.postings_range(entry.title_offset, entry.title_length)?;
        let average_docs = self
            .inner
            .manifest
            .documents
            .checked_div(self.inner.manifest.videos)
            .unwrap_or(0);
        let mut matches =
            Vec::with_capacity((entry.title_frequency.saturating_mul(average_docs)) as usize);
        let mut cursor = 0_usize;
        let mut video_id = 0_u32;
        let mut postings = 0_u32;
        while cursor < bytes.len() {
            video_id = video_id.saturating_add(read_varint(bytes, &mut cursor, bytes.len())?);
            let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
            let video = self.inner.videos.get(video_id as usize).ok_or_else(|| {
                Error::Corrupt("title posting video is out of bounds".to_string())
            })?;
            let score = idf * 6.0 * (1.0 + (frequency as f32).ln());
            for &document in &video.documents {
                matches.push(DocMatch { document, score });
            }
            postings += 1;
        }
        if postings != entry.title_frequency {
            return Err(Error::Corrupt(
                "title posting count does not match lexicon".to_string(),
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
            let mut matches = self.matches_for_entry(entry, fields)?;
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
            let document = self.document_value(candidate.document, cache)?;
            let title_matches = document
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|value| value.to_ascii_lowercase().contains(&phrase));
            let text_matches = document
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|value| value.to_ascii_lowercase().contains(&phrase));
            if title_matches || text_matches {
                retained.push(DocMatch {
                    score: candidate.score + 8.0,
                    ..candidate
                });
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
        let filter = parse_filter(params.filter_by.as_deref());
        let mut groups = HashMap::<u32, VideoMatches>::new();
        for candidate in matches {
            let record = self.inner.metadata.document(candidate.document)?;
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
                documents: Vec::with_capacity(hits_per_group),
            });
            group.score = group.score.max(candidate.score);
            retain_best_document(&mut group.documents, candidate, hits_per_group);
        }

        let found = groups.len();
        let mut groups = groups.into_values().collect::<Vec<_>>();
        self.sort_groups(&mut groups, params.sort_by.as_deref());
        let (offset, limit) = result_window(params);
        let mut grouped_hits = Vec::with_capacity(limit.min(groups.len()));
        for mut group in groups.into_iter().skip(offset).take(limit) {
            group.documents.sort_unstable_by(descending_doc_score);
            let video_id = self.inner.videos[group.video as usize].id.clone();
            let mut hits = Vec::with_capacity(group.documents.len());
            for document in group.documents {
                hits.push(self.hit_value(document.document, terms, cache)?);
            }
            grouped_hits.push(json!({
                "group_key": [video_id],
                "found": hits.len(),
                "hits": hits,
            }));
        }
        Ok(json!({
            "found": found,
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
        let filter = parse_filter(params.filter_by.as_deref());
        let mut filtered = Vec::with_capacity(matches.len());
        for candidate in matches {
            let record = self.inner.metadata.document(candidate.document)?;
            let video = self
                .inner
                .videos
                .get(record.video as usize)
                .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?;
            if video_matches_filter(video, filter) {
                filtered.push(candidate);
            }
        }
        let mut matches = filtered;
        match params.sort_by.as_deref() {
            Some("published_at_timestamp:desc") => matches.sort_unstable_by(|left, right| {
                self.document_published(right.document)
                    .cmp(&self.document_published(left.document))
            }),
            Some("published_at_timestamp:asc") => matches.sort_unstable_by(|left, right| {
                self.document_published(left.document)
                    .cmp(&self.document_published(right.document))
            }),
            _ => matches.sort_unstable_by(descending_doc_score),
        }
        let found = matches.len();
        let (offset, limit) = result_window(params);
        let mut hits = Vec::with_capacity(limit.min(matches.len()));
        for candidate in matches.into_iter().skip(offset).take(limit) {
            hits.push(self.hit_value(candidate.document, terms, cache)?);
        }
        Ok(json!({
            "found": found,
            "out_of": self.inner.manifest.documents,
            "hits": hits,
        }))
    }

    fn sort_groups(&self, groups: &mut [VideoMatches], sort_by: Option<&str>) {
        groups.sort_unstable_by(|left, right| {
            let left_video = &self.inner.videos[left.video as usize];
            let right_video = &self.inner.videos[right.video as usize];
            match sort_by {
                Some("published_at_timestamp:desc") => right_video
                    .published_at_timestamp
                    .cmp(&left_video.published_at_timestamp)
                    .then_with(|| left_video.id.cmp(&right_video.id)),
                Some("published_at_timestamp:asc") => left_video
                    .published_at_timestamp
                    .cmp(&right_video.published_at_timestamp)
                    .then_with(|| left_video.id.cmp(&right_video.id)),
                _ => right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left_video.id.cmp(&right_video.id)),
            }
        });
    }

    fn search_all(&self, params: &SearchParams) -> Result<Value> {
        let filter = parse_filter(params.filter_by.as_deref());
        let (offset, limit) = result_window(params);
        let mut cache = BlockCache::default();
        if params.group_by.as_deref() == Some("video_id") {
            let mut videos = self
                .inner
                .videos
                .iter()
                .enumerate()
                .filter(|(_, video)| video_matches_filter(video, filter))
                .map(|(index, _)| index as u32)
                .collect::<Vec<_>>();
            videos.sort_unstable_by(|left, right| {
                let left = &self.inner.videos[*left as usize];
                let right = &self.inner.videos[*right as usize];
                match params.sort_by.as_deref() {
                    Some("published_at_timestamp:asc") => left
                        .published_at_timestamp
                        .cmp(&right.published_at_timestamp)
                        .then_with(|| left.id.cmp(&right.id)),
                    _ => right
                        .published_at_timestamp
                        .cmp(&left.published_at_timestamp)
                        .then_with(|| left.id.cmp(&right.id)),
                }
            });
            let found = videos.len();
            let hits_per_group = usize::from(params.group_limit.unwrap_or(5).max(1));
            let mut grouped_hits = Vec::with_capacity(limit.min(videos.len()));
            for video_id in videos.into_iter().skip(offset).take(limit) {
                let video = &self.inner.videos[video_id as usize];
                let mut hits = Vec::with_capacity(hits_per_group);
                for &document in video.documents.iter().take(hits_per_group) {
                    hits.push(self.hit_value(document, &[], &mut cache)?);
                }
                grouped_hits.push(json!({
                    "group_key": [video.id],
                    "found": video.documents.len(),
                    "hits": hits,
                }));
            }
            return Ok(json!({
                "found": found,
                "out_of": self.inner.manifest.documents,
                "grouped_hits": grouped_hits,
            }));
        }

        let mut documents = (0..self.inner.manifest.documents)
            .filter(|document| {
                self.inner
                    .metadata
                    .document(*document)
                    .ok()
                    .and_then(|record| self.inner.videos.get(record.video as usize))
                    .is_some_and(|video| video_matches_filter(video, filter))
            })
            .collect::<Vec<_>>();
        match params.sort_by.as_deref() {
            Some("published_at_timestamp:asc") => {
                documents.sort_unstable_by_key(|document| self.document_published(*document));
            }
            _ => documents.sort_unstable_by_key(|document| {
                std::cmp::Reverse(self.document_published(*document))
            }),
        }
        let found = documents.len();
        let mut hits = Vec::with_capacity(limit.min(documents.len()));
        for document in documents.into_iter().skip(offset).take(limit) {
            hits.push(self.hit_value(document, &[], &mut cache)?);
        }
        Ok(json!({
            "found": found,
            "out_of": self.inner.manifest.documents,
            "hits": hits,
        }))
    }

    fn document_published(&self, document: u32) -> i64 {
        self.inner
            .metadata
            .document(document)
            .ok()
            .and_then(|record| self.inner.videos.get(record.video as usize))
            .map_or(0, |video| video.published_at_timestamp)
    }

    fn hit_value(&self, document: u32, terms: &[String], cache: &mut BlockCache) -> Result<Value> {
        let source = self.document_value(document, cache)?;
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

    pub fn search_channels(&self, params: &SearchParams) -> Value {
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
        json!({ "found": found, "out_of": found, "hits": hits })
    }
}

fn retain_best_document(documents: &mut Vec<DocMatch>, candidate: DocMatch, limit: usize) {
    if documents.len() < limit {
        documents.push(candidate);
        return;
    }
    let Some((worst_index, worst)) = documents
        .iter()
        .enumerate()
        .min_by(|left, right| left.1.score.total_cmp(&right.1.score))
    else {
        return;
    };
    if candidate.score > worst.score {
        documents[worst_index] = candidate;
    }
}

fn descending_doc_score(left: &DocMatch, right: &DocMatch) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.document.cmp(&right.document))
}

fn intersect_all(mut lists: Vec<Vec<DocMatch>>) -> Vec<DocMatch> {
    if lists.is_empty() {
        return Vec::new();
    }
    lists.sort_unstable_by_key(Vec::len);
    let mut intersection = lists.remove(0);
    for list in lists {
        intersection = intersect_sorted(intersection, list);
        if intersection.is_empty() {
            break;
        }
    }
    intersection
}

fn intersect_sorted(left: Vec<DocMatch>, right: Vec<DocMatch>) -> Vec<DocMatch> {
    let mut output = Vec::with_capacity(left.len().min(right.len()));
    let mut left_index = 0_usize;
    let mut right_index = 0_usize;
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].document.cmp(&right[right_index].document) {
            Ordering::Less => left_index += 1,
            Ordering::Greater => right_index += 1,
            Ordering::Equal => {
                output.push(DocMatch {
                    document: left[left_index].document,
                    score: left[left_index].score + right[right_index].score,
                });
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

fn union_sorted(left: Vec<DocMatch>, right: Vec<DocMatch>) -> Vec<DocMatch> {
    let mut output = Vec::with_capacity(left.len().saturating_add(right.len()));
    let mut left = left.into_iter().peekable();
    let mut right = right.into_iter().peekable();
    loop {
        match (left.peek(), right.peek()) {
            (Some(left_value), Some(right_value)) => {
                match left_value.document.cmp(&right_value.document) {
                    Ordering::Less => output.push(left.next().expect("peeked left")),
                    Ordering::Greater => output.push(right.next().expect("peeked right")),
                    Ordering::Equal => {
                        let left_value = left.next().expect("peeked left");
                        let right_value = right.next().expect("peeked right");
                        output.push(DocMatch {
                            document: left_value.document,
                            score: left_value.score + right_value.score,
                        });
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

fn token_counts(value: &str) -> HashMap<String, u16> {
    let mut counts = HashMap::new();
    let mut token = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
            continue;
        }
        record_token(&mut counts, &mut token);
    }
    record_token(&mut counts, &mut token);
    counts
}

fn record_token(counts: &mut HashMap<String, u16>, token: &mut String) {
    if token.is_empty() {
        return;
    }
    let owned = std::mem::take(token);
    let count = counts.entry(owned).or_insert(0_u16);
    *count = count.saturating_add(1);
}

fn unique_tokens(value: &str) -> Vec<String> {
    let counts = token_counts(value);
    let mut seen = HashSet::with_capacity(counts.len());
    let mut output = Vec::with_capacity(counts.len());
    let mut token = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
            continue;
        }
        push_unique_token(&mut output, &mut seen, &mut token);
    }
    push_unique_token(&mut output, &mut seen, &mut token);
    output
}

fn push_unique_token(output: &mut Vec<String>, seen: &mut HashSet<String>, token: &mut String) {
    if token.is_empty() {
        return;
    }
    let owned = std::mem::take(token);
    if seen.insert(owned.clone()) {
        output.push(owned);
    }
}

fn query_fields(query_by: Option<&str>) -> QueryFields {
    let requested = query_by.unwrap_or("text,title");
    let text = requested.split(',').any(|field| field.trim() == "text");
    let title = requested.split(',').any(|field| field.trim() == "title");
    if !text && !title {
        return QueryFields {
            text: true,
            title: false,
        };
    }
    QueryFields { text, title }
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

fn response_found(response: &Value) -> u64 {
    response
        .get("found")
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

fn parse_filter(filter: Option<&str>) -> Option<(&str, &str)> {
    let (field, value) = filter?.split_once(":=")?;
    let value = value.trim().trim_matches('`');
    (!field.trim().is_empty() && !value.is_empty()).then_some((field.trim(), value))
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
            json!({"id":"a-1","video_id":"a","title":"Rust performance","text":"a compact inverted index about relativity","youtuber_handle":"@one","youtuber_name":"One","t_start_ms":1000,"t_end_ms":2000,"published_at_timestamp":20}),
            json!({"id":"a-2","video_id":"a","title":"Rust performance","text":"memory mapped search is fast","youtuber_handle":"@one","youtuber_name":"One","t_start_ms":3000,"t_end_ms":4000,"published_at_timestamp":20}),
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

        let channels = engine.search_channels(&SearchParams {
            q: "sauce".to_string(),
            query_by: None,
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
        });
        assert_eq!(channels["found"], 1);
        assert_eq!(channels["hits"][0]["document"]["handle"], "@vsauce");
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
    fn truncated_postings_are_rejected_during_query_decoding() {
        let (root, engine) = test_engine();
        drop(engine);
        let postings_path = root.path().join("index").join(POSTINGS_FILE);
        OpenOptions::new()
            .write(true)
            .open(&postings_path)
            .expect("postings")
            .set_len(HEADER_BYTES as u64)
            .expect("truncate postings");
        let engine = RawSearchEngine::open(&root.path().join("index")).expect("open headers");
        assert!(matches!(
            engine.search(&grouped_query("memory")),
            Err(Error::Corrupt(_))
        ));
    }

    #[test]
    fn truncated_document_blocks_are_rejected_during_hydration() {
        let (root, engine) = test_engine();
        drop(engine);
        let documents_path = root.path().join("index").join(DOCUMENTS_FILE);
        OpenOptions::new()
            .write(true)
            .open(&documents_path)
            .expect("documents")
            .set_len(HEADER_BYTES as u64)
            .expect("truncate documents");
        let engine = RawSearchEngine::open(&root.path().join("index")).expect("open headers");
        assert!(engine.search(&grouped_query("bun")).is_err());
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
