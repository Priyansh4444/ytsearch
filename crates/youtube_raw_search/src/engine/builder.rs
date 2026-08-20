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

struct PostingShardWriter {
    directory: PathBuf,
    outputs: Vec<Option<ColdWriter>>,
}

impl PostingShardWriter {
    fn create(directory: PathBuf) -> Result<Self> {
        fs::create_dir_all(&directory)?;
        Ok(Self {
            directory,
            outputs: (0..POSTING_SHARDS).map(|_| None).collect(),
        })
    }

    fn append(&mut self, term: &str, posting: BuildPosting, title: bool) -> Result<()> {
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
            title,
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
    postings: &mut PostingShardWriter,
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
        postings.append(&term, posting, title)?;
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

    let posting_spool_path = index_path.join(POSTING_SPOOL_DIRECTORY);
    let mut posting_spool = PostingShardWriter::create(posting_spool_path.clone())?;
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
                &mut posting_spool,
                title_counts,
                video,
                true,
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
            &mut posting_spool,
            text_counts,
            document,
            false,
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
    posting_spool.finish()?;

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
    fs::remove_file(&document_record_spool_path)?;
    let position_spool = MappedFile::open(&position_spool_path, POSITION_SPOOL_MAGIC)?;
    let term_count = write_postings(index_path, &posting_spool_path, &videos, &position_spool)?;
    drop(position_spool);
    fs::remove_file(&position_spool_path)?;
    fs::remove_dir_all(&posting_spool_path)?;

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
    let mut manifest_output = ColdWriter::create(&index_path.join(MANIFEST_FILE))?;
    serde_json::to_writer_pretty(&mut manifest_output, &manifest)?;
    manifest_output.finish()?;

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
    output: &mut (impl Write + Seek),
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

fn write_postings(
    index_path: &Path,
    posting_spool_directory: &Path,
    videos: &[VideoMeta],
    position_spool: &MappedFile,
) -> Result<usize> {
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
    Ok(lexicon.len())
}

fn refine_posting_shards(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut next_id = 0_u64;
    for shard in 0..POSTING_SHARDS {
        let path = directory.join(format!("{shard:03}.bin"));
        if path.exists() {
            refine_posting_shard(directory, path, 1, &mut next_id, &mut paths)?;
        }
    }
    Ok(paths)
}

fn refine_posting_shard(
    directory: &Path,
    path: PathBuf,
    depth: usize,
    next_id: &mut u64,
    output_paths: &mut Vec<PathBuf>,
) -> Result<()> {
    if fs::metadata(&path)?.len() <= MAX_POSTING_SHARD_BYTES {
        output_paths.push(path);
        return Ok(());
    }

    let mut input = BufReader::new(ColdReader::open(&path)?);
    let mut outputs = (0..257).map(|_| None).collect::<Vec<Option<ColdWriter>>>();
    let mut child_paths = vec![None; 257];
    let mut can_refine = false;
    while let Some(record) = read_spool_posting(&mut input)? {
        let key = record
            .term
            .as_bytes()
            .get(depth)
            .map_or(0, |byte| usize::from(*byte) + 1);
        can_refine |= record.term.len() > depth;
        if outputs[key].is_none() {
            let child = directory.join(format!("refined-{:08}.bin", *next_id));
            *next_id = next_id
                .checked_add(1)
                .ok_or_else(|| Error::Corrupt("posting shard id overflows".to_string()))?;
            outputs[key] = Some(ColdWriter::create_posting_shard(&child)?);
            child_paths[key] = Some(child);
        }
        write_spool_posting(
            outputs[key].as_mut().expect("refined shard exists"),
            &record.term,
            record.posting,
            record.title,
        )?;
    }
    for output in outputs.into_iter().flatten() {
        output.finish()?;
    }
    fs::remove_file(path)?;

    let populated = child_paths.iter().flatten().count();
    for child in child_paths.into_iter().flatten() {
        if populated == 1 && !can_refine {
            output_paths.push(child);
        } else {
            refine_posting_shard(directory, child, depth + 1, next_id, output_paths)?;
        }
    }
    Ok(())
}

struct SpoolPosting {
    term: String,
    posting: BuildPosting,
    title: bool,
}

fn write_spool_posting(
    output: &mut impl Write,
    term: &str,
    posting: BuildPosting,
    title: bool,
) -> Result<()> {
    let term_bytes = term.as_bytes();
    let term_length = u16::try_from(term_bytes.len())
        .map_err(|_| Error::InvalidInput("term exceeds 65,535 bytes".to_string()))?;
    output.write_all(&term_length.to_le_bytes())?;
    output.write_all(term_bytes)?;
    output.write_all(&[u8::from(title)])?;
    output.write_all(&posting.id.to_le_bytes())?;
    output.write_all(&posting.position_offset.to_le_bytes())?;
    output.write_all(&posting.position_length.to_le_bytes())?;
    output.write_all(&posting.frequency.to_le_bytes())?;
    Ok(())
}

fn read_spool_posting(input: &mut impl Read) -> Result<Option<SpoolPosting>> {
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
    let title = match encoded[0] {
        0 => false,
        1 => true,
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
        title,
    }))
}

fn read_posting_shard(path: &Path) -> Result<HashMap<String, TermBuild>> {
    let mut input = BufReader::new(ColdReader::open(path)?);
    let mut terms = HashMap::<String, TermBuild>::new();
    while let Some(record) = read_spool_posting(&mut input)? {
        let lists = terms.entry(record.term).or_default();
        if record.title {
            lists.title.push(record.posting);
        } else {
            lists.text.push(record.posting);
        }
    }
    Ok(terms)
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
