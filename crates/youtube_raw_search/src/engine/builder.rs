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
                youtuber_id: source.youtuber_id.to_ascii_lowercase(),
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
