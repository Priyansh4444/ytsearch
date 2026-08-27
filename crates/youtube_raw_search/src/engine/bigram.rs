use super::*;

const BIGRAM_MAGIC: &[u8; 8] = b"RAWBIG01";
const LEGACY_BIGRAM_VERSION: u32 = 1;
const LEGACY_BIGRAM_HEADER_BYTES: usize = 40;
const BIGRAM_VERSION: u32 = 2;
const BIGRAM_HEADER_BYTES: usize = 48;
const BIGRAM_ENTRY_BYTES: usize = 24;
pub(super) const TEXT_BIGRAM_FILE: &str = "bigrams.text.bin";
pub(super) const TITLE_BIGRAM_FILE: &str = "bigrams.title.bin";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BigramKey(u64);

impl BigramKey {
    pub(super) const fn new(left_term: u32, right_term: u32) -> Self {
        Self((left_term as u64) << 32 | right_term as u64)
    }

    const fn encoded(self) -> u64 {
        self.0
    }

    const fn left(self) -> u32 {
        (self.0 >> 32) as u32
    }

    const fn right(self) -> u32 {
        self.0 as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BigramPosting {
    pub(super) document: u32,
    pub(super) earliest_end_offset: u32,
}

#[cfg(test)]
pub(super) struct BigramWriteEntry<'a> {
    pub(super) key: BigramKey,
    pub(super) postings: &'a [BigramPosting],
}

#[derive(Clone, Copy)]
struct BigramEntry {
    key: BigramKey,
    offset: u64,
    length: u32,
    count: u32,
}

const _: () = assert!(std::mem::size_of::<BigramEntry>() == BIGRAM_ENTRY_BYTES);

pub(super) struct BigramIndex {
    file: MappedFile,
    count: usize,
    postings_start: usize,
    table_start: usize,
    coverage_start: usize,
    coverage_count: usize,
    coverage_authoritative: bool,
    universe: u32,
}

impl BigramIndex {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let file = MappedFile::open(path, BIGRAM_MAGIC)?;
        let bytes = file.bytes();
        if bytes.len() < LEGACY_BIGRAM_HEADER_BYTES {
            return Err(Error::Corrupt(
                "bigram index is shorter than its header".to_string(),
            ));
        }
        let version = read_u32(bytes, 8)?;
        let postings_start = match version {
            LEGACY_BIGRAM_VERSION => LEGACY_BIGRAM_HEADER_BYTES,
            BIGRAM_VERSION => BIGRAM_HEADER_BYTES,
            _ => {
                return Err(Error::Corrupt(format!(
                    "unsupported bigram index version {version}"
                )));
            }
        };
        if bytes.len() < postings_start {
            return Err(Error::Corrupt(
                "bigram index is shorter than its versioned header".to_string(),
            ));
        }
        let count = read_u32(bytes, 12)? as usize;
        let table_bytes = count
            .checked_mul(BIGRAM_ENTRY_BYTES)
            .ok_or_else(|| Error::Corrupt("bigram entry table length overflows".to_string()))?;
        let coverage_count = if version == BIGRAM_VERSION {
            read_u32(bytes, 40)? as usize
        } else {
            0
        };
        if version == BIGRAM_VERSION && read_u32(bytes, 44)? != 0 {
            return Err(Error::Corrupt(
                "bigram reserved header bytes must be zero".to_string(),
            ));
        }
        let coverage_bytes = coverage_count
            .checked_mul(std::mem::size_of::<u32>())
            .ok_or_else(|| Error::Corrupt("bigram coverage length overflows".to_string()))?;
        let table_start = usize::try_from(read_u64(bytes, 16)?)
            .map_err(|_| Error::Corrupt("bigram table offset exceeds usize".to_string()))?;
        if table_start < postings_start {
            return Err(Error::Corrupt(
                "bigram entry table overlaps the header".to_string(),
            ));
        }
        let declared_length = usize::try_from(read_u64(bytes, 24)?)
            .map_err(|_| Error::Corrupt("bigram file length exceeds usize".to_string()))?;
        if declared_length != bytes.len() {
            return Err(Error::Corrupt(
                "bigram file length does not match its header".to_string(),
            ));
        }
        let coverage_start = table_start
            .checked_add(table_bytes)
            .ok_or_else(|| Error::Corrupt("bigram entry table boundary overflows".to_string()))?;
        let expected_length = coverage_start
            .checked_add(coverage_bytes)
            .ok_or_else(|| Error::Corrupt("bigram coverage boundary overflows".to_string()))?;
        if expected_length != declared_length {
            return Err(Error::Corrupt(
                "bigram entry table and coverage must occupy the end of the file".to_string(),
            ));
        }
        let universe = read_u64(bytes, 32)?;
        if universe == 0 || universe > u64::from(u32::MAX) {
            return Err(Error::Corrupt(
                "bigram universe must fit in a nonzero u32".to_string(),
            ));
        }
        let universe = universe as u32;

