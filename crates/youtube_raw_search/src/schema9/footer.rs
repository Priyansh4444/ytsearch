//! Versioned schema-9 directory metadata. The digest detects corruption only;
//! callers must bind the exact directory digest to a trusted generation manifest.

use super::verified_units::{MAX_UNIT_BYTES, MIN_UNIT_BYTES};
use std::collections::BTreeMap;

const MAGIC: &[u8; 8] = b"YTS9DIR\0";
const SCHEMA_VERSION: u16 = 9;
const DIRECTORY_VERSION: u16 = 1;
const HEADER_BYTES: usize = 100;
const RECORD_BYTES: usize = 20;
const DIGEST_BYTES: usize = 32;
pub(crate) const MAX_DIRECTORY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u16)]
pub(crate) enum FileKind {
    Terms = 1,
    Postings = 2,
    Skips = 3,
    RankBounds = 4,
    Bitmaps = 5,
    Positions = 6,
    Documents = 7,
    Videos = 8,
    Facets = 9,
    Payloads = 10,
    Channels = 11,
}

impl FileKind {
    const ALL: [Self; 11] = [
        Self::Terms,
        Self::Postings,
        Self::Skips,
        Self::RankBounds,
        Self::Bitmaps,
        Self::Positions,
        Self::Documents,
        Self::Videos,
        Self::Facets,
        Self::Payloads,
        Self::Channels,
    ];

    fn from_wire(value: u16) -> Result<Self, DirectoryError> {
        Self::ALL
            .into_iter()
            .find(|kind| *kind as u16 == value)
            .ok_or(DirectoryError::UnknownFileKind(value))
    }

