//! Versioned schema-9 directory metadata. The digest detects corruption only;
//! callers must bind the exact directory digest to a trusted generation manifest.

use super::verified_units::{MAX_UNIT_BYTES, MIN_UNIT_BYTES, UnitError, VerifiedUnits};
use memmap2::{Mmap, MmapOptions};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path, sync::OnceLock};

const MAGIC: &[u8; 8] = b"YTS9DIR\0";
const SCHEMA_VERSION: u16 = 9;
const DIRECTORY_VERSION: u16 = 1;
const HEADER_BYTES: usize = 100;
const RECORD_BYTES: usize = 20;
const DIGEST_BYTES: usize = 32;
const CORPUS_IDENTITY_DOMAIN: &[u8] = b"ytsearch schema9 corpus identity v1\0";
pub(crate) const MAX_DIRECTORY_BYTES: usize = 16 * 1024 * 1024;

/// Combine the captured mutation snapshot with its immutable base archives.
/// Inputs are decoded BLAKE3 digest bytes from the publication job.
pub(crate) fn corpus_digest(
    base_corpus: &[u8; DIGEST_BYTES],
    base_chunks: &[u8; DIGEST_BYTES],
    base_channels: &[u8; DIGEST_BYTES],
    snapshot: &[u8; DIGEST_BYTES],
    mutation_sequence: u64,
) -> [u8; DIGEST_BYTES] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CORPUS_IDENTITY_DOMAIN);
    hasher.update(base_corpus);
    hasher.update(base_chunks);
    hasher.update(base_channels);
    hasher.update(snapshot);
    hasher.update(&mutation_sequence.to_le_bytes());
    *hasher.finalize().as_bytes()
}

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
    pub(crate) const ALL: [Self; 11] = [
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

    pub(crate) fn required(self) -> bool {
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

impl FileRecord {
    /// Hash a completed immutable data file using one bounded scratch buffer.
    pub(crate) fn from_file(
        kind: FileKind,
        path: &Path,
        unit_bytes: usize,
    ) -> Result<Self, DirectoryError> {
        if !(MIN_UNIT_BYTES..=MAX_UNIT_BYTES).contains(&unit_bytes) {
            return Err(DirectoryError::Metadata);
        }
        let mut file = File::open(path).map_err(|_| DirectoryError::Io)?;
        let length = file.metadata().map_err(|_| DirectoryError::Io)?.len();
        if kind.required() && length == 0 {
            return Err(DirectoryError::Metadata);
        }
        let unit_count = if length == 0 {
            0
        } else {
            1 + (length - 1) / unit_bytes as u64
        };
        let capacity = usize::try_from(unit_count).map_err(|_| DirectoryError::Size)?;
        if capacity > MAX_DIRECTORY_BYTES / DIGEST_BYTES {
            return Err(DirectoryError::Size);
        }
        let mut unit_digests = Vec::with_capacity(capacity);
        let mut buffer = vec![0_u8; if length == 0 { 0 } else { unit_bytes }];
        let mut remaining = length;
        while remaining != 0 {
            let bytes = usize::try_from(remaining.min(unit_bytes as u64))
                .map_err(|_| DirectoryError::Size)?;
            file.read_exact(&mut buffer[..bytes])
                .map_err(|_| DirectoryError::Io)?;
            unit_digests.push(*blake3::hash(&buffer[..bytes]).as_bytes());
            remaining -= bytes as u64;
        }
        let mut extra = [0_u8];
        if file.read(&mut extra).map_err(|_| DirectoryError::Io)? != 0 {
            return Err(DirectoryError::FileLength);
        }
        Ok(Self {
            kind,
            length,
            unit_bytes: unit_bytes as u32,
            unit_digests,
        })
    }
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

/// Holds the exact file handles whose lengths were checked at cold open.
pub(crate) struct OpenedGeneration {
    pub(crate) directory: Directory,
    pub(crate) files: BTreeMap<FileKind, File>,
    verifiers: BTreeMap<FileKind, OnceLock<Result<VerifiedUnits<GenerationBytes>, DirectoryError>>>,
}

pub(crate) enum GenerationBytes {
    Empty,
    Mapped(Mmap),
}

impl AsRef<[u8]> for GenerationBytes {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Empty => &[],
            Self::Mapped(map) => map,
        }
    }
}

impl OpenedGeneration {
    pub(crate) fn open_bound(
        root: &Path,
        binding: &DirectoryBinding,
    ) -> Result<Self, DirectoryError> {
        let directory_file =
            File::open(root.join("directory.bin")).map_err(|_| DirectoryError::Io)?;
        let directory_len = directory_file
            .metadata()
            .map_err(|_| DirectoryError::Io)?
            .len();
        if directory_len > MAX_DIRECTORY_BYTES as u64 {
            return Err(DirectoryError::Size);
        }
        let mut encoded = Vec::with_capacity(directory_len as usize);
        directory_file
            .take((MAX_DIRECTORY_BYTES + 1) as u64)
            .read_to_end(&mut encoded)
            .map_err(|_| DirectoryError::Io)?;
        let directory = Directory::decode_bound(&encoded, binding)?;
        let mut files = BTreeMap::new();
        for record in &directory.files {
            let file =
                File::open(root.join(record.kind.file_name())).map_err(|_| DirectoryError::Io)?;
            let metadata = file.metadata().map_err(|_| DirectoryError::Io)?;
            if !metadata.is_file() || metadata.len() != record.length {
                return Err(DirectoryError::FileLength);
            }
            files.insert(record.kind, file);
        }
        for kind in FileKind::ALL {
            if !files.contains_key(&kind) {
                match root.join(kind.file_name()).symlink_metadata() {
                    Ok(_) => return Err(DirectoryError::FileLength),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(DirectoryError::Io),
                }
            }
        }
        let verifiers = directory
            .files
            .iter()
            .map(|record| (record.kind, OnceLock::new()))
            .collect();
        Ok(Self {
            directory,
            files,
            verifiers,
        })
    }

    pub(crate) fn verified_file(
        &self,
        kind: FileKind,
    ) -> Result<&VerifiedUnits<GenerationBytes>, DirectoryError> {
        self.verifiers
            .get(&kind)
            .ok_or(DirectoryError::FileLength)?
            .get_or_init(|| self.map_verified_file(kind))
            .as_ref()
            .map_err(Clone::clone)
    }

    fn map_verified_file(
        &self,
        kind: FileKind,
    ) -> Result<VerifiedUnits<GenerationBytes>, DirectoryError> {
        let record = self
            .directory
            .files
            .iter()
            .find(|record| record.kind == kind)
            .ok_or(DirectoryError::FileLength)?;
        let file = self.files.get(&kind).ok_or(DirectoryError::FileLength)?;
        let bytes = if record.length == 0 {
            GenerationBytes::Empty
        } else {
            // SAFETY: the generation's files must remain immutable while any
            // mapped reader exists. Publication and cleanup own that invariant.
            GenerationBytes::Mapped(
                unsafe { MmapOptions::new().map(file) }.map_err(|_| DirectoryError::Io)?,
            )
        };
        VerifiedUnits::new(
            bytes,
            record.unit_bytes as usize,
            record.unit_digests.clone(),
        )
        .map_err(DirectoryError::Unit)
    }
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
    #[error("unpublished generation contains an unexpected file")]
    UnexpectedFile,
    #[error("directory identity differs from generation manifest")]
    Identity,
    #[error("directory data file cannot be read")]
    Io,
    #[error(transparent)]
    Unit(#[from] UnitError),
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

    #[test]
    fn corpus_identity_binds_each_source_and_captured_sequence() {
        let inputs = ([1; 32], [2; 32], [3; 32], [4; 32], 81_u64);
        let expected = corpus_digest(&inputs.0, &inputs.1, &inputs.2, &inputs.3, inputs.4);
        assert_eq!(
            expected,
            corpus_digest(&inputs.0, &inputs.1, &inputs.2, &inputs.3, inputs.4)
        );
        for different in [
            corpus_digest(&[9; 32], &inputs.1, &inputs.2, &inputs.3, inputs.4),
            corpus_digest(&inputs.0, &[9; 32], &inputs.2, &inputs.3, inputs.4),
            corpus_digest(&inputs.0, &inputs.1, &[9; 32], &inputs.3, inputs.4),
            corpus_digest(&inputs.0, &inputs.1, &inputs.2, &[9; 32], inputs.4),
            corpus_digest(&inputs.0, &inputs.1, &inputs.2, &inputs.3, inputs.4 + 1),
        ] {
            assert_ne!(different, expected);
        }
    }

    #[test]
    fn file_record_hashes_exact_final_unit_and_empty_optional_file() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("postings.bin");
        let mut contents = vec![7_u8; MIN_UNIT_BYTES];
        contents.push(8);
        std::fs::write(&path, &contents).unwrap();
        let record = FileRecord::from_file(FileKind::Postings, &path, MIN_UNIT_BYTES).unwrap();
        assert_eq!(record.length, contents.len() as u64);
        assert_eq!(record.unit_digests.len(), 2);
        assert_eq!(
            record.unit_digests[0],
            *blake3::hash(&contents[..MIN_UNIT_BYTES]).as_bytes()
        );
        assert_eq!(record.unit_digests[1], *blake3::hash(&[8]).as_bytes());
        std::fs::write(&path, []).unwrap();
        assert_eq!(
            FileRecord::from_file(FileKind::Postings, &path, MIN_UNIT_BYTES),
            Err(DirectoryError::Metadata)
        );
        let empty = FileRecord::from_file(FileKind::Bitmaps, &path, MIN_UNIT_BYTES).unwrap();
        assert_eq!(empty.length, 0);
        assert!(empty.unit_digests.is_empty());
    }

    #[test]
    fn cold_open_binds_directory_and_retains_checked_file_handles() {
        let temporary = tempfile::tempdir().unwrap();
        let mut directory = fixture();
        for record in &mut directory.files {
            record.length = 1;
            record.unit_digests = vec![*blake3::hash(&[record.kind as u8]).as_bytes()];
            std::fs::write(
                temporary.path().join(record.kind.file_name()),
                [record.kind as u8],
            )
            .unwrap();
        }
        let encoded = directory.encode().unwrap();
        std::fs::write(temporary.path().join("directory.bin"), &encoded).unwrap();
        let binding = DirectoryBinding {
            generation: directory.generation,
            mutation_sequence: directory.mutation_sequence,
            corpus_digest: directory.corpus_digest,
            directory_digest: Directory::digest(&encoded).unwrap(),
        };
        let mut opened = OpenedGeneration::open_bound(temporary.path(), &binding).unwrap();
        assert_eq!(opened.directory, directory);
        assert_eq!(opened.files.len(), 10);
        assert_eq!(
            opened.verified_file(FileKind::Terms).unwrap().read(0, 1),
            Ok(&[FileKind::Terms as u8][..])
        );
        std::fs::write(temporary.path().join(FileKind::Postings.file_name()), [99]).unwrap();
        let corrupt = opened.verified_file(FileKind::Postings).unwrap();
        assert_eq!(corrupt.read(0, 1), Err(UnitError::Corrupt(0)));
        assert_eq!(corrupt.read(0, 1), Err(UnitError::Corrupt(0)));
        assert!(std::ptr::eq(
            corrupt,
            opened.verified_file(FileKind::Postings).unwrap()
        ));
        std::fs::write(temporary.path().join(FileKind::Bitmaps.file_name()), []).unwrap();
        assert!(matches!(
            OpenedGeneration::open_bound(temporary.path(), &binding),
            Err(DirectoryError::FileLength)
        ));
        std::fs::remove_file(temporary.path().join(FileKind::Bitmaps.file_name())).unwrap();
        std::fs::remove_file(temporary.path().join(FileKind::Terms.file_name())).unwrap();
        let mut byte = [0];
        opened
            .files
            .get_mut(&FileKind::Terms)
            .unwrap()
            .read_exact(&mut byte)
            .unwrap();
        assert_eq!(byte, [FileKind::Terms as u8]);
        assert!(matches!(
            OpenedGeneration::open_bound(temporary.path(), &binding),
            Err(DirectoryError::Io)
        ));
        std::fs::remove_dir_all(temporary.path()).unwrap();
        assert_eq!(
            opened.verified_file(FileKind::Terms).unwrap().read(0, 1),
            Ok(&[FileKind::Terms as u8][..])
        );
    }

    #[test]
    fn declared_empty_bitmap_opens_without_mapping_an_empty_file() {
        let temporary = tempfile::tempdir().unwrap();
        let mut directory = fixture();
        for record in &mut directory.files {
            record.length = 1;
            record.unit_digests = vec![*blake3::hash(&[record.kind as u8]).as_bytes()];
            std::fs::write(
                temporary.path().join(record.kind.file_name()),
                [record.kind as u8],
            )
            .unwrap();
        }
        directory.files.insert(
            4,
            FileRecord {
                kind: FileKind::Bitmaps,
                length: 0,
                unit_bytes: MIN_UNIT_BYTES as u32,
                unit_digests: vec![],
            },
        );
        std::fs::write(temporary.path().join(FileKind::Bitmaps.file_name()), []).unwrap();
        let encoded = directory.encode().unwrap();
        std::fs::write(temporary.path().join("directory.bin"), &encoded).unwrap();
        let binding = DirectoryBinding {
            generation: directory.generation,
            mutation_sequence: directory.mutation_sequence,
            corpus_digest: directory.corpus_digest,
            directory_digest: Directory::digest(&encoded).unwrap(),
        };
        let opened = OpenedGeneration::open_bound(temporary.path(), &binding).unwrap();
        let bitmap = opened.verified_file(FileKind::Bitmaps).unwrap();
        assert_eq!(bitmap.read(0, 0), Ok(&[][..]));
        assert_eq!(bitmap.read(0, 1), Err(UnitError::OutOfBounds));
    }
}