        let mut previous_key = None;
        let mut expected_offset = 0_u64;
        for index in 0..count {
            let entry = read_entry(bytes, table_start, index)?;
            if previous_key.is_some_and(|key| entry.key.encoded() <= key) {
                return Err(Error::Corrupt(
                    "bigram keys must be strictly increasing".to_string(),
                ));
            }
            if entry.offset != expected_offset {
                return Err(Error::Corrupt(
                    "bigram posting ranges must be contiguous".to_string(),
                ));
            }
            if entry.count == 0 || entry.length == 0 {
                return Err(Error::Corrupt(
                    "a bigram entry must contain at least one posting".to_string(),
                ));
            }
            if u64::from(entry.count).saturating_mul(2) > u64::from(entry.length) {
                return Err(Error::Corrupt(
                    "bigram posting count cannot fit in its byte range".to_string(),
                ));
            }
            expected_offset = entry
                .offset
                .checked_add(u64::from(entry.length))
                .ok_or_else(|| Error::Corrupt("bigram posting range overflows".to_string()))?;
            let absolute_end = u64::try_from(postings_start)
                .unwrap_or(u64::MAX)
                .checked_add(expected_offset)
                .ok_or_else(|| Error::Corrupt("bigram posting boundary overflows".to_string()))?;
            if absolute_end > table_start as u64 {
                return Err(Error::Corrupt(
                    "bigram posting range extends into the entry table".to_string(),
                ));
            }
            previous_key = Some(entry.key.encoded());
        }
        if expected_offset != (table_start - postings_start) as u64 {
            return Err(Error::Corrupt(
                "bigram posting data has an unreferenced tail".to_string(),
            ));
        }
        let mut previous_term = None;
        for index in 0..coverage_count {
            let term = read_u32(bytes, coverage_start + index * 4)?;
            if previous_term.is_some_and(|previous| term <= previous) {
                return Err(Error::Corrupt(
                    "bigram covered terms must be strictly increasing".to_string(),
                ));
            }
            previous_term = Some(term);
        }
        Ok(Self {
            file,
            count,
            postings_start,
            table_start,
            coverage_start,
            coverage_count,
            coverage_authoritative: version == BIGRAM_VERSION,
            universe,
        })
    }

    #[cfg(test)]
    pub(super) fn lookup(&self, key: BigramKey) -> Result<Vec<BigramPosting>> {
        let mut postings = Vec::new();
        let _ = self.decode(key, |posting| {
            postings.push(posting);
            Ok(())
        })?;
        Ok(postings)
    }

    pub(super) const fn universe(&self) -> u32 {
        self.universe
    }

    pub(super) fn covers(&self, key: BigramKey) -> Result<bool> {
        Ok(self.covered_term(key.left())? && self.covered_term(key.right())?)
    }

    pub(super) fn has_same_coverage(&self, other: &Self) -> bool {
        self.coverage_authoritative == other.coverage_authoritative
            && self.coverage_count == other.coverage_count
            && self.file.bytes()[self.coverage_start..self.coverage_start + self.coverage_count * 4]
                == other.file.bytes()
                    [other.coverage_start..other.coverage_start + other.coverage_count * 4]
    }