    fn required(self) -> bool {
        self != Self::Bitmaps
    }

    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::Terms => "terms.bin",
            Self::Postings => "postings.bin",
            Self::Skips => "skips.bin",
            Self::RankBounds => "rank-bounds.bin",
            Self::Bitmaps => "bitmaps.bin",
            Self::Positions => "positions.bin",
            Self::Documents => "documents.bin",
            Self::Videos => "videos.bin",
            Self::Facets => "facets.bin",
            Self::Payloads => "payloads.bin",
            Self::Channels => "channels.bin",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileRecord {
    pub(crate) kind: FileKind,
    /// Raw outer file. Individual posting/payload blocks may carry internal codecs.
    pub(crate) length: u64,
    pub(crate) unit_bytes: u32,
    pub(crate) unit_digests: Vec<[u8; DIGEST_BYTES]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Directory {
    pub(crate) normalization_version: u16,
    pub(crate) rank_version: u16,
    pub(crate) generation: u64,
    pub(crate) mutation_sequence: u64,
    pub(crate) corpus_digest: [u8; DIGEST_BYTES],
    pub(crate) document_count: u64,
    pub(crate) video_count: u64,
    pub(crate) term_count: u64,
    pub(crate) bitmap_budget_bytes: u64,
    pub(crate) files: Vec<FileRecord>,
}

/// Values supplied by the trusted generation manifest, not by directory.bin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectoryBinding {
    pub(crate) generation: u64,
    pub(crate) mutation_sequence: u64,
    pub(crate) corpus_digest: [u8; DIGEST_BYTES],
    pub(crate) directory_digest: [u8; DIGEST_BYTES],
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum DirectoryError {
    #[error("directory exceeds 16 MiB or has invalid length")]
    Size,
    #[error("directory magic or version is unsupported")]
    Version,
    #[error("directory contains unknown file kind {0}")]
    UnknownFileKind(u16),
    #[error("directory contains unsupported outer codec {0}")]
    UnsupportedCodec(u16),
    #[error("directory records are unordered, duplicated, or missing a required kind")]
    FileKinds,
    #[error("directory field or unit count is invalid")]
    Metadata,
    #[error("directory digest differs from encoded bytes")]
    Digest,
    #[error("observed file lengths differ from directory records")]
    FileLength,
    #[error("directory identity differs from generation manifest")]
    Identity,
}

impl Directory {
    pub(crate) fn encode(&self) -> Result<Vec<u8>, DirectoryError> {
        self.validate()?;
        let length = self.encoded_length()?;
        let mut out = Vec::with_capacity(length);
        out.extend_from_slice(MAGIC);
        put_u16(&mut out, SCHEMA_VERSION);
        put_u16(&mut out, DIRECTORY_VERSION);
        put_u16(&mut out, self.normalization_version);
        put_u16(&mut out, self.rank_version);
        put_u64(&mut out, self.generation);
        put_u64(&mut out, self.mutation_sequence);
        out.extend_from_slice(&self.corpus_digest);
        put_u64(&mut out, self.document_count);
        put_u64(&mut out, self.video_count);
        put_u64(&mut out, self.term_count);
        put_u64(&mut out, self.bitmap_budget_bytes);
        put_u16(
            &mut out,
            u16::try_from(self.files.len()).map_err(|_| DirectoryError::Size)?,
        );
        put_u16(&mut out, 0); // reserved; readers reject nonzero
        debug_assert_eq!(out.len(), HEADER_BYTES);
        for file in &self.files {
            put_u16(&mut out, file.kind as u16);
            put_u16(&mut out, 0); // raw outer file
            put_u64(&mut out, file.length);
            put_u32(&mut out, file.unit_bytes);
            put_u32(
                &mut out,
                u32::try_from(file.unit_digests.len()).map_err(|_| DirectoryError::Size)?,
            );
            for digest in &file.unit_digests {
                out.extend_from_slice(digest);
            }
        }
        debug_assert_eq!(out.len(), length - DIGEST_BYTES);
        out.extend_from_slice(blake3::hash(&out).as_bytes());
        Ok(out)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DirectoryError> {
        if !(HEADER_BYTES + DIGEST_BYTES..=MAX_DIRECTORY_BYTES).contains(&bytes.len()) {
            return Err(DirectoryError::Size);
        }
        let payload_len = bytes.len() - DIGEST_BYTES;
        if blake3::hash(&bytes[..payload_len]).as_bytes() != &bytes[payload_len..] {
            return Err(DirectoryError::Digest);
        }
        let mut cursor = Cursor::new(&bytes[..payload_len]);
        if cursor.take::<8>()? != *MAGIC
            || cursor.u16()? != SCHEMA_VERSION
            || cursor.u16()? != DIRECTORY_VERSION
        {
            return Err(DirectoryError::Version);
        }
        let normalization_version = cursor.u16()?;
        let rank_version = cursor.u16()?;
        let generation = cursor.u64()?;
        let mutation_sequence = cursor.u64()?;
        let corpus_digest = cursor.take::<DIGEST_BYTES>()?;
        let document_count = cursor.u64()?;
        let video_count = cursor.u64()?;
        let term_count = cursor.u64()?;
        let bitmap_budget_bytes = cursor.u64()?;
        let count = usize::from(cursor.u16()?);
        if cursor.u16()? != 0 {
            return Err(DirectoryError::Metadata);
        }
        let mut files = Vec::with_capacity(count.min(FileKind::ALL.len()));
        if count > FileKind::ALL.len() {
            return Err(DirectoryError::FileKinds);
        }
        for _ in 0..count {
            let kind = FileKind::from_wire(cursor.u16()?)?;
            let codec = cursor.u16()?;
            if codec != 0 {
                return Err(DirectoryError::UnsupportedCodec(codec));
            }
            let length = cursor.u64()?;
            let unit_bytes = cursor.u32()?;
            let unit_count =
                usize::try_from(cursor.u32()?).map_err(|_| DirectoryError::Metadata)?;
            let digest_bytes = unit_count
                .checked_mul(DIGEST_BYTES)
                .ok_or(DirectoryError::Size)?;
            if digest_bytes > cursor.remaining() {
                return Err(DirectoryError::Size);
            }
            let mut unit_digests = Vec::with_capacity(unit_count);
            for _ in 0..unit_count {
                unit_digests.push(cursor.take::<DIGEST_BYTES>()?);
            }
            files.push(FileRecord {
                kind,
                length,
                unit_bytes,
                unit_digests,
            });
        }
        if cursor.remaining() != 0 {
            return Err(DirectoryError::Size);
        }
        let directory = Self {
            normalization_version,
            rank_version,
            generation,
            mutation_sequence,
            corpus_digest,
            document_count,
            video_count,
            term_count,
            bitmap_budget_bytes,
            files,
        };
        directory.validate()?;
        Ok(directory)
    }

    pub(crate) fn digest(encoded: &[u8]) -> Result<[u8; DIGEST_BYTES], DirectoryError> {
        Self::decode(encoded)?;
        encoded[encoded.len() - DIGEST_BYTES..]
            .try_into()
            .map_err(|_| DirectoryError::Size)
    }

    pub(crate) fn decode_bound(
        encoded: &[u8],
        binding: &DirectoryBinding,
    ) -> Result<Self, DirectoryError> {
        let directory = Self::decode(encoded)?;
        if directory.generation != binding.generation
            || directory.mutation_sequence != binding.mutation_sequence
            || directory.corpus_digest != binding.corpus_digest
            || encoded[encoded.len() - DIGEST_BYTES..] != binding.directory_digest
        {
            return Err(DirectoryError::Identity);
        }
        Ok(directory)
    }

    /// Check the complete observed file set, including absence of optional files.
    pub(crate) fn validate_file_lengths(
        &self,
        observed: &[(FileKind, u64)],
    ) -> Result<(), DirectoryError> {
        let unique: BTreeMap<_, _> = observed.iter().copied().collect();
        if observed.len() != self.files.len()
            || unique.len() != observed.len()
            || !self
                .files
                .iter()
                .all(|file| unique.get(&file.kind) == Some(&file.length))
        {
            return Err(DirectoryError::FileLength);
        }
        Ok(())
    }

    fn encoded_length(&self) -> Result<usize, DirectoryError> {
        let mut length = HEADER_BYTES
            .checked_add(DIGEST_BYTES)
            .ok_or(DirectoryError::Size)?;
        for file in &self.files {
            length = length
                .checked_add(RECORD_BYTES)
                .ok_or(DirectoryError::Size)?;
            length = length
                .checked_add(
                    file.unit_digests
                        .len()
                        .checked_mul(DIGEST_BYTES)
                        .ok_or(DirectoryError::Size)?,
                )
                .ok_or(DirectoryError::Size)?;
            if length > MAX_DIRECTORY_BYTES {
                return Err(DirectoryError::Size);
            }
        }
        Ok(length)
    }

    fn validate(&self) -> Result<(), DirectoryError> {
        self.encoded_length()?;
        if self.files.len() > FileKind::ALL.len()
            || self.document_count > u32::MAX as u64
            || self.video_count > u32::MAX as u64
            || self.normalization_version == 0
            || self.rank_version == 0
            || self.generation == 0
            || self.corpus_digest == [0; DIGEST_BYTES]
        {
            return Err(DirectoryError::Metadata);
        }
        let mut previous = 0;
        for file in &self.files {
            let id = file.kind as u16;
            if id <= previous {
                return Err(DirectoryError::FileKinds);
            }
            previous = id;
            if !(MIN_UNIT_BYTES..=MAX_UNIT_BYTES).contains(&(file.unit_bytes as usize)) {
                return Err(DirectoryError::Metadata);
            }
            if file.kind.required() && file.length == 0 {
                return Err(DirectoryError::Metadata);
            }
            let expected = if file.length == 0 {
                0
            } else {
                1 + (file.length - 1) / u64::from(file.unit_bytes)
            };
            if u64::try_from(file.unit_digests.len()).map_err(|_| DirectoryError::Metadata)?
                != expected
            {
                return Err(DirectoryError::Metadata);
            }
        }
        for required in FileKind::ALL.into_iter().filter(|kind| kind.required()) {
            if !self.files.iter().any(|file| file.kind == required) {
                return Err(DirectoryError::FileKinds);
            }
        }
        Ok(())
    }
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
    fn take<const N: usize>(&mut self) -> Result<[u8; N], DirectoryError> {
        let end = self.position.checked_add(N).ok_or(DirectoryError::Size)?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or(DirectoryError::Size)?;
        self.position = end;
        slice.try_into().map_err(|_| DirectoryError::Size)
    }
    fn u16(&mut self) -> Result<u16, DirectoryError> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, DirectoryError> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> Result<u64, DirectoryError> {
        Ok(u64::from_le_bytes(self.take()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Directory {
        Directory {
            normalization_version: 2,
            rank_version: 2,
            generation: 7,
            mutation_sequence: 81,
            corpus_digest: [9; DIGEST_BYTES],
            document_count: 4,
            video_count: 2,
            term_count: 3,
            bitmap_budget_bytes: 64 * 1024 * 1024,
            files: FileKind::ALL
                .into_iter()
                .filter(|kind| kind.required())
                .map(|kind| FileRecord {
                    kind,
                    length: MIN_UNIT_BYTES as u64 + 1,
                    unit_bytes: MIN_UNIT_BYTES as u32,
                    unit_digests: vec![[kind as u8; DIGEST_BYTES]; 2],
                })
                .collect(),
        }
    }

    fn reseal(bytes: &mut [u8]) {
        let end = bytes.len() - DIGEST_BYTES;
        let digest = *blake3::hash(&bytes[..end]).as_bytes();
        bytes[end..].copy_from_slice(&digest);
    }

    #[test]
    fn round_trip_and_exact_lengths() {
        let directory = fixture();
        let bytes = directory.encode().unwrap();
        assert_eq!(
            bytes.len(),
            HEADER_BYTES + 10 * (RECORD_BYTES + 2 * DIGEST_BYTES) + DIGEST_BYTES
        );
        assert_eq!(Directory::decode(&bytes).unwrap(), directory);
        assert_eq!(
            Directory::digest(&bytes).unwrap(),
            bytes[bytes.len() - DIGEST_BYTES..]
        );
        let observed: Vec<_> = directory
            .files
            .iter()
            .map(|file| (file.kind, file.length))
            .collect();
        assert_eq!(directory.validate_file_lengths(&observed), Ok(()));
        let mut reversed = observed.clone();
        reversed.reverse();
        assert_eq!(directory.validate_file_lengths(&reversed), Ok(()));
        let mut wrong = observed.clone();
        wrong[0].1 += 1;
        assert_eq!(
            directory.validate_file_lengths(&wrong),
            Err(DirectoryError::FileLength)
        );
        assert_eq!(
            directory.validate_file_lengths(&observed[1..]),
            Err(DirectoryError::FileLength)
        );
        let mut duplicate = observed.clone();
        duplicate[0] = duplicate[1];
        assert_eq!(
            directory.validate_file_lengths(&duplicate),
            Err(DirectoryError::FileLength)
        );
    }

    #[test]
    fn fixed_offsets_and_little_endian_are_independent_of_decoder() {
        let bytes = fixture().encode().unwrap();
        assert_eq!(&bytes[0..8], b"YTS9DIR\0");
        assert_eq!(&bytes[8..16], &[9, 0, 1, 0, 2, 0, 2, 0]);
        assert_eq!(&bytes[16..24], &7_u64.to_le_bytes());
        assert_eq!(&bytes[24..32], &81_u64.to_le_bytes());
        assert_eq!(&bytes[32..64], &[9; 32]);
        assert_eq!(&bytes[64..72], &4_u64.to_le_bytes());
        assert_eq!(&bytes[72..80], &2_u64.to_le_bytes());
        assert_eq!(&bytes[80..88], &3_u64.to_le_bytes());
        assert_eq!(&bytes[88..96], &(64_u64 * 1024 * 1024).to_le_bytes());
        assert_eq!(&bytes[96..100], &[10, 0, 0, 0]);
        assert_eq!(&bytes[100..104], &[1, 0, 0, 0]);
        assert_eq!(&bytes[104..112], &(MIN_UNIT_BYTES as u64 + 1).to_le_bytes());
        assert_eq!(&bytes[112..116], &(MIN_UNIT_BYTES as u32).to_le_bytes());
        assert_eq!(&bytes[116..120], &[2, 0, 0, 0]);
    }

    #[test]
    fn rejects_corruption_truncation_extension_and_excessive_metadata() {
        let bytes = fixture().encode().unwrap();
        let mut corrupt = bytes.clone();
        corrupt[24] ^= 1;
        assert_eq!(Directory::decode(&corrupt), Err(DirectoryError::Digest));
        for end in [0, HEADER_BYTES, bytes.len() - 1] {
            assert!(Directory::decode(&bytes[..end]).is_err());
        }
        let mut extended = bytes.clone();
        extended.push(0);
        reseal(&mut extended);
        assert_eq!(Directory::decode(&extended), Err(DirectoryError::Size));
        assert_eq!(
            Directory::decode(&vec![0; MAX_DIRECTORY_BYTES + 1]),
            Err(DirectoryError::Size)
        );
    }

    #[test]
    fn rejects_wrong_version_reserved_kind_codec_and_order() {
        let bytes = fixture().encode().unwrap();
        for (offset, value, expected) in [
            (8, 8_u16, DirectoryError::Version),
            (10, 2, DirectoryError::Version),
            (98, 1, DirectoryError::Metadata),
            (HEADER_BYTES, 99, DirectoryError::UnknownFileKind(99)),
            (HEADER_BYTES + 2, 1, DirectoryError::UnsupportedCodec(1)),
            (
                HEADER_BYTES + RECORD_BYTES + 2 * DIGEST_BYTES,
                1,
                DirectoryError::FileKinds,
            ),
        ] {
            let mut modified = bytes.clone();
            modified[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            reseal(&mut modified);
            assert_eq!(Directory::decode(&modified), Err(expected));
        }
    }

    #[test]
    fn rejects_unit_count_size_and_required_omission() {
        let mut directory = fixture();
        directory.files[0].unit_digests.pop();
        assert_eq!(directory.encode(), Err(DirectoryError::Metadata));
        directory = fixture();
        directory.files[0].unit_bytes = (MIN_UNIT_BYTES - 1) as u32;
        assert_eq!(directory.encode(), Err(DirectoryError::Metadata));
        directory = fixture();
        directory.files.remove(0);
        assert_eq!(directory.encode(), Err(DirectoryError::FileKinds));
        directory = fixture();
        directory.files[0].length = u64::MAX;
        assert_eq!(directory.encode(), Err(DirectoryError::Metadata));
    }

    #[test]
    fn exact_unit_boundaries_and_empty_optional_file() {
        let mut directory = fixture();
        for length in [
            1,
            MIN_UNIT_BYTES as u64,
            MIN_UNIT_BYTES as u64 + 1,
            2 * MIN_UNIT_BYTES as u64,
        ] {
            directory.files[0].length = length;
            directory.files[0].unit_digests =
                vec![
                    [1; DIGEST_BYTES];
                    usize::try_from(1 + (length - 1) / MIN_UNIT_BYTES as u64).unwrap()
                ];
            assert_eq!(
                Directory::decode(&directory.encode().unwrap()).unwrap(),
                directory
            );
        }
        directory.files.insert(
            4,
            FileRecord {
                kind: FileKind::Bitmaps,
                length: 0,
                unit_bytes: MAX_UNIT_BYTES as u32,
                unit_digests: vec![],
            },
        );
        assert_eq!(
            Directory::decode(&directory.encode().unwrap()).unwrap(),
            directory
        );
        directory.files[4].unit_digests.push([0; DIGEST_BYTES]);
        assert_eq!(directory.encode(), Err(DirectoryError::Metadata));
    }

    #[test]
    fn file_substitution_needs_manifest_binding_even_with_valid_checksum() {
        let a = fixture().encode().unwrap();
        let mut substituted = fixture();
        substituted.generation += 1;
        let b = substituted.encode().unwrap();
        assert!(Directory::decode(&a).is_ok());
        assert!(Directory::decode(&b).is_ok());
        assert_ne!(
            Directory::digest(&a).unwrap(),
            Directory::digest(&b).unwrap()
        );
        let binding = DirectoryBinding {
            generation: 7,
            mutation_sequence: 81,
            corpus_digest: [9; DIGEST_BYTES],
            directory_digest: Directory::digest(&a).unwrap(),
        };
        assert_eq!(Directory::decode_bound(&a, &binding), Ok(fixture()));
        assert_eq!(
            Directory::decode_bound(&b, &binding),
            Err(DirectoryError::Identity)
        );
        for changed in [
            DirectoryBinding {
                generation: 8,
                ..binding.clone()
            },
            DirectoryBinding {
                mutation_sequence: 82,
                ..binding.clone()
            },
            DirectoryBinding {
                corpus_digest: [8; DIGEST_BYTES],
                ..binding.clone()
            },
            DirectoryBinding {
                directory_digest: [8; DIGEST_BYTES],
                ..binding.clone()
            },
        ] {
            assert_eq!(
                Directory::decode_bound(&a, &changed),
                Err(DirectoryError::Identity)
            );
        }
    }
}
