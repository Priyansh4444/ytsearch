use super::*;

impl Lexicon {
    pub(super) fn open(path: &Path) -> Result<Self> {
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

    pub(super) fn lookup(&self, term: &str) -> Result<Option<TermEntry>> {
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

    pub(super) fn entry_at(&self, index: usize) -> Result<(&str, TermEntry)> {
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
    pub(super) fn open(path: &Path, magic: &[u8; 8], expected_terms: usize) -> Result<Self> {
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

    pub(super) fn range(&self, term: u32, field: IndexedField) -> Result<&[u8]> {
        if term as usize >= self.terms {
            return Err(Error::Corrupt(
                "position term is outside the lexicon".to_string(),
            ));
        }
        let index = term as usize * 2 + field.archive_index();
        let start = read_u32(self.file.bytes(), HEADER_BYTES + index * 4)? as usize;
        let end = read_u32(self.file.bytes(), HEADER_BYTES + (index + 1) * 4)? as usize;
        self.file
            .bytes()
            .get(self.data_start + start..self.data_start + end)
            .ok_or_else(|| Error::Corrupt("position range extends beyond file".to_string()))
    }
}

impl<'a> PositionedPostingCursor<'a> {
    pub(super) fn new(
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

    pub(super) fn next_at_least(
        &mut self,
        target: u32,
        positions: &mut Vec<u32>,
    ) -> Result<Option<u32>> {
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
    pub(super) fn advance_to(&mut self, target: u32) -> Result<()> {
        self.id = self.postings.next_at_least(target, &mut self.positions)?;
        Ok(())
    }
}

impl DocumentTable {
    pub(super) fn open(path: &Path) -> Result<Self> {
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

    pub(super) fn document(&self, id: u32) -> Result<DocumentRecord> {
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

    pub(super) fn block(&self, id: u32) -> Result<BlockRecord> {
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

impl RawSearchEngine {
    pub fn open(index_path: &Path) -> Result<Self> {
        Self::open_with_validation(index_path, ManifestValidation::Full)
    }

    pub(super) fn open_newly_built(index_path: &Path) -> Result<Self> {
        Self::open_with_validation(index_path, ManifestValidation::VerifiedByBuilder)
    }

    pub(crate) fn open_verified_transfer(index_path: &Path) -> Result<Self> {
        Self::open_with_validation(index_path, ManifestValidation::VerifiedByBuilder)
    }

    fn open_with_validation(index_path: &Path, validation: ManifestValidation) -> Result<Self> {
        let manifest: Manifest =
            serde_json::from_reader(BufReader::new(File::open(index_path.join(MANIFEST_FILE))?))?;
        if !matches!(manifest.schema_version, 5..=7) {
            return Err(Error::Corrupt(format!(
                "unsupported schema version {}",
                manifest.schema_version
            )));
        }
        if matches!(validation, ManifestValidation::VerifiedByBuilder)
            && manifest.schema_version != 7
        {
            return Err(Error::Corrupt(
                "newly built index does not use schema version 7".to_string(),
            ));
        }
        if manifest.schema_version >= 6 {
            // Builder-verified opens still check the complete file list and all
            // byte lengths. Only the redundant content reads and BLAKE3 work
            // are skipped.
            validate_manifest_files(index_path, &manifest, validation)?;
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
        let (text_bigrams, title_bigrams) = if manifest.schema_version == 7 {
            let text = bigram::BigramIndex::open(&index_path.join(bigram::TEXT_BIGRAM_FILE))?;
            let title = bigram::BigramIndex::open(&index_path.join(bigram::TITLE_BIGRAM_FILE))?;
            if text.universe() != manifest.documents.max(1)
                || title.universe() != manifest.videos.max(1)
            {
                return Err(Error::Corrupt(
                    "ordered-bigram universes do not match the manifest".to_string(),
                ));
            }
            (Some(text), Some(title))
        } else {
            (None, None)
        };
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
                text_bigrams,
                title_bigrams,
                documents,
                metadata,
                index_bytes: directory_size(index_path)?,
            }),
        })
    }
}
