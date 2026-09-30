//! Schema-9 build-time accelerators.
//!
//! A schema-9 generation is a schema-7 generation plus two sidecars. Neither
//! changes what a search returns; they only let the engine skip work.
//!
//! `accelerators.bin` holds, for each long transcript posting list, one
//! entry per block of `block_postings` postings: the block's last document,
//! the block's maximum document-dependent score factor (BM25 term-frequency
//! and length part times the first-position multiplier), and the cursor
//! state after the block. Seeks jump through the states; block-priority
//! ranking bounds scores with the maxima.
//!
//! `bitmaps.bin` holds dense transcript bitmaps for the highest-DF terms
//! within a byte budget, and title bitmaps over videos for common title
//! terms, so exact totals do not decode the broadest posting lists.

use super::skips::{PositionedStreams, SkipEntry, build_skip_entries};
use super::*;

pub(super) const ACCELERATORS_FILE: &str = "accelerators.bin";
pub(super) const BITMAPS_FILE: &str = "bitmaps.bin";
const ACCELERATORS_MAGIC: &[u8; 8] = b"RAWACC01";
const BITMAPS_MAGIC: &[u8; 8] = b"RAWBMP01";
const ACCELERATOR_PARAMETER_BYTES: usize = 8;
const ACCELERATOR_DIRECTORY_BYTES: usize = 24;
const BLOCK_BYTES: usize = 8;
const CHECKPOINT_BYTES: usize = 20;
const BITMAP_UNIVERSE_BYTES: usize = 8;
const BITMAP_DIRECTORY_BYTES: usize = 16;

#[derive(Clone, Copy, Debug)]
pub(super) struct AcceleratorConfig {
    /// Lists shorter than this get no entry; they are cheap to decode.
    pub(super) min_postings: u32,
    pub(super) block_postings: u32,
    pub(super) text_bitmap_budget_bytes: usize,
    /// Title terms on at least this many per mille of videos get a bitmap.
    pub(super) title_bitmap_min_per_mille: u64,
    pub(super) plan_bounds: super::plan_bounds::PlanBoundConfig,
}

impl AcceleratorConfig {
    pub(super) const PRODUCTION: Self = Self {
        min_postings: 4_096,
        block_postings: 128,
        text_bitmap_budget_bytes: 64 * 1024 * 1024,
        title_bitmap_min_per_mille: 1,
        plan_bounds: super::plan_bounds::PlanBoundConfig {
            window_documents: 512,
            min_documents: super::conjunction::STREAMING_CONJUNCTION_MIN_DOCUMENTS,
        },
    };
}

/// One posting block: its last document and its maximum score factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BlockBound {
    pub(super) last_document: u32,
    pub(super) max_factor: f32,
}

/// The document-dependent part of a transcript contribution. A contribution
/// is this times the term's IDF, the field weight, and the alternative
/// multiplier; the position multiplier is included at its enabled value,
/// which is at least the disabled value of one.
pub(super) fn score_factor(
    frequency: u32,
    document_tokens: u32,
    average_tokens: f32,
    first_position: u32,
) -> f32 {
    bm25_with_length_norm(frequency, bm25_length_norm(document_tokens, average_tokens))
        * token_position_multiplier(first_position, true)
}

/// Adds schema-9 accelerators to a schema-7 generation with production
/// thresholds.
pub fn accelerate_generation(index_path: &Path) -> Result<()> {
    upgrade_to_schema_nine(index_path, AcceleratorConfig::PRODUCTION)
}

/// Turns a complete schema-7 generation into schema 9 by adding the
/// sidecars and rewriting the manifest. Every schema-7 file is left as is,
/// so a schema-9 generation answers exactly as its schema-7 source.
pub(super) fn upgrade_to_schema_nine(index_path: &Path, config: AcceleratorConfig) -> Result<()> {
    let manifest_path = index_path.join(MANIFEST_FILE);
    let mut manifest: Manifest =
        serde_json::from_reader(BufReader::new(File::open(&manifest_path)?))?;
    if manifest.schema_version != 7 {
        return Err(Error::Corrupt(format!(
            "accelerators need a schema-7 generation, found schema {}",
            manifest.schema_version
        )));
    }
    let engine = RawSearchEngine::open_newly_built(index_path)?;
    write_accelerators(&engine, index_path, config)?;
    super::plan_bounds::write_plan_bounds(&engine, index_path, config.plan_bounds)?;
    drop(engine);
    for name in [
        ACCELERATORS_FILE,
        BITMAPS_FILE,
        super::plan_bounds::PLAN_BOUNDS_FILE,
    ] {
        let path = index_path.join(name);
        manifest.files.insert(
            name.to_string(),
            ManifestFile {
                bytes: fs::metadata(&path)?.len(),
                blake3: checksum_file(&path)?,
            },
        );
    }
    manifest.schema_version = 9;
    // Replace the manifest atomically: a crash leaves the schema-7 manifest,
    // which still validates (it ignores unlisted files), never a partial one.
    let staged = index_path.join(format!("{MANIFEST_FILE}.schema9"));
    let mut output = ColdWriter::create(&staged)?;
    serde_json::to_writer_pretty(&mut output, &manifest)?;
    output.finish()?;
    fs::rename(&staged, &manifest_path)?;
    File::open(index_path)?.sync_all()?;
    Ok(())
}

