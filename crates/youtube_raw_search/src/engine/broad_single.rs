use super::*;
use std::{cmp::Ordering, io::BufReader, ops::Range};

mod codec;
use codec::PayloadEncoding;

pub(super) const BROAD_SINGLE_FILE: &str = "broad-single.bin";
const BROAD_SINGLE_MAGIC: &[u8; 8] = b"RAWBRD01";
const DIRECTORY_FIXED_BYTES: usize = 20;
const ANSWER_HEADER_BYTES: usize = 16;
const GROUP_RECORD_BYTES: usize = 20;
const DOCUMENT_RECORD_BYTES: usize = 12;
const MIN_MATERIALIZED_DOCUMENTS: u64 = 25_000;
const MATERIALIZED_HITS_PER_VIDEO: usize = 5;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub(super) enum BroadProfile {
    TextFlat = 0,
    TextPosition = 1,
    TitleFlat = 2,
    TitlePosition = 3,
    CombinedFlat = 4,
    CombinedPosition = 5,
}

impl BroadProfile {
    pub(super) fn from_fields(fields: QueryFields) -> Result<Self> {
        match (fields.text, fields.title, fields.prioritize_position) {
            (true, false, false) => Ok(Self::TextFlat),
            (true, false, true) => Ok(Self::TextPosition),
            (false, true, false) => Ok(Self::TitleFlat),
            (false, true, true) => Ok(Self::TitlePosition),
            (true, true, false) => Ok(Self::CombinedFlat),
            (true, true, true) => Ok(Self::CombinedPosition),
            (false, false, _) => Err(Error::Corrupt(
                "broad query profile has no fields".to_string(),
            )),
        }
    }

    fn decode(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::TextFlat),
            1 => Ok(Self::TextPosition),
            2 => Ok(Self::TitleFlat),
            3 => Ok(Self::TitlePosition),
            4 => Ok(Self::CombinedFlat),
            5 => Ok(Self::CombinedPosition),
            _ => Err(Error::Corrupt(format!(
                "broad query profile {value} is invalid"
            ))),
        }
    }

    fn fields(self) -> Result<QueryFields> {
        match self {
            Self::TextFlat => query_fields(Some("text"), false),
            Self::TextPosition => query_fields(Some("text"), true),
            Self::TitleFlat => query_fields(Some("title"), false),
            Self::TitlePosition => query_fields(Some("title"), true),
            Self::CombinedFlat => query_fields(Some("text,title"), false),
            Self::CombinedPosition => query_fields(Some("text,title"), true),
        }
    }

    const ALL: [Self; 6] = [
        Self::TextFlat,
        Self::TextPosition,
        Self::TitleFlat,
        Self::TitlePosition,
        Self::CombinedFlat,
        Self::CombinedPosition,
    ];
}

#[derive(Clone)]
struct DirectoryEntry {
    key: Range<usize>,
    profile: BroadProfile,
    payload: Range<usize>,
}

pub(super) struct BroadSingleIndex {
    file: MappedFile,
    entries: Vec<DirectoryEntry>,
    retained_depth: usize,
    encoding: PayloadEncoding,
    maximum_answer_bytes: usize,
}

pub(super) struct BroadAnswer {
    pub(super) found: usize,
    pub(super) total_moments: usize,
    pub(super) groups: Vec<VideoMatches>,
}

#[cfg(test)]
pub(super) struct BroadBuildPlan {
    pub(super) token: String,
    pub(super) profile: BroadProfile,
    pub(super) groups: Vec<Option<VideoMatches>>,
}

struct BuildEntry {
    token: String,
    profile: BroadProfile,
    payload_offset: u64,
    payload_length: u64,
}

struct BroadIndexWriter {
    payload_path: PathBuf,
    payload: ColdWriter,
    payload_bytes: u64,
    entries: Vec<BuildEntry>,
}

