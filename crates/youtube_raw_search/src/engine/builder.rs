use super::*;

#[derive(Default)]
struct TermBuild {
    text: Vec<BuildPosting>,
    title: Vec<BuildPosting>,
}

#[derive(Clone, Copy)]
pub(super) struct BuildPosting {
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
    output: ColdWriter,
    position: u32,
    scratch: Vec<u8>,
}

struct SpoolDirectory {
    path: PathBuf,
}

struct ShardSpoolWriter {
    directory: PathBuf,
    outputs: Vec<Option<ColdWriter>>,
    key_scratch: Vec<u8>,
}

impl SpoolDirectory {
    fn create(path: PathBuf) -> Result<Self> {
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SpoolDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), %error, "failed to clean build spool");
        }
    }
}

impl ShardSpoolWriter {
    fn create(directory: &Path) -> Result<Self> {
        Ok(Self {
            directory: directory.to_path_buf(),
            outputs: (0..POSTING_SHARDS).map(|_| None).collect(),
            key_scratch: Vec::new(),
        })
    }

    fn append_posting(
        &mut self,
        term: &str,
        posting: BuildPosting,
        field: IndexedField,
    ) -> Result<()> {
        let shard = usize::from(term.as_bytes()[0]);
        if self.outputs[shard].is_none() {
            self.outputs[shard] = Some(ColdWriter::create_posting_shard(
                &self.directory.join(format!("{shard:03}.bin")),
            )?);
        }
        write_spool_posting(
            self.outputs[shard].as_mut().expect("posting shard exists"),
            term,
            posting,
            field,
        )
    }

    fn append_bigram(
        &mut self,
        left: &str,
        right: &str,
        id: u32,
        earliest_end_offset: u32,
        field: IndexedField,
    ) -> Result<()> {
        self.key_scratch.clear();
        self.key_scratch.extend_from_slice(left.as_bytes());
        self.key_scratch.push(0);
        self.key_scratch.extend_from_slice(right.as_bytes());
        let shard = usize::from(self.key_scratch[0]);
        if self.outputs[shard].is_none() {
            self.outputs[shard] = Some(ColdWriter::create_posting_shard(
                &self.directory.join(format!("{shard:03}.bin")),
            )?);
        }
        write_spool_bigram(
            self.outputs[shard].as_mut().expect("bigram shard exists"),
            &self.key_scratch,
            id,
            earliest_end_offset,
            field,
        )
    }

    fn finish(self) -> Result<()> {
        for output in self.outputs.into_iter().flatten() {
            output.finish()?;
        }
        Ok(())
    }
}

impl PositionSpoolWriter {
    fn create(path: &Path) -> Result<Self> {
        let mut output = ColdWriter::create(path)?;
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

    fn finish(self) -> Result<()> {
        self.output.finish()?;
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
    postings: &mut ShardSpoolWriter,
    counts: TokenCounts,
    id: u32,
    field: IndexedField,
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
        postings.append_posting(&term, posting, field)?;
    }
    Ok(())
}

pub fn build_index(
    chunks_path: &Path,
    channels_path: &Path,
    index_path: &Path,
) -> Result<BuildReport> {
    let chunk_lines = BufReader::new(ColdReader::open(chunks_path)?)
        .lines()
        .map(|line| line.map_err(Error::from));
    let channels = read_json_lines(channels_path)?;
    build_index_from_lines(chunk_lines, channels, index_path)
}

/// Builds and opens a private candidate without hashing its generation files a
/// second time. Manifest construction at the end of `build_index_from_lines`
/// has already read and hashed every immutable file, and this wrapper does not
/// expose a point where another caller can modify the candidate between those
/// checks and the structural open.
pub(crate) fn build_index_from_lines_and_open<I>(
    chunk_lines: I,
    channels: Vec<Value>,
    index_path: &Path,
) -> Result<(BuildReport, RawSearchEngine)>
where
    I: Iterator<Item = Result<String>>,
{
    let report = build_index_from_lines(chunk_lines, channels, index_path)?;
    let engine = RawSearchEngine::open_newly_built(index_path)?;
    Ok((report, engine))
}

fn build_index_from_lines<I>(
    chunk_lines: I,
    channels: Vec<Value>,
    index_path: &Path,
) -> Result<BuildReport>
where
    I: Iterator<Item = Result<String>>,
{
    let started = Instant::now();
    if index_path.join(MANIFEST_FILE).exists() {
        return Err(Error::ExistingIndex(index_path.display().to_string()));
    }
    fs::create_dir_all(index_path)?;

    let documents_path = index_path.join(DOCUMENTS_FILE);
    let mut document_output = ColdWriter::create(&documents_path)?;
    let mut document_header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut document_header, DOCUMENTS_MAGIC, 0);
    document_output.write_all(&document_header)?;
    let position_spool_path = index_path.join(POSITION_SPOOL_FILE);
    let mut position_spool = PositionSpoolWriter::create(&position_spool_path)?;
    let mut position_scratch = Vec::new();

    let posting_spool = SpoolDirectory::create(index_path.join(POSTING_SPOOL_DIRECTORY))?;
    let mut posting_spool_writer = ShardSpoolWriter::create(posting_spool.path())?;
    let mut videos = Vec::<VideoMeta>::new();
    let mut video_ids = HashMap::<String, u32>::new();
    let document_record_spool_path = index_path.join(DOCUMENT_RECORD_SPOOL_FILE);
    let mut document_record_spool = ColdWriter::create(&document_record_spool_path)?;
    let mut document_count = 0_u32;
    let mut last_document_start = Vec::<Option<u32>>::new();
    let mut block_records = Vec::<BlockRecord>::new();
    let mut block_bytes = Vec::<u8>::with_capacity(256 * 1024);
    let mut block_document_count = 0_usize;
    let mut total_tokens = 0_u64;

