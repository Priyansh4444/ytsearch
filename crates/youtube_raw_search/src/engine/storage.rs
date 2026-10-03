use super::*;

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
        skips: Option<Arc<[skips::SkipEntry]>>,
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
            skips,
        }
    }

    pub(super) fn skip_entry(&self) -> skips::SkipEntry {
        skips::SkipEntry {
            previous_id: self.previous_id,
            decoded: self.decoded,
            posting_cursor: self.posting_cursor,
            first_cursor: self.first_cursor,
            remaining_cursor: self.remaining_positions.offset(),
        }
    }

    /// Jumps to the last checkpoint whose postings all precede `target`.
    fn seek_before(&mut self, target: u32) {
        let Some(skips) = &self.skips else {
            return;
        };
        let index = skips.partition_point(|entry| entry.previous_id < target);
        let Some(entry) = index.checked_sub(1).map(|index| skips[index]) else {
            return;
        };
        if entry.decoded <= self.decoded {
            return;
        }
        self.previous_id = entry.previous_id;
        self.decoded = entry.decoded;
        self.posting_cursor = entry.posting_cursor;
        self.first_cursor = entry.first_cursor;
        self.remaining_positions.seek(entry.remaining_cursor);
    }

    pub(super) fn next_at_least(
        &mut self,
        target: u32,
        positions: &mut Vec<u32>,
    ) -> Result<Option<u32>> {
        self.seek_before(target);
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

    #[inline]
    pub(super) fn document_tokens(&self, id: u32) -> Result<u32> {
        if id as usize >= self.documents {
            return Err(Error::Corrupt(format!("document {id} is out of bounds")));
        }
        read_u32(
            self.file.bytes(),
            METADATA_PREFIX_BYTES + id as usize * DOCUMENT_RECORD_BYTES + 16,
        )
    }

    /// Starts loading `id`'s search fields into cache without waiting for
    /// them. Scans that know their next documents call this a few documents
    /// ahead, since each record read is otherwise a cache miss.
    #[inline]
    pub(super) fn prefetch_search_document(&self, id: u32) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: SSE is part of the x86-64 baseline, and a prefetch never
        // dereferences or faults, even for an address outside the mapping.
        unsafe {
            core::arch::x86_64::_mm_prefetch::<{ core::arch::x86_64::_MM_HINT_T0 }>(
                self.file
                    .bytes()
                    .as_ptr()
                    .wrapping_add(METADATA_PREFIX_BYTES + id as usize * DOCUMENT_RECORD_BYTES + 12)
                    .cast(),
            );
        }
    }

    #[inline]
    pub(super) fn search_document(&self, id: u32) -> Result<SearchDocumentRecord> {
        if id as usize >= self.documents {
            return Err(Error::Corrupt(format!("document {id} is out of bounds")));
        }
        let offset = METADATA_PREFIX_BYTES + id as usize * DOCUMENT_RECORD_BYTES + 12;
        let bytes =
            self.file.bytes().get(offset..offset + 12).ok_or_else(|| {
                Error::Corrupt("document search metadata is truncated".to_string())
            })?;
        Ok(SearchDocumentRecord {
            video: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            tokens: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            t_start_ms: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
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
        // Schemas 5 and 8 are retired (no production generation used them).
        // Schema 6 remains the immutable base index; published generations
        // are schema 7, or schema 9 (7 plus accelerators).
        if !matches!(manifest.schema_version, 6 | 7 | 9) {
            return Err(Error::Corrupt(format!(
                "unsupported schema version {}; this reader opens schemas 6, 7, and 9",
                manifest.schema_version
            )));
        }
        if matches!(validation, ManifestValidation::VerifiedByBuilder)
            && !matches!(manifest.schema_version, 7 | 9)
        {
            return Err(Error::Corrupt(
                "newly built index does not use schema version 7 or 9".to_string(),
            ));
        }
        {
            // Builder-verified opens still check the complete file list and all
            // byte lengths. Only the redundant content reads and BLAKE3 work
            // are skipped.
            validate_manifest_files(index_path, &manifest, validation)?;
        }
        let metadata_started = std::time::Instant::now();
        tracing::info!(
            schema_version = manifest.schema_version,
            "opening raw generation metadata and indexes"
        );
        let videos: Vec<VideoMeta> =
            serde_json::from_reader(BufReader::new(File::open(index_path.join(VIDEOS_FILE))?))?;
        // Scans treat a video whose span equals its length as one contiguous
        // run, which holds only if its documents ascend strictly.
        if !videos
            .iter()
            .all(|video| video.documents.is_sorted_by(|left, right| left < right))
        {
            return Err(Error::Corrupt(
                "video documents must be strictly increasing".to_string(),
            ));
        }
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
        let full_positions = PositionIndex::open(
            &index_path.join(FULL_POSITIONS_FILE),
            FULL_POSITIONS_MAGIC,
            lexicon.count,
        )?;
        let documents = MappedFile::open(&index_path.join(DOCUMENTS_FILE), DOCUMENTS_MAGIC)?;
        // Responses read a few ~5 KB compressed blocks at scattered offsets.
        documents.advise_random();
        let metadata = DocumentTable::open(&index_path.join(METADATA_FILE))?;
        let (text_bigrams, title_bigrams) = if manifest.schema_version >= 7 {
            let text = bigram::BigramIndex::open(&index_path.join(bigram::TEXT_BIGRAM_FILE))?;
            let title = bigram::BigramIndex::open(&index_path.join(bigram::TITLE_BIGRAM_FILE))?;
            text.validate_term_universe(lexicon.count)?;
            title.validate_term_universe(lexicon.count)?;
            if text.universe() != manifest.documents.max(1)
                || title.universe() != manifest.videos.max(1)
            {
                return Err(Error::Corrupt(
                    "ordered-bigram universes do not match the manifest".to_string(),
                ));
            }
            if !text.has_same_coverage(&title) {
                return Err(Error::Corrupt(
                    "ordered-bigram field coverage does not match".to_string(),
                ));
            }
            (Some(text), Some(title))
        } else {
            (None, None)
        };
        let (accelerators, bitmaps, plan_bounds) = if manifest.schema_version >= 9 {
            (
                Some(accelerators::Accelerators::open(
                    &index_path.join(accelerators::ACCELERATORS_FILE),
                    lexicon.count,
                )?),
                Some(accelerators::Bitmaps::open(
                    &index_path.join(accelerators::BITMAPS_FILE),
                    lexicon.count,
                    manifest.documents,
                    manifest.videos,
                )?),
                Some(plan_bounds::PlanBounds::open(
                    &index_path.join(plan_bounds::PLAN_BOUNDS_FILE),
                    lexicon.count,
                    manifest.documents,
                )?),
            )
        } else {
            (None, None, None)
        };
        tracing::info!(
            elapsed_ms = metadata_started.elapsed().as_millis() as u64,
            "opened raw generation metadata and indexes"
        );
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
        let engine = Self {
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
                accelerators,
                bitmaps,
                plan_bounds,
                documents,
                metadata,
                partition: std::sync::OnceLock::new(),
                skips: skips::SkipDirectory::default(),
                index_bytes: directory_size(index_path)?,
            }),
        };
        // A startup open has just checksummed and cached the scanned files,
        // so warming is cheap and serving need not wait. A publication open
        // is warmed by the publisher before activation instead.
        if matches!(validation, ManifestValidation::Full) {
            warm_in_background(&engine.inner);
        }
        Ok(engine)
    }

    /// Reads a generation's scanned files into memory and plans its scan
    /// partitions. Publication runs this (via `warm_within`) before activation: a published
    /// generation arrives as new, cold files, and swapping to it first made
    /// the next searches fault from the volume (a 32 s `fire` on the canary)
    /// while the previous, warm generation could still have served them.
    pub(crate) fn warm(&self) {
        let started = std::time::Instant::now();
        prefetch_hot_mappings(&self.inner);
        partition::DocumentPartition::plan_now(&self.inner);
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "warmed raw generation"
        );
    }
}

