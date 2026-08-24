use super::*;

const BIGRAM_MAGIC: &[u8; 8] = b"RAWBIG01";
const BIGRAM_VERSION: u32 = 1;
const BIGRAM_HEADER_BYTES: usize = 40;
const BIGRAM_ENTRY_BYTES: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BigramKey(u64);

impl BigramKey {
    pub(super) const fn new(left_term: u32, right_term: u32) -> Self {
        Self((left_term as u64) << 32 | right_term as u64)
    }

    const fn encoded(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BigramPosting {
    pub(super) document: u32,
    pub(super) earliest_end_offset: u32,
}

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

pub(super) struct BigramIndex {
    file: MappedFile,
    count: usize,
    postings_start: usize,
    universe: u32,
}

impl BigramIndex {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let file = MappedFile::open(path, BIGRAM_MAGIC)?;
        let bytes = file.bytes();
        if bytes.len() < BIGRAM_HEADER_BYTES {
            return Err(Error::Corrupt(
                "bigram index is shorter than its header".to_string(),
            ));
        }
        let version = read_u32(bytes, 8)?;
        if version != BIGRAM_VERSION {
            return Err(Error::Corrupt(format!(
                "unsupported bigram index version {version}"
            )));
        }
        let count = read_u32(bytes, 12)? as usize;
        let expected_postings_start = BIGRAM_HEADER_BYTES
            .checked_add(
                count.checked_mul(BIGRAM_ENTRY_BYTES).ok_or_else(|| {
                    Error::Corrupt("bigram entry table length overflows".to_string())
                })?,
            )
            .ok_or_else(|| Error::Corrupt("bigram entry table boundary overflows".to_string()))?;
        let postings_start = usize::try_from(read_u64(bytes, 16)?)
            .map_err(|_| Error::Corrupt("bigram postings offset exceeds usize".to_string()))?;
        if postings_start != expected_postings_start || postings_start > bytes.len() {
            return Err(Error::Corrupt(
                "bigram postings offset does not follow the entry table".to_string(),
            ));
        }
        let declared_length = usize::try_from(read_u64(bytes, 24)?)
            .map_err(|_| Error::Corrupt("bigram file length exceeds usize".to_string()))?;
        if declared_length != bytes.len() {
            return Err(Error::Corrupt(
                "bigram file length does not match its header".to_string(),
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
            let entry = read_entry(bytes, index)?;
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
            if absolute_end > bytes.len() as u64 {
                return Err(Error::Corrupt(
                    "bigram posting range extends beyond the file".to_string(),
                ));
            }
            previous_key = Some(entry.key.encoded());
        }
        if expected_offset != (bytes.len() - postings_start) as u64 {
            return Err(Error::Corrupt(
                "bigram posting data has an unreferenced tail".to_string(),
            ));
        }
        Ok(Self {
            file,
            count,
            postings_start,
            universe,
        })
    }

    pub(super) fn lookup(&self, key: BigramKey) -> Result<Vec<BigramPosting>> {
        let mut postings = Vec::new();
        self.lookup_into(key, &mut postings)?;
        Ok(postings)
    }

    pub(super) fn lookup_into(
        &self,
        key: BigramKey,
        postings: &mut Vec<BigramPosting>,
    ) -> Result<()> {
        postings.clear();
        let Some(entry) = self.find(key)? else {
            return Ok(());
        };
        let count = entry.count as usize;
        postings.try_reserve(count).map_err(|_| {
            Error::Corrupt("bigram posting count exceeds memory limits".to_string())
        })?;
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
            postings.push(BigramPosting {
                document,
                earliest_end_offset,
            });
            previous_document = document;
        }
        if cursor != encoded.len() {
            return Err(Error::Corrupt(
                "bigram posting count does not consume its byte range".to_string(),
            ));
        }
        Ok(())
    }

    fn find(&self, key: BigramKey) -> Result<Option<BigramEntry>> {
        let bytes = self.file.bytes();
        let mut low = 0_usize;
        let mut high = self.count;
        while low < high {
            let middle = low + (high - low) / 2;
            let entry = read_entry(bytes, middle)?;
            match entry.key.encoded().cmp(&key.encoded()) {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return Ok(Some(entry)),
            }
        }
        Ok(None)
    }
}

pub(super) fn write_bigram_index(
    path: &Path,
    universe: u32,
    entries: &[BigramWriteEntry<'_>],
) -> Result<()> {
    if universe == 0 {
        return Err(Error::InvalidInput(
            "bigram universe must be positive".to_string(),
        ));
    }
    let count = u32::try_from(entries.len())
        .map_err(|_| Error::InvalidInput("bigram pair count exceeds u32".to_string()))?;
    let postings_start = BIGRAM_HEADER_BYTES
        .checked_add(
            entries
                .len()
                .checked_mul(BIGRAM_ENTRY_BYTES)
                .ok_or_else(|| {
                    Error::InvalidInput("bigram entry table length overflows".to_string())
                })?,
        )
        .ok_or_else(|| Error::InvalidInput("bigram postings offset overflows".to_string()))?;
    let mut output = ColdWriter::create(path)?;
    output.write_all(BIGRAM_MAGIC)?;
    output.write_all(&BIGRAM_VERSION.to_le_bytes())?;
    output.write_all(&count.to_le_bytes())?;
    output.write_all(&(postings_start as u64).to_le_bytes())?;
    output.write_all(&0_u64.to_le_bytes())?;
    output.write_all(&u64::from(universe).to_le_bytes())?;
    output.seek(SeekFrom::Start(postings_start as u64))?;

    let mut metadata = Vec::with_capacity(entries.len());
    let mut previous_key = None;
    let mut encoded = Vec::with_capacity(10);
    for entry in entries {
        if previous_key.is_some_and(|key| entry.key.encoded() <= key) {
            return Err(Error::InvalidInput(
                "bigram keys must be strictly increasing".to_string(),
            ));
        }
        if entry.postings.is_empty() {
            return Err(Error::InvalidInput(
                "a bigram entry must contain at least one posting".to_string(),
            ));
        }
        let offset = output
            .position()
            .checked_sub(postings_start as u64)
            .ok_or_else(|| {
                Error::Corrupt("bigram output position precedes its postings".to_string())
            })?;
        let mut previous_document = 0_u32;
        for (index, posting) in entry.postings.iter().enumerate() {
            if (index > 0 && posting.document <= previous_document)
                || posting.document >= universe
                || posting.earliest_end_offset == 0
            {
                return Err(Error::InvalidInput(
                    "bigram postings require increasing documents and positive offsets".to_string(),
                ));
            }
            let delta = if index == 0 {
                posting.document
            } else {
                posting.document - previous_document
            };
            encoded.clear();
            write_varint(delta, &mut encoded);
            write_varint(posting.earliest_end_offset, &mut encoded);
            output.write_all(&encoded)?;
            previous_document = posting.document;
        }
        let length = output
            .position()
            .checked_sub(postings_start as u64 + offset)
            .ok_or_else(|| Error::Corrupt("bigram output length underflows".to_string()))?;
        metadata.push(BigramEntry {
            key: entry.key,
            offset,
            length: u32::try_from(length).map_err(|_| {
                Error::InvalidInput("one bigram posting list exceeds 4 GiB".to_string())
            })?,
            count: u32::try_from(entry.postings.len()).map_err(|_| {
                Error::InvalidInput("one bigram has more than 4 billion documents".to_string())
            })?,
        });
        previous_key = Some(entry.key.encoded());
    }

    let file_length = output.position();
    output.seek(SeekFrom::Start(24))?;
    output.write_all(&file_length.to_le_bytes())?;
    output.seek(SeekFrom::Start(BIGRAM_HEADER_BYTES as u64))?;
    for entry in metadata {
        output.write_all(&entry.key.encoded().to_le_bytes())?;
        output.write_all(&entry.offset.to_le_bytes())?;
        output.write_all(&entry.length.to_le_bytes())?;
        output.write_all(&entry.count.to_le_bytes())?;
    }
    output.finish()?;
    Ok(())
}

fn read_entry(bytes: &[u8], index: usize) -> Result<BigramEntry> {
    let start = BIGRAM_HEADER_BYTES
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
    use std::{fs::OpenOptions, io::Write};
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
        write_bigram_index(
            &path,
            1_000,
            &[
                BigramWriteEntry {
                    key: BigramKey::new(1, 2),
                    postings: &FIRST,
                },
                BigramWriteEntry {
                    key: BigramKey::new(2, 3),
                    postings: &SECOND,
                },
            ],
        )
        .unwrap();
        (directory, path)
    }

    #[test]
    fn sorted_pair_lookup_round_trips_delta_documents_and_offsets() {
        let (_directory, path) = fixture();
        let index = BigramIndex::open(&path).unwrap();
        assert_eq!(index.lookup(BigramKey::new(1, 2)).unwrap(), FIRST);
        assert_eq!(index.lookup(BigramKey::new(2, 3)).unwrap(), SECOND);
        assert!(index.lookup(BigramKey::new(9, 9)).unwrap().is_empty());

        let mut reused = Vec::with_capacity(8);
        let allocation = reused.as_ptr();
        index
            .lookup_into(BigramKey::new(1, 2), &mut reused)
            .unwrap();
        assert_eq!(reused, FIRST);
        assert_eq!(reused.as_ptr(), allocation);
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
        assert!(write_bigram_index(&path, 1_000, &entries).is_err());

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
        for (name, offset, bytes) in [
            ("version", 8_u64, 2_u32.to_le_bytes().to_vec()),
            ("length", 24, 1_u64.to_le_bytes().to_vec()),
            (
                "keys",
                (BIGRAM_HEADER_BYTES + BIGRAM_ENTRY_BYTES) as u64,
                0_u64.to_le_bytes().to_vec(),
            ),
            (
                "range",
                (BIGRAM_HEADER_BYTES + 8) as u64,
                1_u64.to_le_bytes().to_vec(),
            ),
            (
                "count",
                (BIGRAM_HEADER_BYTES + 20) as u64,
                u32::MAX.to_le_bytes().to_vec(),
            ),
            ("universe", 32, 0_u64.to_le_bytes().to_vec()),
        ] {
            let (_directory, path) = fixture();
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
            &[BigramWriteEntry {
                key: BigramKey::new(1, 2),
                postings: &postings,
            }],
        )
        .unwrap();
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start(
            (BIGRAM_HEADER_BYTES + BIGRAM_ENTRY_BYTES + 2) as u64,
        ))
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