pub(super) fn write_accelerators(
    engine: &RawSearchEngine,
    index_path: &Path,
    config: AcceleratorConfig,
) -> Result<()> {
    let inner = &engine.inner;
    let mut entries = Vec::new();
    for index in 0..inner.lexicon.count {
        let (_, entry) = inner.lexicon.entry_at(index)?;
        if entry.text_frequency >= config.min_postings.max(1) {
            entries.push(entry);
        }
    }
    entries.sort_unstable_by_key(|entry| entry.term);

    let mut directory = Vec::with_capacity(entries.len() * ACCELERATOR_DIRECTORY_BYTES);
    let mut payload = Vec::new();
    let payload_start =
        HEADER_BYTES + ACCELERATOR_PARAMETER_BYTES + entries.len() * ACCELERATOR_DIRECTORY_BYTES;
    for entry in &entries {
        let streams = text_streams(engine, *entry)?;
        let blocks = block_bounds(engine, streams, config.block_postings)?;
        let checkpoints = build_skip_entries(streams, config.block_postings)?
            .into_iter()
            .filter(|checkpoint| checkpoint.decoded < entry.text_frequency)
            .collect::<Vec<_>>();
        if checkpoints.len() + 1 != blocks.len() {
            return Err(Error::Corrupt(
                "accelerator checkpoints do not match posting blocks".to_string(),
            ));
        }
        let offset = u64::try_from(payload_start + payload.len())
            .map_err(|_| Error::Corrupt("accelerator offset overflows".to_string()))?;
        directory.extend_from_slice(&entry.term.to_le_bytes());
        directory.extend_from_slice(&u32_len(blocks.len())?.to_le_bytes());
        directory.extend_from_slice(&entry.text_frequency.to_le_bytes());
        directory.extend_from_slice(&0_u32.to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        for block in &blocks {
            payload.extend_from_slice(&block.last_document.to_le_bytes());
            payload.extend_from_slice(&block.max_factor.to_le_bytes());
        }
        for checkpoint in &checkpoints {
            for value in [
                checkpoint.previous_id,
                checkpoint.decoded,
                u32_len(checkpoint.posting_cursor)?,
                u32_len(checkpoint.first_cursor)?,
                u32_len(checkpoint.remaining_cursor)?,
            ] {
                payload.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    let mut output = Vec::with_capacity(payload_start + payload.len());
    write_header(&mut output, ACCELERATORS_MAGIC, u32_len(entries.len())?);
    output.extend_from_slice(&config.block_postings.to_le_bytes());
    output.extend_from_slice(&config.min_postings.to_le_bytes());
    output.extend_from_slice(&directory);
    output.extend_from_slice(&payload);
    let mut file = ColdWriter::create(&index_path.join(ACCELERATORS_FILE))?;
    file.write_all(&output)?;
    file.finish()?;

    write_bitmaps(engine, index_path, config)
}

fn write_bitmaps(
    engine: &RawSearchEngine,
    index_path: &Path,
    config: AcceleratorConfig,
) -> Result<()> {
    let inner = &engine.inner;
    let documents = inner.manifest.documents;
    let videos = u32_len(inner.videos.len())?;
    let text_bytes = (documents as usize).div_ceil(64) * 8;
    let mut text_terms = Vec::new();
    let mut title_terms = Vec::new();
    for index in 0..inner.lexicon.count {
        let (_, entry) = inner.lexicon.entry_at(index)?;
        if entry.text_frequency > 0 {
            text_terms.push(entry);
        }
        if entry.title_frequency > 0
            && u64::from(entry.title_frequency) * 1_000
                >= u64::from(videos) * config.title_bitmap_min_per_mille
        {
            title_terms.push(entry);
        }
    }
    text_terms.sort_unstable_by(|left, right| {
        right
            .text_frequency
            .cmp(&left.text_frequency)
            .then(left.term.cmp(&right.term))
    });
    text_terms.truncate(
        config
            .text_bitmap_budget_bytes
            .checked_div(text_bytes)
            .unwrap_or(0),
    );

    let mut selected = text_terms
        .iter()
        .map(|entry| (*entry, IndexedField::Text))
        .chain(
            title_terms
                .iter()
                .map(|entry| (*entry, IndexedField::Title)),
        )
        .collect::<Vec<_>>();
    selected.sort_unstable_by_key(|(entry, field)| (entry.term, *field as u8));

    let payload_start =
        HEADER_BYTES + BITMAP_UNIVERSE_BYTES + selected.len() * BITMAP_DIRECTORY_BYTES;
    let mut directory = Vec::with_capacity(selected.len() * BITMAP_DIRECTORY_BYTES);
    let mut payload = Vec::new();
    for (entry, field) in selected {
        let (offset, length, universe) = match field {
            IndexedField::Text => (entry.text_offset, entry.text_length, documents),
            IndexedField::Title => (entry.title_offset, entry.title_length, videos),
        };
        let mut words = vec![0_u64; (universe as usize).div_ceil(64)];
        let bytes = engine.postings_range(offset, length)?;
        let (mut cursor, mut id) = (0, 0_u32);
        while cursor < bytes.len() {
            id = id
                .checked_add(read_varint(bytes, &mut cursor, bytes.len())?)
                .filter(|id| *id < universe)
                .ok_or_else(|| Error::Corrupt("bitmap posting is out of bounds".to_string()))?;
            read_varint(bytes, &mut cursor, bytes.len())?;
            words[(id / 64) as usize] |= 1 << (id % 64);
        }
        let position = u64::try_from(payload_start + payload.len())
            .map_err(|_| Error::Corrupt("bitmap offset overflows".to_string()))?;
        directory.extend_from_slice(&entry.term.to_le_bytes());
        directory.push(field as u8);
        directory.extend_from_slice(&[0; 3]);
        directory.extend_from_slice(&position.to_le_bytes());
        for word in words {
            payload.extend_from_slice(&word.to_le_bytes());
        }
    }
    let mut output = Vec::with_capacity(payload_start + payload.len());
    write_header(
        &mut output,
        BITMAPS_MAGIC,
        u32_len(directory.len() / BITMAP_DIRECTORY_BYTES)?,
    );
    output.extend_from_slice(&documents.to_le_bytes());
    output.extend_from_slice(&videos.to_le_bytes());
    output.extend_from_slice(&directory);
    output.extend_from_slice(&payload);
    let mut file = ColdWriter::create(&index_path.join(BITMAPS_FILE))?;
    file.write_all(&output)?;
    Ok(file.finish()?)
}

fn u32_len(value: usize) -> Result<u32> {
    u32::try_from(value).map_err(|_| Error::Corrupt("accelerator value exceeds u32".to_string()))
}

pub(super) fn text_streams(
    engine: &RawSearchEngine,
    entry: TermEntry,
) -> Result<PositionedStreams<'_>> {
    Ok(PositionedStreams {
        postings: engine.postings_range(entry.text_offset, entry.text_length)?,
        first_positions: engine
            .inner
            .positions
            .range(entry.term, IndexedField::Text)?,
        remaining_positions: engine
            .inner
            .full_positions
            .range(entry.term, IndexedField::Text)?,
        expected: entry.text_frequency,
    })
}

/// Decodes one list and records each block's last document and maximum
/// score factor, with the same validation as the posting cursors.
pub(super) fn block_bounds(
    engine: &RawSearchEngine,
    streams: PositionedStreams<'_>,
    block_postings: u32,
) -> Result<Vec<BlockBound>> {
    let average = engine.inner.manifest.average_document_tokens;
    let documents = engine.inner.manifest.documents;
    let (postings, first_positions) = (streams.postings, streams.first_positions);
    let (mut cursor, mut first_cursor) = (0, 0);
    let (mut document, mut decoded) = (0_u32, 0_u32);
    let mut blocks = Vec::new();
    let mut current: Option<BlockBound> = None;
    while cursor < postings.len() {
        let delta = read_varint(postings, &mut cursor, postings.len())?;
        if decoded > 0 && delta == 0 {
            return Err(Error::Corrupt(
                "posting identifiers must be strictly increasing".to_string(),
            ));
        }
        document = document
            .checked_add(delta)
            .filter(|document| *document < documents)
            .ok_or_else(|| Error::Corrupt("text posting document is out of bounds".to_string()))?;
        let frequency = read_varint(postings, &mut cursor, postings.len())?;
        let first_position =
            read_varint(first_positions, &mut first_cursor, first_positions.len())?;
        let tokens = engine.inner.metadata.search_document(document)?.tokens;
        let factor = score_factor(frequency, tokens, average, first_position);
        let block = current.get_or_insert(BlockBound {
            last_document: document,
            max_factor: factor,
        });
        block.last_document = document;
        block.max_factor = block.max_factor.max(factor);
        decoded += 1;
        if decoded.is_multiple_of(block_postings) {
            blocks.extend(current.take());
        }
    }
    blocks.extend(current.take());
    if decoded != streams.expected || first_cursor != first_positions.len() {
        return Err(Error::Corrupt(
            "accelerator posting count does not match lexicon".to_string(),
        ));
    }
    Ok(blocks)
}

/// Validated view of `accelerators.bin`.
pub(super) struct Accelerators {
    file: MappedFile,
    block_postings: u32,
    /// Sorted by term: (term, blocks, postings, payload offset).
    directory: Vec<(u32, u32, u32, usize)>,
}

impl Accelerators {
    pub(super) fn open(path: &Path, terms: usize) -> Result<Self> {
        let file = MappedFile::open(path, ACCELERATORS_MAGIC)?;
        let bytes = file.bytes();
        let count = read_u32(bytes, 8)? as usize;
        let block_postings = read_u32(bytes, HEADER_BYTES)?;
        if block_postings == 0 {
            return Err(Error::Corrupt("accelerator block size is zero".to_string()));
        }
        let mut directory: Vec<(u32, u32, u32, usize)> = Vec::with_capacity(count);
        let mut end =
            HEADER_BYTES + ACCELERATOR_PARAMETER_BYTES + count * ACCELERATOR_DIRECTORY_BYTES;
        for index in 0..count {
            let at =
                HEADER_BYTES + ACCELERATOR_PARAMETER_BYTES + index * ACCELERATOR_DIRECTORY_BYTES;
            let term = read_u32(bytes, at)?;
            let blocks = read_u32(bytes, at + 4)?;
            let postings = read_u32(bytes, at + 8)?;
            let offset = usize::try_from(read_u64(bytes, at + 16)?)
                .map_err(|_| Error::Corrupt("accelerator offset overflows".to_string()))?;
            let expected_blocks = postings.div_ceil(block_postings);
            if term as usize >= terms
                || directory
                    .last()
                    .is_some_and(|(previous, ..)| *previous >= term)
                || blocks != expected_blocks
                || offset != end
            {
                return Err(Error::Corrupt(
                    "accelerator directory is invalid".to_string(),
                ));
            }
            end = offset
                + blocks as usize * BLOCK_BYTES
                + (blocks as usize).saturating_sub(1) * CHECKPOINT_BYTES;
            directory.push((term, blocks, postings, offset));
        }
        if end != bytes.len() {
            return Err(Error::Corrupt(
                "accelerator file has an invalid length".to_string(),
            ));
        }
        Ok(Self {
            file,
            block_postings,
            directory,
        })
    }

    pub(super) fn contains(&self, term: u32) -> bool {
        self.entry(term).is_some()
    }

    fn entry(&self, term: u32) -> Option<(u32, u32, usize)> {
        let index = self
            .directory
            .binary_search_by_key(&term, |(term, ..)| *term)
            .ok()?;
        let (_, blocks, postings, offset) = self.directory[index];
        Some((blocks, postings, offset))
    }

    /// Block bounds of one term, or `None` for a short list. Documents must
    /// increase strictly; a violation is corruption.
    pub(super) fn blocks(&self, term: u32) -> Result<Option<Vec<BlockBound>>> {
        let Some((blocks, _, offset)) = self.entry(term) else {
            return Ok(None);
        };
        let bytes = self.file.bytes();
        let mut bounds = Vec::with_capacity(blocks as usize);
        for index in 0..blocks as usize {
            let at = offset + index * BLOCK_BYTES;
            let bound = BlockBound {
                last_document: read_u32(bytes, at)?,
                max_factor: f32::from_bits(read_u32(bytes, at + 4)?),
            };
            if bounds
                .last()
                .is_some_and(|previous: &BlockBound| previous.last_document >= bound.last_document)
                || !bound.max_factor.is_finite()
            {
                return Err(Error::Corrupt(
                    "accelerator block bounds are invalid".to_string(),
                ));
            }
            bounds.push(bound);
        }
        Ok(Some(bounds))
    }

    /// Seek checkpoints after every full block, validated against the
    /// term's streams so a cursor never resumes outside them.
    pub(super) fn checkpoints(
        &self,
        term: u32,
        streams: PositionedStreams<'_>,
    ) -> Result<Option<Arc<[SkipEntry]>>> {
        let Some((blocks, postings, offset)) = self.entry(term) else {
            return Ok(None);
        };
        if postings != streams.expected {
            return Err(Error::Corrupt(
                "accelerator posting count does not match lexicon".to_string(),
            ));
        }
        let bytes = self.file.bytes();
        let start = offset + blocks as usize * BLOCK_BYTES;
        let mut entries: Vec<SkipEntry> = Vec::with_capacity((blocks as usize).saturating_sub(1));
        for index in 0..(blocks as usize).saturating_sub(1) {
            let at = start + index * CHECKPOINT_BYTES;
            let entry = SkipEntry {
                previous_id: read_u32(bytes, at)?,
                decoded: read_u32(bytes, at + 4)?,
                posting_cursor: read_u32(bytes, at + 8)? as usize,
                first_cursor: read_u32(bytes, at + 12)? as usize,
                remaining_cursor: read_u32(bytes, at + 16)? as usize,
            };
            let expected_decoded = (index as u32 + 1) * self.block_postings;
            let monotonic = entries.last().is_none_or(|previous| {
                previous.previous_id < entry.previous_id
                    && previous.posting_cursor < entry.posting_cursor
                    && previous.first_cursor < entry.first_cursor
                    && previous.remaining_cursor <= entry.remaining_cursor
            });
            if entry.decoded != expected_decoded
                || !monotonic
                || entry.posting_cursor > streams.postings.len()
                || entry.first_cursor > streams.first_positions.len()
                || entry.remaining_cursor > streams.remaining_positions.len()
            {
                return Err(Error::Corrupt(
                    "accelerator checkpoints are invalid".to_string(),
                ));
            }
            entries.push(entry);
        }
        Ok(Some(entries.into()))
    }
}

/// Validated view of `bitmaps.bin`.
pub(super) struct Bitmaps {
    file: MappedFile,
    /// Sorted by (term, field): payload offset.
    directory: Vec<((u32, u8), usize)>,
    documents: u32,
    videos: u32,
}

impl Bitmaps {
    pub(super) fn open(path: &Path, terms: usize, documents: u32, videos: u32) -> Result<Self> {
        let file = MappedFile::open(path, BITMAPS_MAGIC)?;
        let bytes = file.bytes();
        let count = read_u32(bytes, 8)? as usize;
        if read_u32(bytes, HEADER_BYTES)? != documents
            || read_u32(bytes, HEADER_BYTES + 4)? != videos
        {
            return Err(Error::Corrupt(
                "bitmap universes do not match the generation".to_string(),
            ));
        }
        let mut directory: Vec<((u32, u8), usize)> = Vec::with_capacity(count);
        let mut end = HEADER_BYTES + BITMAP_UNIVERSE_BYTES + count * BITMAP_DIRECTORY_BYTES;
        for index in 0..count {
            let at = HEADER_BYTES + BITMAP_UNIVERSE_BYTES + index * BITMAP_DIRECTORY_BYTES;
            let term = read_u32(bytes, at)?;
            let field = bytes[at + 4];
            let offset = usize::try_from(read_u64(bytes, at + 8)?)
                .map_err(|_| Error::Corrupt("bitmap offset overflows".to_string()))?;
            let universe = match field {
                0 => documents,
                1 => videos,
                _ => return Err(Error::Corrupt("bitmap field is invalid".to_string())),
            };
            if term as usize >= terms
                || directory
                    .last()
                    .is_some_and(|(previous, _)| *previous >= (term, field))
                || offset != end
            {
                return Err(Error::Corrupt("bitmap directory is invalid".to_string()));
            }
            end = offset + (universe as usize).div_ceil(64) * 8;
            directory.push(((term, field), offset));
        }
        if end != bytes.len() {
            return Err(Error::Corrupt(
                "bitmap file has an invalid length".to_string(),
            ));
        }
        Ok(Self {
            file,
            directory,
            documents,
            videos,
        })
    }

    /// The term's bitmap words for `field`, if one was materialized.
    pub(super) fn words(
        &self,
        term: u32,
        field: IndexedField,
    ) -> Option<impl Iterator<Item = u64> + '_> {
        let index = self
            .directory
            .binary_search_by_key(&(term, field as u8), |(key, _)| *key)
            .ok()?;
        let offset = self.directory[index].1;
        let universe = match field {
            IndexedField::Text => self.documents,
            IndexedField::Title => self.videos,
        };
        let words = (universe as usize).div_ceil(64);
        let bytes = &self.file.bytes()[offset..offset + words * 8];
        Some(
            bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|chunk| u64::from_le_bytes(*chunk)),
        )
    }
}