/// Longest an activation waits for warm-up. It must leave headroom under the
/// publisher's admin request timeout, which activation shares.
const ACTIVATION_WARM_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);
const _: () = assert!(
    ACTIVATION_WARM_BUDGET.as_secs() + 5 <= youtube_index_store::ADMIN_REQUEST_TIMEOUT.as_secs(),
    "activation warm-up must finish well inside the admin request timeout"
);

/// An engine whose scanned files were read ahead and whose scan partitions
/// were planned before it serves. `ReloadableEngine::replace` accepts only
/// this, so no activation path can swap in a cold generation.
pub struct WarmEngine(RawSearchEngine);

impl WarmEngine {
    pub(crate) fn into_engine(self) -> RawSearchEngine {
        self.0
    }
}

impl RawSearchEngine {
    /// Warms for activation within `ACTIVATION_WARM_BUDGET`; anything left
    /// finishes in the background after the swap.
    pub(crate) fn warmed_for_activation(self) -> WarmEngine {
        self.warm_within(ACTIVATION_WARM_BUDGET);
        WarmEngine(self)
    }

    /// Warms, but waits at most `budget`. Whatever is left finishes in the
    /// background.
    fn warm_within(&self, budget: std::time::Duration) {
        let (done, finished) = std::sync::mpsc::channel();
        let engine = self.clone();
        let spawned = std::thread::Builder::new()
            .name("raw-warm".to_string())
            .spawn(move || {
                engine.warm();
                let _ = done.send(());
            });
        match spawned {
            Ok(_) => {
                if finished.recv_timeout(budget).is_err() {
                    tracing::warn!(
                        budget_ms = budget.as_millis() as u64,
                        "raw generation still warming at activation"
                    );
                }
            }
            Err(error) => tracing::warn!(%error, "raw generation warm-up did not start"),
        }
    }
}

/// Warms a generation opened at startup without holding the health check.
/// The thread starts from a weak reference, so a generation retired before it
/// runs is not pinned. Until planning finishes, scans run as one exact range.
fn warm_in_background(inner: &Arc<EngineInner>) {
    let inner = Arc::downgrade(inner);
    let spawned = std::thread::Builder::new()
        .name("raw-warm".to_string())
        .spawn(move || {
            if let Some(inner) = inner.upgrade() {
                RawSearchEngine { inner }.warm();
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "raw generation warm-up did not start");
    }
}

fn prefetch_hot_mappings(inner: &EngineInner) {
    let started = std::time::Instant::now();
    let files = [
        &inner.lexicon.file,
        &inner.fuzzy.file,
        &inner.postings,
        &inner.positions.file,
        &inner.full_positions.file,
        &inner.metadata.file,
    ];
    for file in files {
        file.prefetch();
    }
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        "prefetched raw generation mappings"
    );
}