    pub(super) fn validate_term_universe(&self, terms: usize) -> Result<()> {
        if terms > u32::MAX as usize {
            return Err(Error::Corrupt(
                "bigram term universe exceeds u32".to_string(),
            ));
        }
        if terms == 0 {
            return if self.count == 0 && self.coverage_count == 0 {
                Ok(())
            } else {
                Err(Error::Corrupt(
                    "empty lexicon has nonempty bigram metadata".to_string(),
                ))
            };
        }
        if self.coverage_count > 0 {
            let last = read_u32(
                self.file.bytes(),
                self.coverage_start + (self.coverage_count - 1) * 4,
            )?;
            if last as usize >= terms {
                return Err(Error::Corrupt(
                    "bigram covered term is outside the lexicon".to_string(),
                ));
            }
        }
        for index in 0..self.count {
            let entry = read_entry(self.file.bytes(), self.table_start, index)?;
            if entry.key.left() as usize >= terms || entry.key.right() as usize >= terms {
                return Err(Error::Corrupt(
                    "bigram key term is outside the lexicon".to_string(),
                ));
            }
            if self.coverage_authoritative && !self.covers(entry.key)? {
                return Err(Error::Corrupt(
                    "bigram key uses a term outside authoritative coverage".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn covered_term(&self, target: u32) -> Result<bool> {
        let bytes = self.file.bytes();
        let mut low = 0_usize;
        let mut high = self.coverage_count;
        while low < high {
            let middle = low + (high - low) / 2;
            match read_u32(bytes, self.coverage_start + middle * 4)?.cmp(&target) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return Ok(true),
            }
        }
        Ok(false)
    }

    pub(super) fn lookup_targets_into(
        &self,
        key: BigramKey,
        targets: &[u32],
        postings: &mut Vec<BigramPosting>,
    ) -> Result<bool> {
        postings.clear();
        if targets.windows(2).any(|window| window[0] >= window[1])
            || targets
                .last()
                .is_some_and(|target| *target >= self.universe)
        {
            return Err(Error::InvalidInput(
                "bigram lookup targets must be increasing and in-universe".to_string(),
            ));
        }
        postings.try_reserve(targets.len()).map_err(|_| {
            Error::InvalidInput("bigram target count exceeds memory limits".to_string())
        })?;
        let mut target_index = 0_usize;
        self.decode(key, |posting| {
            while targets
                .get(target_index)
                .is_some_and(|target| *target < posting.document)
            {
                target_index += 1;
            }
            if targets.get(target_index).copied() == Some(posting.document) {
                postings.push(posting);
                target_index += 1;
            }
            Ok(())
        })
    }

    fn decode(
        &self,
        key: BigramKey,
        mut visit: impl FnMut(BigramPosting) -> Result<()>,
    ) -> Result<bool> {
        let Some(entry) = self.find(key)? else {
            return Ok(false);
        };
        let count = entry.count as usize;
        let relative_end = entry
            .offset
            .checked_add(u64::from(entry.length))
            .ok_or_else(|| Error::Corrupt("bigram posting range overflows".to_string()))?;
        let start =
            self.postings_start
                .checked_add(usize::try_from(entry.offset).map_err(|_| {
                    Error::Corrupt("bigram posting offset exceeds usize".to_string())
                })?)
                .ok_or_else(|| Error::Corrupt("bigram posting start overflows".to_string()))?;
        let end =
            self.postings_start
                .checked_add(usize::try_from(relative_end).map_err(|_| {
                    Error::Corrupt("bigram posting boundary exceeds usize".to_string())
                })?)
                .ok_or_else(|| Error::Corrupt("bigram posting end overflows".to_string()))?;
        let encoded =
            self.file.bytes().get(start..end).ok_or_else(|| {
                Error::Corrupt("bigram posting range is out of bounds".to_string())
            })?;
        let mut cursor = 0_usize;
        let mut previous_document = 0_u32;
        for index in 0..count {
            let delta = read_varint(encoded, &mut cursor, encoded.len())?;
            if index > 0 && delta == 0 {
                return Err(Error::Corrupt(
                    "bigram document identifiers must be strictly increasing".to_string(),
                ));
            }
            let document = previous_document.checked_add(delta).ok_or_else(|| {
                Error::Corrupt("bigram document identifier overflows".to_string())
            })?;
            if document >= self.universe {
                return Err(Error::Corrupt(
                    "bigram document identifier is outside its universe".to_string(),
                ));
            }
            let earliest_end_offset = read_varint(encoded, &mut cursor, encoded.len())?;
            if earliest_end_offset == 0 {
                return Err(Error::Corrupt(
                    "bigram end offsets must be positive".to_string(),
                ));
            }
            visit(BigramPosting {
                document,
                earliest_end_offset,
            })?;
            previous_document = document;
        }
        if cursor != encoded.len() {
            return Err(Error::Corrupt(
                "bigram posting count does not consume its byte range".to_string(),
            ));
        }
        Ok(true)
    }

    fn find(&self, key: BigramKey) -> Result<Option<BigramEntry>> {
        let bytes = self.file.bytes();
        let mut low = 0_usize;
        let mut high = self.count;
        while low < high {
            let middle = low + (high - low) / 2;
            let entry = read_entry(bytes, self.table_start, middle)?;
            match entry.key.encoded().cmp(&key.encoded()) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return Ok(Some(entry)),
            }
        }
        Ok(None)
    }
}

pub(super) struct BigramIndexWriter {
    output: ColdWriter,
    universe: u32,
    covered_terms: Vec<u32>,
    metadata: Vec<BigramEntry>,
    previous_key: Option<u64>,
    encoded: Vec<u8>,
}

impl BigramIndexWriter {
    pub(super) fn create(path: &Path, universe: u32, covered_terms: &[u32]) -> Result<Self> {
        if universe == 0 {
            return Err(Error::InvalidInput(
                "bigram universe must be positive".to_string(),
            ));
        }
        if covered_terms
            .windows(2)
            .any(|window| window[0] >= window[1])
        {
            return Err(Error::InvalidInput(
                "bigram covered terms must be strictly increasing".to_string(),
            ));
        }
        let coverage_count = u32::try_from(covered_terms.len()).map_err(|_| {
            Error::InvalidInput("bigram covered-term count exceeds u32".to_string())
        })?;
        let mut output = ColdWriter::create(path)?;
        output.write_all(BIGRAM_MAGIC)?;
        output.write_all(&BIGRAM_VERSION.to_le_bytes())?;
        output.write_all(&0_u32.to_le_bytes())?;
        output.write_all(&0_u64.to_le_bytes())?;
        output.write_all(&0_u64.to_le_bytes())?;
        output.write_all(&u64::from(universe).to_le_bytes())?;
        output.write_all(&coverage_count.to_le_bytes())?;
        output.write_all(&0_u32.to_le_bytes())?;
        Ok(Self {
            output,
            universe,
            covered_terms: covered_terms.to_vec(),
            metadata: Vec::new(),
            previous_key: None,
            encoded: Vec::with_capacity(10),
        })
    }

    pub(super) fn append(&mut self, key: BigramKey, postings: &[BigramPosting]) -> Result<()> {
        if !self.append_iter(key, postings.iter().copied().map(Ok))? {
            return Err(Error::InvalidInput(
                "a bigram entry must contain at least one posting".to_string(),
            ));
        }
        Ok(())
    }

    /// Appends one posting list without requiring it to be materialized.
    ///
    /// Empty iterators are ignored so a caller can make one bounded pass per
    /// field over an interleaved spool shard. Non-empty iterators retain the
    /// same ordering and universe contract as [`Self::append`].
    pub(super) fn append_iter(
        &mut self,
        key: BigramKey,
        postings: impl IntoIterator<Item = Result<BigramPosting>>,
    ) -> Result<bool> {
        let mut postings = postings.into_iter();
        let Some(first) = postings.next().transpose()? else {
            return Ok(false);
        };
        if self
            .previous_key
            .is_some_and(|previous| key.encoded() <= previous)
        {
            return Err(Error::InvalidInput(
                "bigram keys must be strictly increasing".to_string(),
            ));
        }
        let offset = self
            .output
            .position()
            .checked_sub(BIGRAM_HEADER_BYTES as u64)
            .ok_or_else(|| {
                Error::Corrupt("bigram output position precedes its postings".to_string())
            })?;
        let mut previous_document = 0_u32;
        let mut count = 0_u32;
        for posting in std::iter::once(Ok(first)).chain(postings) {
            let posting = posting?;
            let index = count;
            if (index > 0 && posting.document <= previous_document)
                || posting.document >= self.universe
                || posting.earliest_end_offset == 0
            {
                return Err(Error::InvalidInput(
                    "bigram postings require in-universe increasing identifiers and positive offsets"
                    .to_string(),
                ));
            }
            let delta = if index == 0 {
                posting.document
            } else {
                posting.document - previous_document
            };
            self.encoded.clear();
            write_varint(delta, &mut self.encoded);
            write_varint(posting.earliest_end_offset, &mut self.encoded);
            self.output.write_all(&self.encoded)?;
            previous_document = posting.document;
            count = count.checked_add(1).ok_or_else(|| {
                Error::InvalidInput("one bigram has more than 4 billion documents".to_string())
            })?;
        }
        let length = self
            .output
            .position()
            .checked_sub(BIGRAM_HEADER_BYTES as u64 + offset)
            .ok_or_else(|| Error::Corrupt("bigram output length underflows".to_string()))?;
        self.metadata.push(BigramEntry {
            key,
            offset,
            length: u32::try_from(length).map_err(|_| {
                Error::InvalidInput("one bigram posting list exceeds 4 GiB".to_string())
            })?,
            count,
        });
        self.previous_key = Some(key.encoded());
        Ok(true)
    }

    pub(super) fn finish(mut self) -> Result<()> {
        let count = u32::try_from(self.metadata.len())
            .map_err(|_| Error::InvalidInput("bigram pair count exceeds u32".to_string()))?;
        let table_start = self.output.position();
        for entry in &self.metadata {
            self.output.write_all(&entry.key.encoded().to_le_bytes())?;
            self.output.write_all(&entry.offset.to_le_bytes())?;
            self.output.write_all(&entry.length.to_le_bytes())?;
            self.output.write_all(&entry.count.to_le_bytes())?;
        }
        for term in &self.covered_terms {
            self.output.write_all(&term.to_le_bytes())?;
        }
        let file_length = self.output.position();
        self.output.seek(SeekFrom::Start(12))?;
        self.output.write_all(&count.to_le_bytes())?;
        self.output.write_all(&table_start.to_le_bytes())?;
        self.output.write_all(&file_length.to_le_bytes())?;
        self.output.finish()?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn write_bigram_index(
    path: &Path,
    universe: u32,
    covered_terms: &[u32],
    entries: &[BigramWriteEntry<'_>],
) -> Result<()> {
    let mut writer = BigramIndexWriter::create(path, universe, covered_terms)?;
    for entry in entries {
        writer.append(entry.key, entry.postings)?;
    }
    writer.finish()?;
    Ok(())
}

fn read_entry(bytes: &[u8], table_start: usize, index: usize) -> Result<BigramEntry> {
    let start = table_start
        .checked_add(
            index
                .checked_mul(BIGRAM_ENTRY_BYTES)
                .ok_or_else(|| Error::Corrupt("bigram entry offset overflows".to_string()))?,
        )
        .ok_or_else(|| Error::Corrupt("bigram entry boundary overflows".to_string()))?;
    Ok(BigramEntry {
        key: BigramKey(read_u64(bytes, start)?),
        offset: read_u64(bytes, start + 8)?,
        length: read_u32(bytes, start + 16)?,
        count: read_u32(bytes, start + 20)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, fs::OpenOptions, io::Write};
    use tempfile::tempdir;

    const FIRST: [BigramPosting; 3] = [
        BigramPosting {
            document: 0,
            earliest_end_offset: 1,
        },
        BigramPosting {
            document: 7,
            earliest_end_offset: 4,
        },
        BigramPosting {
            document: 300,
            earliest_end_offset: 129,
        },
    ];
    const SECOND: [BigramPosting; 1] = [BigramPosting {
        document: 5,
        earliest_end_offset: 2,
    }];

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempdir().unwrap();
        let path = directory.path().join("bigrams.bin");
        let mut writer = BigramIndexWriter::create(&path, 1_000, &[1, 2, 3]).unwrap();
        assert_eq!(writer.output.position(), BIGRAM_HEADER_BYTES as u64);
        writer.append(BigramKey::new(1, 2), &FIRST).unwrap();
        writer.append(BigramKey::new(2, 3), &SECOND).unwrap();
        assert_eq!(writer.output.position(), BIGRAM_HEADER_BYTES as u64 + 10);
        writer.finish().unwrap();
        (directory, path)
    }

    fn fixture_table_start(path: &Path) -> u64 {
        read_u64(&fs::read(path).unwrap(), 16).unwrap()
    }

    #[test]
    fn sorted_pair_lookup_round_trips_delta_documents_and_offsets() {
        let (_directory, path) = fixture();
        let encoded = fs::read(&path).unwrap();
        assert_eq!(read_u32(&encoded, 12).unwrap(), 2);
        let table_start = BIGRAM_HEADER_BYTES as u64 + 10;
        assert_eq!(read_u64(&encoded, 16).unwrap(), table_start);
        assert_eq!(
            read_u64(&encoded, 24).unwrap(),
            table_start + 2 * BIGRAM_ENTRY_BYTES as u64 + 3 * 4
        );
        assert_eq!(read_u32(&encoded, 40).unwrap(), 3);
        let index = BigramIndex::open(&path).unwrap();
        assert!(index.covers(BigramKey::new(1, 3)).unwrap());
        assert!(!index.covers(BigramKey::new(1, 9)).unwrap());
        assert_eq!(index.lookup(BigramKey::new(1, 2)).unwrap(), FIRST);
        assert_eq!(index.lookup(BigramKey::new(2, 3)).unwrap(), SECOND);
        assert!(index.lookup(BigramKey::new(9, 9)).unwrap().is_empty());

        let mut reused = Vec::with_capacity(8);
        let allocation = reused.as_ptr();
        assert!(
            index
                .lookup_targets_into(BigramKey::new(1, 2), &[0, 4], &mut reused)
                .unwrap()
        );
        assert_eq!(reused, [FIRST[0]]);
        assert_eq!(reused.as_ptr(), allocation);
        assert!(
            index
                .lookup_targets_into(BigramKey::new(1, 2), &[], &mut reused)
                .unwrap()
        );
        assert!(reused.is_empty());
        assert!(
            !index
                .lookup_targets_into(BigramKey::new(9, 9), &[0], &mut reused)
                .unwrap()
        );
        assert!(index.validate_term_universe(4).is_ok());
        assert!(index.validate_term_universe(3).is_err());
    }

    #[test]
    fn legacy_v1_remains_readable_without_authoritative_coverage() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("legacy-v1.bin");
        let mut encoded = Vec::new();
        let mut previous = 0_u32;
        for (index, posting) in FIRST.iter().enumerate() {
            write_varint(
                if index == 0 {
                    posting.document
                } else {
                    posting.document - previous
                },
                &mut encoded,
            );
            write_varint(posting.earliest_end_offset, &mut encoded);
            previous = posting.document;
        }
        let table_start = LEGACY_BIGRAM_HEADER_BYTES as u64 + encoded.len() as u64;
        let file_length = table_start + BIGRAM_ENTRY_BYTES as u64;
        let mut output = File::create(&path).unwrap();
        output.write_all(BIGRAM_MAGIC).unwrap();
        output
            .write_all(&LEGACY_BIGRAM_VERSION.to_le_bytes())
            .unwrap();
        output.write_all(&1_u32.to_le_bytes()).unwrap();
        output.write_all(&table_start.to_le_bytes()).unwrap();
        output.write_all(&file_length.to_le_bytes()).unwrap();
        output.write_all(&1_000_u64.to_le_bytes()).unwrap();
        output.write_all(&encoded).unwrap();
        output
            .write_all(&BigramKey::new(1, 2).encoded().to_le_bytes())
            .unwrap();
        output.write_all(&0_u64.to_le_bytes()).unwrap();
        output
            .write_all(&(encoded.len() as u32).to_le_bytes())
            .unwrap();
        output
            .write_all(&(FIRST.len() as u32).to_le_bytes())
            .unwrap();
        drop(output);

        let index = BigramIndex::open(&path).unwrap();
        assert_eq!(index.lookup(BigramKey::new(1, 2)).unwrap(), FIRST);
        assert!(!index.covers(BigramKey::new(1, 2)).unwrap());
        assert!(index.validate_term_universe(3).is_ok());
    }

    #[test]
    fn authoritative_entries_must_use_covered_lexicon_terms() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("uncovered-key.bin");
        let mut writer = BigramIndexWriter::create(&path, 1_000, &[1]).unwrap();
        writer.append(BigramKey::new(1, 2), &FIRST).unwrap();
        writer.finish().unwrap();

        let index = BigramIndex::open(&path).unwrap();
        assert!(index.validate_term_universe(3).is_err());
    }

    #[test]
    fn targeted_lookup_rejects_ambiguous_or_out_of_universe_targets() {
        let (_directory, path) = fixture();
        let index = BigramIndex::open(&path).unwrap();
        let mut postings = Vec::new();

        assert!(
            index
                .lookup_targets_into(BigramKey::new(1, 2), &[4, 4], &mut postings)
                .is_err()
        );
        assert!(
            index
                .lookup_targets_into(BigramKey::new(1, 2), &[4, 3], &mut postings)
                .is_err()
        );
        assert!(
            index
                .lookup_targets_into(BigramKey::new(1, 2), &[1_000], &mut postings)
                .is_err()
        );
    }

    #[test]
    fn writer_rejects_unsorted_pairs_and_postings() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("unsorted-pairs.bin");
        let entries = [
            BigramWriteEntry {
                key: BigramKey::new(2, 3),
                postings: &SECOND,
            },
            BigramWriteEntry {
                key: BigramKey::new(1, 2),
                postings: &FIRST,
            },
        ];
        assert!(write_bigram_index(&path, 1_000, &[1, 2, 3], &entries).is_err());

        let path = directory.path().join("unsorted-postings.bin");
        let postings = [
            BigramPosting {
                document: 4,
                earliest_end_offset: 2,
            },
            BigramPosting {
                document: 4,
                earliest_end_offset: 3,
            },
        ];
        assert!(
            write_bigram_index(
                &path,
                1_000,
                &[1, 2],
                &[BigramWriteEntry {
                    key: BigramKey::new(1, 2),
                    postings: &postings,
                }]
            )
            .is_err()
        );
    }

    #[test]
    fn open_rejects_version_length_key_and_range_corruption() {
        for name in [
            "version",
            "length",
            "table",
            "keys",
            "range",
            "count",
            "universe",
            "coverage-count",
            "coverage-reserved",
            "coverage-order",
        ] {
            let (_directory, path) = fixture();
            let table_start = fixture_table_start(&path);
            let (offset, bytes) = match name {
                "version" => (8, 1_u32.to_le_bytes().to_vec()),
                "length" => (24, 1_u64.to_le_bytes().to_vec()),
                "table" => (16, (BIGRAM_HEADER_BYTES as u64 - 1).to_le_bytes().to_vec()),
                "keys" => (
                    table_start + BIGRAM_ENTRY_BYTES as u64,
                    0_u64.to_le_bytes().to_vec(),
                ),
                "range" => (table_start + 8, 1_u64.to_le_bytes().to_vec()),
                "count" => (table_start + 20, u32::MAX.to_le_bytes().to_vec()),
                "universe" => (32, 0_u64.to_le_bytes().to_vec()),
                "coverage-count" => (40, u32::MAX.to_le_bytes().to_vec()),
                "coverage-reserved" => (44, 1_u32.to_le_bytes().to_vec()),
                "coverage-order" => (
                    table_start + 2 * BIGRAM_ENTRY_BYTES as u64 + 4,
                    1_u32.to_le_bytes().to_vec(),
                ),
                _ => unreachable!(),
            };
            let mut file = OpenOptions::new().write(true).open(&path).unwrap();
            file.seek(SeekFrom::Start(offset)).unwrap();
            file.write_all(&bytes).unwrap();
            drop(file);
            assert!(
                BigramIndex::open(&path).is_err(),
                "accepted {name} corruption"
            );
        }
    }

    #[test]
    fn lookup_rejects_payload_count_and_monotonicity_corruption() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("payload.bin");
        let postings = [
            BigramPosting {
                document: 0,
                earliest_end_offset: 1,
            },
            BigramPosting {
                document: 1,
                earliest_end_offset: 2,
            },
        ];
        write_bigram_index(
            &path,
            1_000,
            &[1, 2],
            &[BigramWriteEntry {
                key: BigramKey::new(1, 2),
                postings: &postings,
            }],
        )
        .unwrap();
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start((BIGRAM_HEADER_BYTES + 2) as u64))
            .unwrap();
        file.write_all(&[0]).unwrap();
        drop(file);

        let index = BigramIndex::open(&path).unwrap();
        assert!(index.lookup(BigramKey::new(1, 2)).is_err());
    }

    #[test]
    fn lookup_rejects_a_posting_outside_the_declared_universe() {
        let (_directory, path) = fixture();
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(32)).unwrap();
        file.write_all(&300_u64.to_le_bytes()).unwrap();
        drop(file);

        let index = BigramIndex::open(&path).unwrap();
        assert!(index.lookup(BigramKey::new(1, 2)).is_err());
    }
}