impl BroadSingleIndex {
    pub(super) fn open(path: &Path, videos: usize, documents: u32) -> Result<Self> {
        let file = MappedFile::open(path, BROAD_SINGLE_MAGIC)?;
        let bytes = file.bytes();
        let count = read_u32(bytes, 8)? as usize;
        let version = read_u32(bytes, 12)?;
        let (retained_depth, encoding) = match version {
            1 => (usize::from(MAX_PAGE_SIZE), PayloadEncoding::Raw),
            2 => (MATERIALIZED_HITS_PER_VIDEO, PayloadEncoding::Raw),
            3 => (MATERIALIZED_HITS_PER_VIDEO, PayloadEncoding::Zstd),
            _ => {
                return Err(Error::Corrupt(format!(
                    "unsupported broad query index version {version}"
                )));
            }
        };
        let maximum_answer_bytes = videos
            .checked_mul(GROUP_RECORD_BYTES + DOCUMENT_RECORD_BYTES * retained_depth)
            .and_then(|bytes| bytes.checked_add(ANSWER_HEADER_BYTES))
            .ok_or_else(|| Error::Corrupt("broad answer bounds overflow".into()))?;

        let mut cursor = HEADER_BYTES;
        let mut entries = Vec::with_capacity(count);
        let mut previous: Option<(Vec<u8>, BroadProfile)> = None;
        for _ in 0..count {
            let key_length = read_u16(bytes, cursor)? as usize;
            let profile = BroadProfile::decode(read_byte(bytes, cursor + 2)?)?;
            if read_byte(bytes, cursor + 3)? != 0 {
                return Err(Error::Corrupt(
                    "broad query directory reserved byte is non-zero".to_string(),
                ));
            }
            let payload_start = usize::try_from(read_u64(bytes, cursor + 4)?)
                .map_err(|_| Error::Corrupt("broad payload offset overflows".to_string()))?;
            let payload_length = usize::try_from(read_u64(bytes, cursor + 12)?)
                .map_err(|_| Error::Corrupt("broad payload length overflows".to_string()))?;
            let key_start = cursor
                .checked_add(DIRECTORY_FIXED_BYTES)
                .ok_or_else(|| Error::Corrupt("broad directory offset overflows".to_string()))?;
            let key_end = key_start
                .checked_add(key_length)
                .ok_or_else(|| Error::Corrupt("broad key length overflows".to_string()))?;
            let key = bytes
                .get(key_start..key_end)
                .ok_or_else(|| Error::Corrupt("broad query key is truncated".to_string()))?;
            std::str::from_utf8(key)
                .map_err(|_| Error::Corrupt("broad query key is not UTF-8".to_string()))?;
            if key.is_empty() {
                return Err(Error::Corrupt("broad query key is empty".to_string()));
            }
            if previous
                .as_ref()
                .is_some_and(|value| (value.0.as_slice(), value.1) >= (key, profile))
            {
                return Err(Error::Corrupt(
                    "broad query directory is not strictly ordered".to_string(),
                ));
            }
            previous = Some((key.to_vec(), profile));
            let payload_end = payload_start
                .checked_add(payload_length)
                .ok_or_else(|| Error::Corrupt("broad payload range overflows".to_string()))?;
            bytes
                .get(payload_start..payload_end)
                .ok_or_else(|| Error::Corrupt("broad payload is truncated".to_string()))?;
            entries.push(DirectoryEntry {
                key: key_start..key_end,
                profile,
                payload: payload_start..payload_end,
            });
            cursor = key_end;
        }
        if entries
            .first()
            .is_some_and(|entry| entry.payload.start != cursor)
        {
            return Err(Error::Corrupt(
                "broad payloads do not begin after the directory".to_string(),
            ));
        }
        let mut expected_payload_start = cursor;
        for entry in &entries {
            if entry.payload.start != expected_payload_start {
                return Err(Error::Corrupt(
                    "broad payloads are not contiguous".to_string(),
                ));
            }
            let payload = codec::decode(
                &bytes[entry.payload.clone()],
                encoding,
                maximum_answer_bytes,
            )?;
            validate_answer(
                &payload,
                &(0..payload.len()),
                videos,
                documents,
                retained_depth,
            )?;
            expected_payload_start = entry.payload.end;
        }
        if expected_payload_start != bytes.len() {
            return Err(Error::Corrupt(
                "broad query index has trailing bytes".to_string(),
            ));
        }
        Ok(Self {
            file,
            entries,
            retained_depth,
            encoding,
            maximum_answer_bytes,
        })
    }