    for line in chunk_lines {
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
                &mut posting_spool_writer,
                title_counts,
                video,
                IndexedField::Title,
                &mut position_spool,
                &mut position_scratch,
            )?;
            videos.push(VideoMeta {
                id: source.video_id.clone(),
                youtuber_id: source.youtuber_id.to_ascii_lowercase(),
                handle: source.youtuber_handle.to_ascii_lowercase(),
                title: source.title,
                published_at_timestamp: published_at_timestamp.unwrap_or_default(),
                view_count: source.view_count,
                like_count: source.like_count,
                comment_count: source.comment_count,
                documents: Vec::new(),
            });
            last_document_start.push(None);
            video_ids.insert(source.video_id, video);
            video
        };

        let document = document_count;
        document_count = document_count.checked_add(1).ok_or_else(|| {
            Error::InvalidInput("corpus contains more than 4 billion documents".to_string())
        })?;
        let text_counts = token_counts(&source.text)?;
        let document_tokens = text_counts
            .terms
            .values()
            .map(|token| u32::from(token.frequency))
            .sum::<u32>();
        total_tokens = total_tokens.saturating_add(u64::from(document_tokens));
        append_token_postings(
            &mut posting_spool_writer,
            text_counts,
            document,
            IndexedField::Text,
            &mut position_spool,
            &mut position_scratch,
        )?;

        let line_bytes = line.as_bytes();
        let offset = block_bytes.len() as u32;
        block_bytes.extend_from_slice(line_bytes);
        if let Some(previous_start) = last_document_start[video as usize]
            && source.t_start_ms < previous_start
        {
            return Err(Error::InvalidInput(format!(
                "video {} has out-of-order moments: {} follows {}",
                videos[video as usize].id, source.t_start_ms, previous_start
            )));
        }
        let record = DocumentRecord {
            block: block_records.len() as u32,
            offset,
            length: line_bytes.len() as u32,
            video,
            tokens: document_tokens,
            t_start_ms: source.t_start_ms,
        };
        write_document_record(&mut document_record_spool, record)?;
        last_document_start[video as usize] = Some(source.t_start_ms);
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
    document_output.finish()?;
    document_record_spool.finish()?;
    position_spool.finish()?;
    posting_spool_writer.finish()?;

    let mut channel_output = ColdWriter::create(&index_path.join(CHANNELS_FILE))?;
    serde_json::to_writer(&mut channel_output, &channels)?;
    channel_output.finish()?;
    let mut video_output = ColdWriter::create(&index_path.join(VIDEOS_FILE))?;
    serde_json::to_writer(&mut video_output, &videos)?;
    video_output.finish()?;
    write_metadata(
        index_path,
        document_count,
        &document_record_spool_path,
        &block_records,
    )?;
    let position_spool = MappedFile::open(&position_spool_path, POSITION_SPOOL_MAGIC)?;
    let lexicon = write_postings(index_path, posting_spool.path(), &videos, &position_spool)?;
    drop(posting_spool);
    drop(position_spool);
    fs::remove_file(&position_spool_path)?;

    // Bigram records used to be emitted alongside unigram records. On the
    // production corpus that kept a 3 GiB bigram spool alive at the same time
    // as the 2.6 GiB posting spool. Replaying the compressed document store
    // after the lexicon is complete makes those temporary lifetimes disjoint.
    let bigram_spool = SpoolDirectory::create(index_path.join(BIGRAM_SPOOL_DIRECTORY))?;
    let mut bigram_spool_writer = ShardSpoolWriter::create(bigram_spool.path())?;
    let covered_terms = lexicon
        .iter()
        .filter(|entry| safe_document_frequency((0, entry)) > MIN_BIGRAM_TERM_DOCUMENTS)
        .map(|entry| entry.term.as_str())
        .collect::<HashSet<_>>();
    for (video, metadata) in videos.iter().enumerate() {
        append_value_bigrams(
            &metadata.title,
            video as u32,
            IndexedField::Title,
            &mut bigram_spool_writer,
            &covered_terms,
        )?;
    }
    replay_document_bigrams(
        &documents_path,
        &document_record_spool_path,
        &block_records,
        document_count,
        &mut bigram_spool_writer,
        &covered_terms,
    )?;
    bigram_spool_writer.finish()?;
    fs::remove_file(&document_record_spool_path)?;
    write_bigram_indexes(
        index_path,
        bigram_spool.path(),
        &lexicon,
        document_count,
        videos.len() as u32,
    )?;
    drop(bigram_spool);
    let term_count = lexicon.len();

    let average_document_tokens = if document_count == 0 {
        0.0
    } else {
        total_tokens as f32 / document_count as f32
    };
    let mut manifest = Manifest {
        schema_version: 7,
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
        files: generation_files(7)?
            .iter()
            .map(|name| {
                Ok((
                    (*name).to_string(),
                    ManifestFile {
                        bytes: fs::metadata(index_path.join(name))?.len(),
                        blake3: String::new(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?,
    };
    let mut manifest_output = ColdWriter::create(&index_path.join(MANIFEST_FILE))?;
    serde_json::to_writer_pretty(&mut manifest_output, &manifest)?;
    manifest_output.finish()?;

    let broad_started = Instant::now();
    let base_elapsed_ms = started.elapsed().as_millis();
    let preliminary = RawSearchEngine::open_newly_built(index_path)?;
    let broad_plans = broad_single::build_index(
        &preliminary,
        &index_path.join(broad_single::BROAD_SINGLE_FILE),
    )?;
    drop(preliminary);
    tracing::info!(
        base_elapsed_ms,
        broad_elapsed_ms = broad_started.elapsed().as_millis(),
        broad_plans,
        broad_bytes = fs::metadata(index_path.join(broad_single::BROAD_SINGLE_FILE))?.len(),
        "raw schema-eight build phases"
    );
    manifest.schema_version = 8;
    manifest.files = build_manifest_files(index_path, 8)?;
    let payload_bytes = manifest
        .files
        .values()
        .try_fold(0_u64, |total, file| total.checked_add(file.bytes))
        .ok_or_else(|| Error::Corrupt("manifest file bytes overflow u64".to_string()))?;
    let mut manifest_output = ColdWriter::create(&index_path.join(MANIFEST_FILE))?;
    serde_json::to_writer_pretty(&mut manifest_output, &manifest)?;
    manifest_output.finish()?;
    let index_bytes = payload_bytes
        .checked_add(fs::metadata(index_path.join(MANIFEST_FILE))?.len())
        .ok_or_else(|| Error::Corrupt("generation file bytes overflow u64".to_string()))?;

    Ok(BuildReport {
        documents: u64::from(document_count),
        videos: videos.len(),
        channels: channels.len(),
        terms: term_count,
        broad_plans,
        index_bytes,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

pub fn slice_corpus(
    input_path: &Path,
    output_path: &Path,
    max_videos: usize,
) -> Result<SliceReport> {
    let mut videos = HashSet::new();
    for line in BufReader::new(ColdReader::open(input_path)?).lines() {
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

    let mut output = ColdWriter::create(output_path)?;
    let mut documents = 0_u64;
    let mut bytes = 0_u64;
    for line in BufReader::new(ColdReader::open(input_path)?).lines() {
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
    output.finish()?;
    Ok(SliceReport {
        documents,
        videos: selected.len(),
        bytes,
    })
}

fn flush_document_block(
    output: &mut ColdWriter,
    uncompressed: &mut Vec<u8>,
    blocks: &mut Vec<BlockRecord>,
) -> Result<()> {
    let compressed = zstd::stream::encode_all(uncompressed.as_slice(), 3)?;
    let offset = output.position();
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
    document_count: u32,
    document_record_spool: &Path,
    blocks: &[BlockRecord],
) -> Result<()> {
    let mut output = ColdWriter::create(&index_path.join(METADATA_FILE))?;
    let mut header = Vec::with_capacity(METADATA_PREFIX_BYTES);
    write_header(&mut header, METADATA_MAGIC, document_count);
    header.extend_from_slice(&(blocks.len() as u32).to_le_bytes());
    header.extend_from_slice(&0_u32.to_le_bytes());
    output.write_all(&header)?;
    std::io::copy(&mut ColdReader::open(document_record_spool)?, &mut output)?;
    for block in blocks {
        output.write_all(&block.offset.to_le_bytes())?;
        output.write_all(&block.compressed_length.to_le_bytes())?;
        output.write_all(&block.uncompressed_length.to_le_bytes())?;
    }
    output.finish()?;
    Ok(())
}

fn write_document_record(output: &mut impl Write, document: DocumentRecord) -> Result<()> {
    output.write_all(&document.block.to_le_bytes())?;
    output.write_all(&document.offset.to_le_bytes())?;
    output.write_all(&document.length.to_le_bytes())?;
    output.write_all(&document.video.to_le_bytes())?;
    output.write_all(&document.tokens.to_le_bytes())?;
    output.write_all(&document.t_start_ms.to_le_bytes())?;
    Ok(())
}

fn replay_document_bigrams(
    documents_path: &Path,
    records_path: &Path,
    blocks: &[BlockRecord],
    document_count: u32,
    output: &mut ShardSpoolWriter,
    covered_terms: &HashSet<&str>,
) -> Result<()> {
    #[derive(Deserialize)]
    struct ReplayDocument<'a> {
        #[serde(borrow)]
        text: std::borrow::Cow<'a, str>,
    }

    let mut documents = ColdReader::open(documents_path)?;
    let mut document_header = [0_u8; HEADER_BYTES];
    documents.read_exact(&mut document_header)?;
    if document_header[..8] != DOCUMENTS_MAGIC[..] {
        return Err(Error::Corrupt(
            "document block store has the wrong magic".to_string(),
        ));
    }
    let mut records = BufReader::new(ColdReader::open(records_path)?);
    let mut decoded_block_id = None;
    let mut decoded_block = Vec::new();
    let mut encoded = [0_u8; 24];
    let mut next_block_offset = HEADER_BYTES as u64;

    for document in 0..document_count {
        records.read_exact(&mut encoded)?;
        let block_id = u32::from_le_bytes(encoded[0..4].try_into().expect("block id bytes"));
        let offset = u32::from_le_bytes(encoded[4..8].try_into().expect("offset bytes"));
        let length = u32::from_le_bytes(encoded[8..12].try_into().expect("length bytes"));
        if decoded_block_id != Some(block_id) {
            let block = blocks
                .get(block_id as usize)
                .ok_or_else(|| Error::Corrupt("document block is out of bounds".to_string()))?;
            if block.offset != next_block_offset {
                return Err(Error::Corrupt(
                    "document blocks are not contiguous".to_string(),
                ));
            }
            let mut compressed = vec![0_u8; block.compressed_length as usize];
            documents.read_exact(&mut compressed)?;
            next_block_offset = next_block_offset
                .checked_add(u64::from(block.compressed_length))
                .ok_or_else(|| Error::Corrupt("document block range overflows".to_string()))?;
            decoded_block.clear();
            let decoder = zstd::stream::read::Decoder::new(compressed.as_slice())?;
            decoder
                .take(u64::from(block.uncompressed_length) + 1)
                .read_to_end(&mut decoded_block)?;
            if decoded_block.len() != block.uncompressed_length as usize {
                return Err(Error::Corrupt(
                    "decoded document block length does not match metadata".to_string(),
                ));
            }
            decoded_block_id = Some(block_id);
        }
        let start = offset as usize;
        let end = start
            .checked_add(length as usize)
            .ok_or_else(|| Error::Corrupt("document range overflows".to_string()))?;
        let line = decoded_block
            .get(start..end)
            .ok_or_else(|| Error::Corrupt("document range is truncated".to_string()))?;
        let source: ReplayDocument<'_> = serde_json::from_slice(line)?;
        append_value_bigrams(
            &source.text,
            document,
            IndexedField::Text,
            output,
            covered_terms,
        )?;
    }
    if records.read(&mut [0_u8; 1])? != 0 {
        return Err(Error::Corrupt(
            "document record spool has trailing bytes".to_string(),
        ));
    }
    Ok(())
}

fn write_postings(
    index_path: &Path,
    posting_spool_directory: &Path,
    videos: &[VideoMeta],
    position_spool: &MappedFile,
) -> Result<Vec<LexiconBuildEntry>> {
    let shard_paths = refine_posting_shards(posting_spool_directory)?;
    let mut term_count = 0_usize;
    for path in &shard_paths {
        term_count = term_count
            .checked_add(read_posting_shard(path)?.len())
            .ok_or_else(|| Error::Corrupt("term count overflows usize".to_string()))?;
    }
    let term_count_u32 = u32::try_from(term_count)
        .map_err(|_| Error::Corrupt("term count exceeds u32".to_string()))?;
    let mut postings = ColdWriter::create(&index_path.join(POSTINGS_FILE))?;
    let mut postings_header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut postings_header, POSTINGS_MAGIC, term_count_u32);
    postings.write_all(&postings_header)?;
    let mut positions = ColdWriter::create(&index_path.join(POSITIONS_FILE))?;
    let mut positions_header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut positions_header, POSITIONS_MAGIC, term_count_u32);
    positions.write_all(&positions_header)?;
    let mut full_positions = ColdWriter::create(&index_path.join(FULL_POSITIONS_FILE))?;
    let mut full_positions_header = Vec::with_capacity(HEADER_BYTES);
    write_header(
        &mut full_positions_header,
        FULL_POSITIONS_MAGIC,
        term_count_u32,
    );
    full_positions.write_all(&full_positions_header)?;
    let position_offsets_count = term_count
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
    let mut lexicon = Vec::with_capacity(term_count);
    let mut position_scratch = Vec::with_capacity(5);

    for path in shard_paths {
        let mut terms = read_posting_shard(&path)?.into_iter().collect::<Vec<_>>();
        terms.sort_unstable_by(|left, right| left.0.cmp(&right.0));
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
                .ok_or_else(|| {
                    Error::Corrupt("full positions file exceeds u32 length".to_string())
                })?;
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
                .ok_or_else(|| {
                    Error::Corrupt("full positions file exceeds u32 length".to_string())
                })?;
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
    }
    position_offsets.push(positions_position);
    full_position_offsets.push(full_positions_position);
    postings.finish()?;
    positions.seek(SeekFrom::Start(HEADER_BYTES as u64))?;
    for offset in position_offsets {
        positions.write_all(&offset.to_le_bytes())?;
    }
    positions.finish()?;
    full_positions.seek(SeekFrom::Start(HEADER_BYTES as u64))?;
    for offset in full_position_offsets {
        full_positions.write_all(&offset.to_le_bytes())?;
    }
    full_positions.finish()?;
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
    Ok(lexicon)
}

fn refine_posting_shards(directory: &Path) -> Result<Vec<PathBuf>> {
    refine_spool_shards::<SpoolPosting>(directory)
}

fn refine_bigram_shards(directory: &Path, maximum_bytes: u64) -> Result<Vec<PathBuf>> {
    refine_spool_shards_with_limit::<SpoolBigram>(directory, maximum_bytes)
}

fn refine_spool_shards<R: ShardRecord>(directory: &Path) -> Result<Vec<PathBuf>> {
    refine_spool_shards_with_limit::<R>(directory, MAX_POSTING_SHARD_BYTES)
}

fn refine_spool_shards_with_limit<R: ShardRecord>(
    directory: &Path,
    maximum_bytes: u64,
) -> Result<Vec<PathBuf>> {
    if maximum_bytes == 0 {
        return Err(Error::InvalidInput(
            "spool shard limit must be positive".to_string(),
        ));
    }
    let mut paths = Vec::new();
    let mut next_id = 0_u64;
    for shard in 0..POSTING_SHARDS {
        let path = directory.join(format!("{shard:03}.bin"));
        if path.exists() {
            refine_spool_shard::<R>(directory, path, 1, maximum_bytes, &mut next_id, &mut paths)?;
        }
    }
    Ok(paths)
}

fn refine_spool_shard<R: ShardRecord>(
    directory: &Path,
    path: PathBuf,
    depth: usize,
    maximum_bytes: u64,
    next_id: &mut u64,
    output_paths: &mut Vec<PathBuf>,
) -> Result<()> {
    let mut pending = vec![(path, depth)];
    while let Some((path, depth)) = pending.pop() {
        if fs::metadata(&path)?.len() <= maximum_bytes {
            output_paths.push(path);
            continue;
        }

        let mut input = BufReader::new(ColdReader::open(&path)?);
        let mut outputs = (0..257).map(|_| None).collect::<Vec<Option<ColdWriter>>>();
        let mut child_paths = vec![None; 257];
        let mut first_key = None::<Vec<u8>>;
        let mut common_prefix_length = usize::MAX;
        let mut all_same_key = true;
        while let Some(record) = R::read(&mut input)? {
            let record_key = record.key();
            if let Some(first_key) = first_key.as_deref() {
                let shared_prefix = depth.min(first_key.len()).min(record_key.len());
                common_prefix_length = common_prefix_length.min(
                    shared_prefix
                        + first_key[shared_prefix..]
                            .iter()
                            .zip(&record_key[shared_prefix..])
                            .take_while(|(left, right)| left == right)
                            .count(),
                );
                all_same_key &= first_key == record_key;
            } else {
                first_key = Some(record_key.to_vec());
            }
            let key = record_key
                .get(depth)
                .map_or(0, |byte| usize::from(*byte) + 1);
            if outputs[key].is_none() {
                let child = directory.join(format!("refined-{:08}.bin", *next_id));
                *next_id = next_id
                    .checked_add(1)
                    .ok_or_else(|| Error::Corrupt("posting shard id overflows".to_string()))?;
                outputs[key] = Some(ColdWriter::create_posting_shard(&child)?);
                child_paths[key] = Some(child);
            }
            record.write(outputs[key].as_mut().expect("refined shard exists"))?;
        }
        drop(input);
        for output in outputs.into_iter().flatten() {
            output.finish()?;
        }
        fs::remove_file(path)?;

        let mut children = child_paths.into_iter().flatten().collect::<Vec<_>>();
        if children.len() == 1 {
            let child = children.pop().expect("one refined child");
            if all_same_key {
                // A complete key cannot be partitioned further. The caller
                // streams this one oversized posting list without decoding it
                // into a corpus-sized allocation.
                output_paths.push(child);
            } else {
                if common_prefix_length <= depth {
                    return Err(Error::Corrupt(
                        "posting shard refinement did not advance".to_string(),
                    ));
                }
                pending.push((child, common_prefix_length));
            }
            continue;
        }

        // The stack is LIFO. Push in reverse byte order so completed leaves
        // retain the lexicographic ordering required by the index writers.
        for child in children.into_iter().rev() {
            pending.push((child, depth + 1));
        }
    }
    Ok(())
}

struct SpoolPosting {
    term: String,
    posting: BuildPosting,
    field: IndexedField,
}

struct SpoolBigram {
    key: Vec<u8>,
    id: u32,
    earliest_end_offset: u32,
    field: IndexedField,
}

trait ShardRecord: Sized {
    fn key(&self) -> &[u8];
    fn read(input: &mut dyn Read) -> Result<Option<Self>>;
    fn write(&self, output: &mut dyn Write) -> Result<()>;
}

impl ShardRecord for SpoolPosting {
    fn key(&self) -> &[u8] {
        self.term.as_bytes()
    }

    fn read(input: &mut dyn Read) -> Result<Option<Self>> {
        read_spool_posting(input)
    }

    fn write(&self, output: &mut dyn Write) -> Result<()> {
        write_spool_posting(output, &self.term, self.posting, self.field)
    }
}

impl ShardRecord for SpoolBigram {
    fn key(&self) -> &[u8] {
        &self.key
    }

    fn read(input: &mut dyn Read) -> Result<Option<Self>> {
        read_spool_bigram(input)
    }

    fn write(&self, output: &mut dyn Write) -> Result<()> {
        write_spool_bigram(
            output,
            &self.key,
            self.id,
            self.earliest_end_offset,
            self.field,
        )
    }
}

fn write_spool_posting(
    output: &mut (impl Write + ?Sized),
    term: &str,
    posting: BuildPosting,
    field: IndexedField,
) -> Result<()> {
    let term_bytes = term.as_bytes();
    let term_length = u16::try_from(term_bytes.len())
        .map_err(|_| Error::InvalidInput("term exceeds 65,535 bytes".to_string()))?;
    output.write_all(&term_length.to_le_bytes())?;
    output.write_all(term_bytes)?;
    output.write_all(&[field as u8])?;
    output.write_all(&posting.id.to_le_bytes())?;
    output.write_all(&posting.position_offset.to_le_bytes())?;
    output.write_all(&posting.position_length.to_le_bytes())?;
    output.write_all(&posting.frequency.to_le_bytes())?;
    Ok(())
}

fn read_spool_posting(input: &mut (impl Read + ?Sized)) -> Result<Option<SpoolPosting>> {
    let mut length_bytes = [0_u8; 2];
    if input.read(&mut length_bytes[..1])? == 0 {
        return Ok(None);
    }
    input.read_exact(&mut length_bytes[1..])?;
    let length = usize::from(u16::from_le_bytes(length_bytes));
    if length == 0 {
        return Err(Error::Corrupt(
            "posting shard contains an empty term".to_string(),
        ));
    }
    let mut term = vec![0_u8; length];
    input.read_exact(&mut term)?;
    let term = String::from_utf8(term)
        .map_err(|_| Error::Corrupt("posting shard term is not UTF-8".to_string()))?;
    let mut encoded = [0_u8; 13];
    input.read_exact(&mut encoded)?;
    let field = match encoded[0] {
        0 => IndexedField::Text,
        1 => IndexedField::Title,
        _ => {
            return Err(Error::Corrupt(
                "posting shard field discriminator is invalid".to_string(),
            ));
        }
    };
    Ok(Some(SpoolPosting {
        term,
        posting: BuildPosting {
            id: u32::from_le_bytes(encoded[1..5].try_into().expect("posting id bytes")),
            position_offset: u32::from_le_bytes(
                encoded[5..9].try_into().expect("posting offset bytes"),
            ),
            position_length: u16::from_le_bytes(
                encoded[9..11].try_into().expect("posting length bytes"),
            ),
            frequency: u16::from_le_bytes(
                encoded[11..13].try_into().expect("posting frequency bytes"),
            ),
        },
        field,
    }))
}

fn write_spool_bigram(
    output: &mut (impl Write + ?Sized),
    key: &[u8],
    id: u32,
    earliest_end_offset: u32,
    field: IndexedField,
) -> Result<()> {
    let key_length = u16::try_from(key.len())
        .map_err(|_| Error::InvalidInput("bigram key exceeds 65,535 bytes".to_string()))?;
    if earliest_end_offset == 0 {
        return Err(Error::InvalidInput(
            "bigram end offsets must be positive".to_string(),
        ));
    }
    output.write_all(&key_length.to_le_bytes())?;
    output.write_all(key)?;
    output.write_all(&[field as u8])?;
    output.write_all(&id.to_le_bytes())?;
    output.write_all(&earliest_end_offset.to_le_bytes())?;
    Ok(())
}

fn read_spool_bigram(input: &mut (impl Read + ?Sized)) -> Result<Option<SpoolBigram>> {
    let mut length_bytes = [0_u8; 2];
    if input.read(&mut length_bytes[..1])? == 0 {
        return Ok(None);
    }
    input.read_exact(&mut length_bytes[1..])?;
    let length = usize::from(u16::from_le_bytes(length_bytes));
    if length < 3 {
        return Err(Error::Corrupt(
            "bigram shard contains an invalid key".to_string(),
        ));
    }
    let mut key = vec![0_u8; length];
    input.read_exact(&mut key)?;
    let Some(separator) = key.iter().position(|byte| *byte == 0) else {
        return Err(Error::Corrupt(
            "bigram shard key is missing its separator".to_string(),
        ));
    };
    if separator == 0
        || separator + 1 == key.len()
        || key[separator + 1..].contains(&0)
        || std::str::from_utf8(&key[..separator]).is_err()
        || std::str::from_utf8(&key[separator + 1..]).is_err()
    {
        return Err(Error::Corrupt("bigram shard key is malformed".to_string()));
    }
    let mut encoded = [0_u8; 9];
    input.read_exact(&mut encoded)?;
    let field = match encoded[0] {
        0 => IndexedField::Text,
        1 => IndexedField::Title,
        _ => {
            return Err(Error::Corrupt(
                "bigram shard field discriminator is invalid".to_string(),
            ));
        }
    };
    let earliest_end_offset =
        u32::from_le_bytes(encoded[5..9].try_into().expect("bigram end offset bytes"));
    if earliest_end_offset == 0 {
        return Err(Error::Corrupt(
            "bigram shard end offset is zero".to_string(),
        ));
    }
    Ok(Some(SpoolBigram {
        key,
        id: u32::from_le_bytes(encoded[1..5].try_into().expect("bigram id bytes")),
        earliest_end_offset,
        field,
    }))
}

fn read_posting_shard(path: &Path) -> Result<HashMap<String, TermBuild>> {
    let mut input = BufReader::new(ColdReader::open(path)?);
    let mut terms = HashMap::<String, TermBuild>::new();
    while let Some(record) = read_spool_posting(&mut input)? {
        let lists = terms.entry(record.term).or_default();
        match record.field {
            IndexedField::Text => lists.text.push(record.posting),
            IndexedField::Title => lists.title.push(record.posting),
        }
    }
    Ok(terms)
}

#[derive(Default)]
struct BigramBuild {
    text: Vec<bigram::BigramPosting>,
    title: Vec<bigram::BigramPosting>,
}

struct ResolvedBigramKey<'a> {
    encoded: bigram::BigramKey,
    left: (u32, &'a LexiconBuildEntry),
    right: (u32, &'a LexiconBuildEntry),
}

fn write_bigram_indexes(
    index_path: &Path,
    spool_directory: &Path,
    lexicon: &[LexiconBuildEntry],
    documents: u32,
    videos: u32,
) -> Result<()> {
    write_bigram_indexes_with_threshold(
        index_path,
        spool_directory,
        lexicon,
        documents,
        videos,
        MIN_BIGRAM_TERM_DOCUMENTS,
        MAX_BIGRAM_SHARD_BYTES,
    )
}

fn write_bigram_indexes_with_threshold(
    index_path: &Path,
    spool_directory: &Path,
    lexicon: &[LexiconBuildEntry],
    documents: u32,
    videos: u32,
    minimum_term_documents: u32,
    maximum_shard_bytes: u64,
) -> Result<()> {
    let shard_paths = refine_bigram_shards(spool_directory, maximum_shard_bytes)?;
    let covered_terms = lexicon
        .iter()
        .enumerate()
        .filter_map(|(term, entry)| {
            (safe_document_frequency((term as u32, entry)) > minimum_term_documents)
                .then_some(term as u32)
        })
        .collect::<Vec<_>>();
    let mut text = bigram::BigramIndexWriter::create(
        &index_path.join(bigram::TEXT_BIGRAM_FILE),
        documents.max(1),
        &covered_terms,
    )?;
    let mut title = bigram::BigramIndexWriter::create(
        &index_path.join(bigram::TITLE_BIGRAM_FILE),
        videos.max(1),
        &covered_terms,
    )?;
    for path in shard_paths {
        if fs::metadata(&path)?.len() > maximum_shard_bytes {
            write_oversized_bigram_shard(
                &path,
                lexicon,
                minimum_term_documents,
                &mut text,
                &mut title,
            )?;
            continue;
        }
        let mut pairs = read_bigram_shard(&path)?.into_iter().collect::<Vec<_>>();
        pairs.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        for (key, mut postings) in pairs {
            let pair = resolve_bigram_key(lexicon, &key)?;
            if safe_document_frequency(pair.left) <= minimum_term_documents
                || safe_document_frequency(pair.right) <= minimum_term_documents
            {
                continue;
            }
            coalesce_bigram_postings(&mut postings.text)?;
            if !postings.text.is_empty() {
                text.append(pair.encoded, &postings.text)?;
            }
            coalesce_bigram_postings(&mut postings.title)?;
            if !postings.title.is_empty() {
                title.append(pair.encoded, &postings.title)?;
            }
        }
    }
    text.finish()?;
    title.finish()?;
    Ok(())
}

fn write_oversized_bigram_shard(
    path: &Path,
    lexicon: &[LexiconBuildEntry],
    minimum_term_documents: u32,
    text: &mut bigram::BigramIndexWriter,
    title: &mut bigram::BigramIndexWriter,
) -> Result<()> {
    let mut input = BufReader::new(ColdReader::open(path)?);
    let first = read_spool_bigram(&mut input)?
        .ok_or_else(|| Error::Corrupt("oversized bigram shard contains no records".to_string()))?;
    let expected_key = first.key;
    let pair = resolve_bigram_key(lexicon, &expected_key)?;
    let retained = safe_document_frequency(pair.left) > minimum_term_documents
        && safe_document_frequency(pair.right) > minimum_term_documents;

    // Refinement can no longer split an oversized leaf only when all records
    // have the same complete key. Read the leaf once per field so each posting
    // list is coalesced and encoded in document order without materializing a
    // corpus-sized Vec. The iterator also validates the exact-key invariant.
    let text_postings =
        CoalescedBigramShardPostings::open(path, expected_key.clone(), IndexedField::Text)?;
    if retained {
        text.append_iter(pair.encoded, text_postings)?;
    } else {
        for posting in text_postings {
            posting?;
        }
    }

    let title_postings =
        CoalescedBigramShardPostings::open(path, expected_key, IndexedField::Title)?;
    if retained {
        title.append_iter(pair.encoded, title_postings)?;
    } else {
        for posting in title_postings {
            posting?;
        }
    }
    Ok(())
}

struct CoalescedBigramShardPostings {
    input: BufReader<ColdReader>,
    expected_key: Vec<u8>,
    field: IndexedField,
    pending: Option<bigram::BigramPosting>,
    finished: bool,
}

impl CoalescedBigramShardPostings {
    fn open(path: &Path, expected_key: Vec<u8>, field: IndexedField) -> Result<Self> {
        Ok(Self {
            input: BufReader::new(ColdReader::open(path)?),
            expected_key,
            field,
            pending: None,
            finished: false,
        })
    }

    fn fail(&mut self, message: &str) -> Option<Result<bigram::BigramPosting>> {
        self.finished = true;
        Some(Err(Error::Corrupt(message.to_string())))
    }
}

impl Iterator for CoalescedBigramShardPostings {
    type Item = Result<bigram::BigramPosting>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        loop {
            let record = match read_spool_bigram(&mut self.input) {
                Ok(Some(record)) => record,
                Ok(None) => {
                    self.finished = true;
                    return self.pending.take().map(Ok);
                }
                Err(error) => {
                    self.finished = true;
                    return Some(Err(error));
                }
            };
            if record.key != self.expected_key {
                return self.fail("oversized bigram shard contains more than one key");
            }
            if record.field != self.field {
                continue;
            }
            let candidate = bigram::BigramPosting {
                document: record.id,
                earliest_end_offset: record.earliest_end_offset,
            };
            let Some(previous) = self.pending.as_mut() else {
                self.pending = Some(candidate);
                continue;
            };
            if candidate.document < previous.document {
                return self.fail("bigram spool identifiers are not monotonic");
            }
            if candidate.document == previous.document {
                previous.earliest_end_offset = previous
                    .earliest_end_offset
                    .min(candidate.earliest_end_offset);
                continue;
            }
            return Some(Ok(self
                .pending
                .replace(candidate)
                .expect("pending posting")));
        }
    }
}

fn read_bigram_shard(path: &Path) -> Result<HashMap<Vec<u8>, BigramBuild>> {
    let mut input = BufReader::new(ColdReader::open(path)?);
    let mut pairs = HashMap::<Vec<u8>, BigramBuild>::new();
    while let Some(record) = read_spool_bigram(&mut input)? {
        let postings = pairs.entry(record.key).or_default();
        let posting = bigram::BigramPosting {
            document: record.id,
            earliest_end_offset: record.earliest_end_offset,
        };
        match record.field {
            IndexedField::Text => postings.text.push(posting),
            IndexedField::Title => postings.title.push(posting),
        }
    }
    Ok(pairs)
}

fn resolve_bigram_key<'a>(
    lexicon: &'a [LexiconBuildEntry],
    key: &[u8],
) -> Result<ResolvedBigramKey<'a>> {
    let separator = key
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| Error::Corrupt("bigram key has no separator".to_string()))?;
    let left = resolve_bigram_term(lexicon, &key[..separator])?;
    let right = resolve_bigram_term(lexicon, &key[separator + 1..])?;
    Ok(ResolvedBigramKey {
        encoded: bigram::BigramKey::new(left.0, right.0),
        left,
        right,
    })
}

fn resolve_bigram_term<'a>(
    lexicon: &'a [LexiconBuildEntry],
    term: &[u8],
) -> Result<(u32, &'a LexiconBuildEntry)> {
    let index = lexicon
        .binary_search_by(|entry| entry.term.as_bytes().cmp(term))
        .map_err(|_| Error::Corrupt("bigram term is missing from the lexicon".to_string()))?;
    Ok((index as u32, &lexicon[index]))
}

fn safe_document_frequency(term: (u32, &LexiconBuildEntry)) -> u32 {
    term.1
        .text_frequency
        .saturating_add(term.1.fuzzy_title_frequency)
}

fn coalesce_bigram_postings(postings: &mut Vec<bigram::BigramPosting>) -> Result<()> {
    if postings.is_empty() {
        return Ok(());
    }
    let mut output = 1_usize;
    for input in 1..postings.len() {
        let previous = postings[output - 1];
        let candidate = postings[input];
        if candidate.document < previous.document {
            return Err(Error::Corrupt(
                "bigram spool identifiers are not monotonic".to_string(),
            ));
        }
        if candidate.document == previous.document {
            postings[output - 1].earliest_end_offset = previous
                .earliest_end_offset
                .min(candidate.earliest_end_offset);
            continue;
        }
        postings[output] = candidate;
        output += 1;
    }
    postings.truncate(output);
    Ok(())
}

fn write_position_lists(
    postings: &[BuildPosting],
    spool: &MappedFile,
    first_positions: &mut impl Write,
    remaining_positions: &mut impl Write,
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

    let mut output = ColdWriter::create(&index_path.join(LEXICON_FILE))?;
    let mut header = Vec::with_capacity(HEADER_BYTES);
    write_header(&mut header, LEXICON_MAGIC, entries.len() as u32);
    output.write_all(&header)?;
    for offset in offsets {
        output.write_all(&offset.to_le_bytes())?;
    }
    output.write_all(&entry_bytes)?;
    output.finish()?;
    Ok(())
}

fn token_counts(value: &str) -> Result<TokenCounts> {
    let mut counts = TokenCounts {
        terms: HashMap::new(),
        tails: Vec::new(),
    };
    let mut position = 0_u32;
    for_each_token(value, |token| {
        record_token(&mut counts, token, &mut position)
    })?;
    Ok(counts)
}

fn record_token(counts: &mut TokenCounts, token: String, position: &mut u32) -> Result<()> {
    match counts.terms.entry(token) {
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

fn for_each_token(value: &str, mut visit: impl FnMut(String) -> Result<()>) -> Result<()> {
    let mut token = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            token.extend(character.to_lowercase());
        } else if !token.is_empty() {
            visit(std::mem::take(&mut token))?;
        }
    }
    if !token.is_empty() {
        visit(token)?;
    }
    Ok(())
}

fn append_value_bigrams(
    value: &str,
    id: u32,
    field: IndexedField,
    output: &mut ShardSpoolWriter,
    covered_terms: &HashSet<&str>,
) -> Result<()> {
    let mut previous = None::<String>;
    let mut position = 0_u32;
    for_each_token(value, |token| {
        if let Some(left) = previous.as_deref()
            && covered_terms.contains(left)
            && covered_terms.contains(token.as_str())
        {
            output.append_bigram(left, &token, id, position, field)?;
        }
        previous = Some(token);
        position = position
            .checked_add(1)
            .ok_or_else(|| Error::InvalidInput("one field exceeds 4 billion tokens".to_string()))?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn posting() -> BuildPosting {
        BuildPosting {
            id: 7,
            position_offset: 11,
            position_length: 13,
            frequency: 17,
        }
    }

    #[test]
    fn bigram_prefilter_preserves_adjacency_and_original_offsets() {
        let directory = tempdir().unwrap();
        let mut spool = ShardSpoolWriter::create(directory.path()).unwrap();
        let covered = HashSet::from(["the", "and"]);
        append_value_bigrams(
            "THE rare and the and",
            7,
            IndexedField::Text,
            &mut spool,
            &covered,
        )
        .unwrap();
        spool.finish().unwrap();
        let mut records = Vec::new();
        for path in fs::read_dir(directory.path()).unwrap() {
            let mut input = BufReader::new(File::open(path.unwrap().path()).unwrap());
            while let Some(record) = read_spool_bigram(&mut input).unwrap() {
                assert_eq!(record.id, 7);
                records.push((record.key, record.earliest_end_offset));
            }
        }
        records.sort_unstable();
        assert_eq!(
            records,
            vec![(b"and\0the".to_vec(), 3), (b"the\0and".to_vec(), 4)]
        );
    }

    #[test]
    fn spool_field_discriminator_round_trips_and_rejects_unknown_values() {
        for field in [IndexedField::Text, IndexedField::Title] {
            let mut bytes = Vec::new();
            write_spool_posting(&mut bytes, "term", posting(), field).unwrap();
            let decoded = read_spool_posting(&mut Cursor::new(&bytes))
                .unwrap()
                .unwrap();
            assert_eq!(decoded.term, "term");
            assert_eq!(decoded.field, field);
            assert_eq!(decoded.posting.id, 7);
            assert_eq!(decoded.posting.position_offset, 11);
            assert_eq!(decoded.posting.position_length, 13);
            assert_eq!(decoded.posting.frequency, 17);
        }

        let mut invalid = Vec::new();
        write_spool_posting(&mut invalid, "term", posting(), IndexedField::Text).unwrap();
        invalid[2 + "term".len()] = 2;
        assert!(matches!(
            read_spool_posting(&mut Cursor::new(invalid)),
            Err(Error::Corrupt(_))
        ));
    }

    fn lexicon_entry(term: &str, frequency: u32) -> LexiconBuildEntry {
        LexiconBuildEntry {
            term: term.to_string(),
            text_offset: 0,
            text_length: 0,
            text_frequency: frequency,
            title_offset: 0,
            title_length: 0,
            title_frequency: 0,
            fuzzy_title_frequency: 0,
        }
    }

    #[test]
    fn selective_bigram_merge_keeps_earliest_offsets_for_text_and_titles() {
        let root = tempdir().unwrap();
        let spool = SpoolDirectory::create(root.path().join("spool")).unwrap();
        let mut writer = ShardSpoolWriter::create(spool.path()).unwrap();
        for (id, offset, field) in [
            (3, 5, IndexedField::Text),
            (3, 2, IndexedField::Text),
            (8, 4, IndexedField::Text),
            (1, 3, IndexedField::Title),
        ] {
            writer
                .append_bigram("alpha", "beta", id, offset, field)
                .unwrap();
        }
        writer.finish().unwrap();
        let output = root.path().join("index");
        fs::create_dir(&output).unwrap();
        let lexicon = [lexicon_entry("alpha", 1), lexicon_entry("beta", 1)];
        write_bigram_indexes_with_threshold(
            &output,
            spool.path(),
            &lexicon,
            16,
            4,
            0,
            MAX_BIGRAM_SHARD_BYTES,
        )
        .unwrap();

        let text = bigram::BigramIndex::open(&output.join(bigram::TEXT_BIGRAM_FILE)).unwrap();
        assert_eq!(
            text.lookup(bigram::BigramKey::new(0, 1)).unwrap(),
            [
                bigram::BigramPosting {
                    document: 3,
                    earliest_end_offset: 2,
                },
                bigram::BigramPosting {
                    document: 8,
                    earliest_end_offset: 4,
                },
            ]
        );
        let title = bigram::BigramIndex::open(&output.join(bigram::TITLE_BIGRAM_FILE)).unwrap();
        assert_eq!(
            title.lookup(bigram::BigramKey::new(0, 1)).unwrap(),
            [bigram::BigramPosting {
                document: 1,
                earliest_end_offset: 3,
            }]
        );
    }

    #[test]
    fn recursive_pair_runs_respect_the_configured_memory_bound() {
        let root = tempdir().unwrap();
        let spool = SpoolDirectory::create(root.path().join("spool")).unwrap();
        let mut writer = ShardSpoolWriter::create(spool.path()).unwrap();
        for index in 0..100_u32 {
            writer
                .append_bigram(
                    &format!("left{index:03}"),
                    "right",
                    index,
                    1,
                    IndexedField::Text,
                )
                .unwrap();
        }
        writer.finish().unwrap();
        let limit = 128_u64;
        let paths = refine_spool_shards_with_limit::<SpoolBigram>(spool.path(), limit).unwrap();
        assert!(paths.len() > 1);
        assert!(
            paths
                .iter()
                .all(|path| fs::metadata(path).unwrap().len() <= limit)
        );
    }

    #[test]
    fn oversized_single_pair_streams_and_coalesces_without_a_decoded_run() {
        let root = tempdir().unwrap();
        let spool = SpoolDirectory::create(root.path().join("spool")).unwrap();
        let mut writer = ShardSpoolWriter::create(spool.path()).unwrap();
        for id in 0..100_u32 {
            writer
                .append_bigram("alpha", "beta", id, 9, IndexedField::Text)
                .unwrap();
            if id < 20 {
                writer
                    .append_bigram("alpha", "beta", id, 7, IndexedField::Title)
                    .unwrap();
            }
            if id == 42 {
                writer
                    .append_bigram("alpha", "beta", id, 2, IndexedField::Text)
                    .unwrap();
            }
        }
        writer.finish().unwrap();

        let output = root.path().join("index");
        fs::create_dir(&output).unwrap();
        let lexicon = [lexicon_entry("alpha", 100), lexicon_entry("beta", 100)];
        let shard_limit = 128;
        write_bigram_indexes_with_threshold(
            &output,
            spool.path(),
            &lexicon,
            100,
            20,
            0,
            shard_limit,
        )
        .unwrap();

        let text = bigram::BigramIndex::open(&output.join(bigram::TEXT_BIGRAM_FILE)).unwrap();
        let postings = text.lookup(bigram::BigramKey::new(0, 1)).unwrap();
        assert_eq!(postings.len(), 100);
        assert_eq!(postings[42].earliest_end_offset, 2);
        let title = bigram::BigramIndex::open(&output.join(bigram::TITLE_BIGRAM_FILE)).unwrap();
        assert_eq!(
            title.lookup(bigram::BigramKey::new(0, 1)).unwrap().len(),
            20
        );
    }

    #[test]
    fn oversized_long_pairs_skip_common_prefixes_without_recursive_rewrites() {
        let root = tempdir().unwrap();
        let spool = SpoolDirectory::create(root.path().join("spool")).unwrap();
        let mut writer = ShardSpoolWriter::create(spool.path()).unwrap();
        let common = "a".repeat(30_000);
        let left = [format!("{common}x"), format!("{common}y")];
        for id in 0..4_u32 {
            for term in &left {
                writer
                    .append_bigram(term, "beta", id, 1, IndexedField::Text)
                    .unwrap();
            }
        }
        writer.finish().unwrap();

        let paths = refine_spool_shards_with_limit::<SpoolBigram>(spool.path(), 128).unwrap();
        assert_eq!(paths.len(), 2);
        for (path, term) in paths.iter().zip(&left) {
            assert!(fs::metadata(path).unwrap().len() > 128);
            assert_eq!(
                CoalescedBigramShardPostings::open(
                    path,
                    format!("{term}\0beta").into_bytes(),
                    IndexedField::Text,
                )
                .unwrap()
                .collect::<Result<Vec<_>>>()
                .unwrap()
                .len(),
                4
            );
        }
    }

    #[test]
    fn spool_directories_clean_stale_and_failed_build_state() {
        let root = tempdir().unwrap();
        let path = root.path().join("spool");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("stale"), b"old").unwrap();
        {
            let spool = SpoolDirectory::create(path.clone()).unwrap();
            assert!(!spool.path().join("stale").exists());
            fs::write(spool.path().join("partial"), b"new").unwrap();
        }
        assert!(!path.exists());
    }
}