    pub(super) fn supports_depth(&self, requested: usize) -> bool {
        requested <= self.retained_depth
    }

    pub(super) fn lookup(&self, token: &str, fields: QueryFields) -> Result<Option<BroadAnswer>> {
        let profile = BroadProfile::from_fields(fields)?;
        let bytes = self.file.bytes();
        let found = self.entries.binary_search_by(|entry| {
            bytes[entry.key.clone()]
                .cmp(token.as_bytes())
                .then_with(|| entry.profile.cmp(&profile))
        });
        let Ok(index) = found else {
            return Ok(None);
        };
        let payload = codec::decode(
            &bytes[self.entries[index].payload.clone()],
            self.encoding,
            self.maximum_answer_bytes,
        )?;
        decode_answer(&payload, &(0..payload.len())).map(Some)
    }
}

pub(super) fn build_index(engine: &RawSearchEngine, path: &Path) -> Result<usize> {
    let mut writer = BroadIndexWriter::create(path)?;
    let average_documents_per_video = engine
        .inner
        .manifest
        .documents
        .checked_div(engine.inner.manifest.videos)
        .unwrap_or(0);
    let tokens = engine
        .inner
        .fuzzy
        .broad_prefixes(MIN_MATERIALIZED_DOCUMENTS, average_documents_per_video)?;
    for token in tokens {
        for profile in BroadProfile::ALL {
            let fields = profile.fields()?;
            let terms = [token.clone()];
            let (query_plans, _) = engine.conjunction_plans(&terms, fields, true, 0)?;
            let [query_plan] = query_plans.as_slice() else {
                continue;
            };
            if query_plan.estimated_documents < MIN_MATERIALIZED_DOCUMENTS {
                continue;
            }
            let dominant_exact_mask = query_plan
                .alternatives
                .iter()
                .fold(0_u8, |mask, alternative| {
                    mask | alternative.fields.exact_mask
                });
            let mut groups = engine.empty_group_slots();
            engine.visit_term_plan(query_plan, |candidate, record| {
                engine.record_group_match_values(
                    &mut groups,
                    candidate,
                    record.video,
                    record.t_start_ms,
                    MATERIALIZED_HITS_PER_VIDEO,
                    Some(dominant_exact_mask),
                )
            })?;
            if groups.iter().flatten().count() >= TYPO_RESULT_THRESHOLD {
                writer.append(&token, profile, &groups)?;
            }
        }
    }
    writer.finish(path)
}

#[cfg(test)]
pub(super) fn write_index(path: &Path, plans: &[BroadBuildPlan]) -> Result<()> {
    let mut ordered = plans.iter().collect::<Vec<_>>();
    ordered.sort_unstable_by(|left, right| {
        left.token
            .as_bytes()
            .cmp(right.token.as_bytes())
            .then_with(|| left.profile.cmp(&right.profile))
    });
    for pair in ordered.windows(2) {
        if pair[0].token == pair[1].token && pair[0].profile == pair[1].profile {
            return Err(Error::InvalidInput(format!(
                "duplicate broad query plan: {} {:?}",
                pair[0].token, pair[0].profile
            )));
        }
    }

    let mut writer = BroadIndexWriter::create(path)?;
    for plan in ordered {
        writer.append(&plan.token, plan.profile, &plan.groups)?;
    }
    writer.finish(path).map(|_| ())
}

impl BroadIndexWriter {
    fn create(path: &Path) -> Result<Self> {
        let payload_path = path.with_extension("payloads.spool");
        let payload = ColdWriter::create_large_spool(&payload_path)?;
        Ok(Self {
            payload_path,
            payload,
            payload_bytes: 0,
            entries: Vec::new(),
        })
    }

    fn append(
        &mut self,
        token: &str,
        profile: BroadProfile,
        groups: &[Option<VideoMatches>],
    ) -> Result<()> {
        if token.is_empty() || token.len() > usize::from(u16::MAX) {
            return Err(Error::InvalidInput(
                "broad query key has an invalid length".to_string(),
            ));
        }
        if self.entries.last().is_some_and(|previous| {
            (previous.token.as_bytes(), previous.profile) >= (token.as_bytes(), profile)
        }) {
            return Err(Error::InvalidInput(
                "broad build plans are not strictly ordered".to_string(),
            ));
        }
        let payload = codec::encode(&encode_answer(groups)?)?;
        let payload_length = payload.len() as u64;
        self.payload.write_all(&payload)?;
        self.entries.push(BuildEntry {
            token: token.to_string(),
            profile,
            payload_offset: self.payload_bytes,
            payload_length,
        });
        self.payload_bytes = self
            .payload_bytes
            .checked_add(payload_length)
            .ok_or_else(|| Error::InvalidInput("broad payload bytes overflow".to_string()))?;
        Ok(())
    }

    fn finish(self, path: &Path) -> Result<usize> {
        self.payload.finish()?;
        let count = u32::try_from(self.entries.len())
            .map_err(|_| Error::InvalidInput("too many broad query plans".to_string()))?;
        let directory_bytes = self.entries.iter().try_fold(HEADER_BYTES, |total, entry| {
            total
                .checked_add(DIRECTORY_FIXED_BYTES + entry.token.len())
                .ok_or_else(|| Error::InvalidInput("broad directory is too large".to_string()))
        })?;
        let mut output = ColdWriter::create(path)?;
        let mut header = Vec::new();
        write_header(&mut header, BROAD_SINGLE_MAGIC, count);
        header[12..16].copy_from_slice(&3_u32.to_le_bytes());
        output.write_all(&header)?;
        for entry in &self.entries {
            output.write_all(&(entry.token.len() as u16).to_le_bytes())?;
            output.write_all(&[entry.profile as u8, 0])?;
            output.write_all(&(directory_bytes as u64 + entry.payload_offset).to_le_bytes())?;
            output.write_all(&entry.payload_length.to_le_bytes())?;
            output.write_all(entry.token.as_bytes())?;
        }
        let mut payload = BufReader::new(ColdReader::open(&self.payload_path)?);
        std::io::copy(&mut payload, &mut output)?;
        output.finish()?;
        fs::remove_file(&self.payload_path)?;
        Ok(self.entries.len())
    }
}

fn encode_answer(groups: &[Option<VideoMatches>]) -> Result<Vec<u8>> {
    let groups = groups.iter().flatten().collect::<Vec<_>>();
    let total_moments = groups
        .iter()
        .try_fold(0_u64, |total, group| {
            total.checked_add(visible_group_moments(group) as u64)
        })
        .ok_or_else(|| Error::InvalidInput("broad moment total overflows".to_string()))?;
    let mut output = Vec::new();
    output.extend_from_slice(&(groups.len() as u32).to_le_bytes());
    output.extend_from_slice(&total_moments.to_le_bytes());
    output.extend_from_slice(&(groups.len() as u32).to_le_bytes());
    for group in groups {
        output.extend_from_slice(&group.video.to_le_bytes());
        output.extend_from_slice(&group.score.to_bits().to_le_bytes());
        output.push(group.exact_tokens);
        output.push(group.exact_mask);
        output.push(u8::from(group.ordered_phrase));
        output.push(0);
        output.extend_from_slice(&(visible_group_moments(group) as u32).to_le_bytes());
        output.extend_from_slice(&(group.documents.len() as u16).to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        for document in &group.documents {
            output.extend_from_slice(&document.document.to_le_bytes());
            output.extend_from_slice(&document.score.to_bits().to_le_bytes());
            output.push(document.flags);
            output.extend_from_slice(&[0; 3]);
        }
    }
    Ok(output)
}

fn validate_answer(
    bytes: &[u8],
    range: &Range<usize>,
    videos: usize,
    documents: u32,
    retained_depth: usize,
) -> Result<()> {
    let answer = decode_answer(bytes, range)?;
    if answer.found != answer.groups.len() || answer.groups.len() > videos {
        return Err(Error::Corrupt(
            "broad answer group count is invalid".to_string(),
        ));
    }
    if answer.total_moments
        != answer
            .groups
            .iter()
            .map(visible_group_moments)
            .sum::<usize>()
    {
        return Err(Error::Corrupt(
            "broad answer moment total is invalid".to_string(),
        ));
    }
    let mut previous_video = None;
    for group in &answer.groups {
        if group.video as usize >= videos
            || previous_video.is_some_and(|previous| previous >= group.video)
        {
            return Err(Error::Corrupt(
                "broad answer video IDs are invalid".to_string(),
            ));
        }
        if !group.score.is_finite() || group.documents.len() > retained_depth {
            return Err(Error::Corrupt("broad answer rank is invalid".to_string()));
        }
        for document in &group.documents {
            if document.document >= documents || !document.score.is_finite() {
                return Err(Error::Corrupt(
                    "broad answer document is invalid".to_string(),
                ));
            }
        }
        if !group
            .documents
            .is_sorted_by(|left, right| descending_doc_score(left, right) != Ordering::Greater)
        {
            return Err(Error::Corrupt(
                "broad answer documents are not ranked".to_string(),
            ));
        }
        previous_video = Some(group.video);
    }
    Ok(())
}

fn decode_answer(bytes: &[u8], range: &Range<usize>) -> Result<BroadAnswer> {
    let payload = bytes
        .get(range.clone())
        .ok_or_else(|| Error::Corrupt("broad answer is truncated".to_string()))?;
    if payload.len() < ANSWER_HEADER_BYTES {
        return Err(Error::Corrupt(
            "broad answer header is truncated".to_string(),
        ));
    }
    let found = read_u32(payload, 0)? as usize;
    let total_moments = usize::try_from(read_u64(payload, 4)?)
        .map_err(|_| Error::Corrupt("broad moment total overflows".to_string()))?;
    let group_count = read_u32(payload, 12)? as usize;
    if found != group_count {
        return Err(Error::Corrupt(
            "broad answer found count does not match groups".to_string(),
        ));
    }
    if group_count > (payload.len() - ANSWER_HEADER_BYTES) / GROUP_RECORD_BYTES {
        return Err(Error::Corrupt(
            "broad group count exceeds payload bytes".into(),
        ));
    }
    let mut cursor = ANSWER_HEADER_BYTES;
    let mut groups = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        let end = cursor
            .checked_add(GROUP_RECORD_BYTES)
            .ok_or_else(|| Error::Corrupt("broad group offset overflows".to_string()))?;
        payload
            .get(cursor..end)
            .ok_or_else(|| Error::Corrupt("broad group is truncated".to_string()))?;
        let video = read_u32(payload, cursor)?;
        let score = f32::from_bits(read_u32(payload, cursor + 4)?);
        let exact_tokens = read_byte(payload, cursor + 8)?;
        let exact_mask = read_byte(payload, cursor + 9)?;
        let ordered_phrase = match read_byte(payload, cursor + 10)? {
            0 => false,
            1 => true,
            _ => {
                return Err(Error::Corrupt(
                    "broad ordered-phrase flag is invalid".to_string(),
                ));
            }
        };
        if read_byte(payload, cursor + 11)? != 0 || read_u16(payload, cursor + 18)? != 0 {
            return Err(Error::Corrupt(
                "broad group reserved bytes are non-zero".to_string(),
            ));
        }
        let moments = read_u32(payload, cursor + 12)? as usize;
        let document_count = read_u16(payload, cursor + 16)? as usize;
        cursor = end;
        if document_count > (payload.len() - cursor) / DOCUMENT_RECORD_BYTES {
            return Err(Error::Corrupt(
                "broad document count exceeds payload bytes".into(),
            ));
        }
        let mut group_documents = SmallVec::with_capacity(document_count);
        for _ in 0..document_count {
            let end = cursor
                .checked_add(DOCUMENT_RECORD_BYTES)
                .ok_or_else(|| Error::Corrupt("broad document offset overflows".to_string()))?;
            let record = payload
                .get(cursor..end)
                .ok_or_else(|| Error::Corrupt("broad document is truncated".to_string()))?;
            if record[9..12] != [0; 3] {
                return Err(Error::Corrupt(
                    "broad document reserved bytes are non-zero".to_string(),
                ));
            }
            group_documents.push(DocMatch {
                document: read_u32(payload, cursor)?,
                score: f32::from_bits(read_u32(payload, cursor + 4)?),
                flags: read_byte(payload, cursor + 8)?,
            });
            cursor = end;
        }
        groups.push(VideoMatches {
            video,
            score,
            exact_tokens,
            exact_mask,
            ordered_phrase,
            moments: MomentCounter {
                clusters: moments,
                cluster_start_ms: None,
                last_start_ms: None,
            },
            documents: group_documents,
        });
    }
    if cursor != payload.len() {
        return Err(Error::Corrupt(
            "broad answer has trailing bytes".to_string(),
        ));
    }
    Ok(BroadAnswer {
        found,
        total_moments,
        groups,
    })
}

fn read_byte(bytes: &[u8], offset: usize) -> Result<u8> {
    bytes
        .get(offset)
        .copied()
        .ok_or_else(|| Error::Corrupt("broad byte extends beyond file".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_group(video: u32, document: u32, score: f32) -> VideoMatches {
        VideoMatches {
            video,
            score,
            exact_tokens: 1,
            exact_mask: 1,
            ordered_phrase: false,
            moments: MomentCounter {
                clusters: 2,
                cluster_start_ms: None,
                last_start_ms: None,
            },
            documents: SmallVec::from_vec(vec![DocMatch::new(document, score, true)]),
        }
    }

    #[test]
    fn broad_index_round_trips_exact_score_bits_and_totals() {
        let root = std::env::temp_dir().join(format!(
            "ytsearch-broad-index-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(BROAD_SINGLE_FILE);
        let groups = vec![Some(fixture_group(0, 3, 7.25)), None];
        write_index(
            &path,
            &[BroadBuildPlan {
                token: "react".to_string(),
                profile: BroadProfile::TextPosition,
                groups,
            }],
        )
        .unwrap();
        let index = BroadSingleIndex::open(&path, 2, 10).unwrap();
        let fields = query_fields(Some("text"), true).unwrap();
        let answer = index.lookup("react", fields).unwrap().unwrap();
        assert_eq!(answer.found, 1);
        assert_eq!(answer.total_moments, 2);
        assert_eq!(answer.groups[0].score.to_bits(), 7.25_f32.to_bits());
        assert_eq!(answer.groups[0].documents[0].document(), 3);
        assert!(index.lookup("fire", fields).unwrap().is_none());
        assert!(index.supports_depth(5));
        assert!(!index.supports_depth(6));
        drop(index);
        let mut bytes = fs::read(&path).unwrap();
        let payload_start = read_u64(&bytes, HEADER_BYTES + 4).unwrap() as usize;
        let raw_payload = codec::decode(&bytes[payload_start..], PayloadEncoding::Zstd, 176)
            .unwrap()
            .into_owned();
        bytes.truncate(payload_start);
        bytes[HEADER_BYTES + 12..HEADER_BYTES + 20]
            .copy_from_slice(&(raw_payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&raw_payload);
        bytes[12..16].copy_from_slice(&1_u32.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        let legacy = BroadSingleIndex::open(&path, 2, 10).unwrap();
        assert!(legacy.supports_depth(250));
        assert!(!legacy.supports_depth(251));
        drop(legacy);
        bytes[12..16].copy_from_slice(&2_u32.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        let bounded = BroadSingleIndex::open(&path, 2, 10).unwrap();
        assert!(bounded.supports_depth(5));
        assert!(!bounded.supports_depth(6));
        assert_eq!(
            bounded.lookup("react", fields).unwrap().unwrap().groups[0]
                .score
                .to_bits(),
            7.25_f32.to_bits()
        );
        drop(bounded);
        bytes[12..16].copy_from_slice(&4_u32.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(BroadSingleIndex::open(&path, 2, 10).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn broad_index_rejects_corrupt_reserved_bytes() {
        let root = std::env::temp_dir().join(format!(
            "ytsearch-broad-corrupt-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(BROAD_SINGLE_FILE);
        let groups = vec![Some(fixture_group(0, 0, 1.0))];
        write_index(
            &path,
            &[BroadBuildPlan {
                token: "fire".to_string(),
                profile: BroadProfile::TextFlat,
                groups,
            }],
        )
        .unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[HEADER_BYTES + 3] = 1;
        fs::write(&path, bytes).unwrap();
        assert!(BroadSingleIndex::open(&path, 1, 1).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
