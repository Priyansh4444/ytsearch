//! Bounded schema-9 fast-field headers and the document summary spine.

use super::{
    footer::{DirectoryError, FileKind, GenerationBytes, OpenedGeneration},
    sets::{DocId, SetId, VideoId},
    verified_units::VerifiedUnits,
};

const HEADER_BYTES: usize = 64;
const DESCRIPTOR_BYTES: usize = 32;
const DOCUMENT_WIDTHS: [u32; 6] = [8, 4, 1, 8, 4, 0];
const VIDEO_WIDTHS: [u32; 14] = [4, 1, 4, 4, 8, 8, 8, 8, 1, 8, 4, 0, 4, 4];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Section {
    offset: u64,
    length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum FastFieldError {
    #[error("fast-field header or section directory is malformed")]
    Format,
    #[error("fast-field row is outside its declared universe")]
    Range,
    #[error("fast-field document references an unknown video")]
    Video,
    #[error(transparent)]
    Directory(#[from] DirectoryError),
    #[error(transparent)]
    Unit(#[from] super::verified_units::UnitError),
}

struct Table<'g> {
    bytes: &'g VerifiedUnits<GenerationBytes>,
    count: u64,
    sections: Vec<Section>,
}

impl<'g> Table<'g> {
    fn open(
        bytes: &'g VerifiedUnits<GenerationBytes>,
        file_length: u64,
        magic: &[u8; 8],
        widths: &[u32],
        count: u64,
        generation: u64,
        mutation_sequence: u64,
    ) -> Result<Self, FastFieldError> {
        let header = bytes.read(0, HEADER_BYTES)?;
        if header.get(..8) != Some(magic.as_slice())
            || u16_at(header, 8)? != 9
            || u16_at(header, 10)? != 1
            || u16_at(header, 12)? != HEADER_BYTES as u16
            || u16_at(header, 14)? != DESCRIPTOR_BYTES as u16
            || u64_at(header, 16)? != count
            || u32_at(header, 24)? != widths.len() as u32
            || u32_at(header, 28)? != 0
            || u64_at(header, 32)? != generation
            || u64_at(header, 40)? != mutation_sequence
            || header[48..64] != [0; 16]
        {
            return Err(FastFieldError::Format);
        }
        let descriptor_end = HEADER_BYTES
            .checked_add(
                widths
                    .len()
                    .checked_mul(DESCRIPTOR_BYTES)
                    .ok_or(FastFieldError::Format)?,
            )
            .ok_or(FastFieldError::Format)?;
        let descriptors = bytes.read(HEADER_BYTES, descriptor_end - HEADER_BYTES)?;
        let mut sections = Vec::with_capacity(widths.len());
        let mut next_offset = descriptor_end as u64;
        for (index, &width) in widths.iter().enumerate() {
            let entry = descriptors
                .get(index * DESCRIPTOR_BYTES..(index + 1) * DESCRIPTOR_BYTES)
                .ok_or(FastFieldError::Format)?;
            let tag = u16_at(entry, 0)?;
            let codec = u16_at(entry, 2)?;
            let actual_width = u32_at(entry, 4)?;
            let offset = u64_at(entry, 8)?;
            let length = u64_at(entry, 16)?;
            if tag != index as u16 + 1
                || codec != 0
                || actual_width != width
                || u64_at(entry, 24)? != 0
                || offset != next_offset
                || (width != 0 && count.checked_mul(u64::from(width)) != Some(length))
            {
                return Err(FastFieldError::Format);
            }
            next_offset = offset.checked_add(length).ok_or(FastFieldError::Format)?;
            if next_offset > file_length {
                return Err(FastFieldError::Format);
            }
            sections.push(Section { offset, length });
        }
        if next_offset != file_length {
            return Err(FastFieldError::Format);
        }
        Ok(Self {
            bytes,
            count,
            sections,
        })
    }

    fn row(&self, section: usize, id: u32, width: u64) -> Result<&[u8], FastFieldError> {
        if u64::from(id) >= self.count {
            return Err(FastFieldError::Range);
        }
        let section = self.sections.get(section).ok_or(FastFieldError::Format)?;
        let relative = u64::from(id)
            .checked_mul(width)
            .ok_or(FastFieldError::Range)?;
        let end = relative.checked_add(width).ok_or(FastFieldError::Range)?;
        if end > section.length {
            return Err(FastFieldError::Range);
        }
        let offset = section
            .offset
            .checked_add(relative)
            .ok_or(FastFieldError::Range)?;
        let offset = usize::try_from(offset).map_err(|_| FastFieldError::Range)?;
        self.bytes
            .read(offset, width as usize)
            .map_err(FastFieldError::Unit)
    }
}

pub(crate) struct FastFields<'g> {
    documents: Table<'g>,
    _videos: Table<'g>,
    video_count: u64,
}

impl<'g> FastFields<'g> {
    pub(crate) fn open(generation: &'g OpenedGeneration) -> Result<Self, FastFieldError> {
        let directory = &generation.directory;
        let file_length = |kind| {
            directory
                .files
                .iter()
                .find(|record| record.kind == kind)
                .map(|record| record.length)
                .ok_or(FastFieldError::Format)
        };
        let documents = Table::open(
            generation.verified_file(FileKind::Documents)?,
            file_length(FileKind::Documents)?,
            b"YTS9DOC\0",
            &DOCUMENT_WIDTHS,
            directory.document_count,
            directory.generation,
            directory.mutation_sequence,
        )?;
        let videos = Table::open(
            generation.verified_file(FileKind::Videos)?,
            file_length(FileKind::Videos)?,
            b"YTS9VID\0",
            &VIDEO_WIDTHS,
            directory.video_count,
            directory.generation,
            directory.mutation_sequence,
        )?;
        Ok(Self {
            documents,
            _videos: videos,
            video_count: directory.video_count,
        })
    }

    pub(crate) fn document_spine(&self, id: DocId) -> Result<(VideoId, u32), FastFieldError> {
        let row = self.documents.row(0, id.raw(), 8)?;
        let video = u32_at(row, 0)?;
        if u64::from(video) >= self.video_count {
            return Err(FastFieldError::Video);
        }
        Ok((VideoId::from_raw(video), u32_at(row, 4)?))
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, FastFieldError> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or(FastFieldError::Format)?
            .try_into()
            .map_err(|_| FastFieldError::Format)?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, FastFieldError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or(FastFieldError::Format)?
            .try_into()
            .map_err(|_| FastFieldError::Format)?,
    ))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, FastFieldError> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset + 8)
            .ok_or(FastFieldError::Format)?
            .try_into()
            .map_err(|_| FastFieldError::Format)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use memmap2::MmapOptions;
    use std::io::Write;

    fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
        bytes[offset..offset + value.len()].copy_from_slice(value);
    }

    fn table_file(magic: &[u8; 8], widths: &[u32], count: u64) -> Vec<u8> {
        let descriptor_end = HEADER_BYTES + widths.len() * DESCRIPTOR_BYTES;
        let mut bytes = vec![0_u8; descriptor_end];
        put(&mut bytes, 0, magic);
        put(&mut bytes, 8, &9_u16.to_le_bytes());
        put(&mut bytes, 10, &1_u16.to_le_bytes());
        put(&mut bytes, 12, &64_u16.to_le_bytes());
        put(&mut bytes, 14, &32_u16.to_le_bytes());
        put(&mut bytes, 16, &count.to_le_bytes());
        put(&mut bytes, 24, &(widths.len() as u32).to_le_bytes());
        put(&mut bytes, 32, &7_u64.to_le_bytes());
        put(&mut bytes, 40, &81_u64.to_le_bytes());
        for (index, width) in widths.iter().enumerate() {
            let offset = HEADER_BYTES + index * DESCRIPTOR_BYTES;
            put(&mut bytes, offset, &(index as u16 + 1).to_le_bytes());
            put(&mut bytes, offset + 4, &width.to_le_bytes());
            let section_offset = bytes.len() as u64;
            put(&mut bytes, offset + 8, &section_offset.to_le_bytes());
            let length = if *width == 0 {
                2
            } else {
                count * u64::from(*width)
            };
            put(&mut bytes, offset + 16, &length.to_le_bytes());
            bytes.resize(bytes.len() + length as usize, 0);
        }
        bytes
    }

    fn document_file() -> Vec<u8> {
        let mut bytes = table_file(b"YTS9DOC\0", &DOCUMENT_WIDTHS, 2);
        let spine = HEADER_BYTES + DOCUMENT_WIDTHS.len() * DESCRIPTOR_BYTES;
        put(&mut bytes, spine + 4, &123_u32.to_le_bytes());
        put(&mut bytes, spine + 12, &456_u32.to_le_bytes());
        bytes
    }

    fn verifier(bytes: &[u8]) -> VerifiedUnits<GenerationBytes> {
        let mut temporary = tempfile::tempfile().unwrap();
        temporary.write_all(bytes).unwrap();
        // SAFETY: the anonymous temporary file is not mutated after mapping.
        let map = unsafe { MmapOptions::new().map(&temporary).unwrap() };
        VerifiedUnits::new(
            GenerationBytes::Mapped(map),
            super::super::verified_units::MIN_UNIT_BYTES,
            vec![*blake3::hash(bytes).as_bytes()],
        )
        .unwrap()
    }

    #[test]
    fn bounded_document_sections_and_spine_decode() {
        let bytes = document_file();
        assert_eq!(&bytes[..8], b"YTS9DOC\0");
        assert_eq!(&bytes[64..66], &1_u16.to_le_bytes());
        assert_eq!(&bytes[72..80], &256_u64.to_le_bytes());
        let verifier = verifier(&bytes);
        let table = Table::open(
            &verifier,
            bytes.len() as u64,
            b"YTS9DOC\0",
            &DOCUMENT_WIDTHS,
            2,
            7,
            81,
        )
        .unwrap();
        assert_eq!(u32_at(table.row(0, 0, 8).unwrap(), 4).unwrap(), 123);
        assert_eq!(u32_at(table.row(0, 1, 8).unwrap(), 4).unwrap(), 456);
        assert_eq!(table.row(0, 2, 8), Err(FastFieldError::Range));
    }

    #[test]
    fn valid_checksum_does_not_authorize_overlapping_or_wrong_sections() {
        for (offset, value) in [
            (10, 2_u64),   // unsupported file version
            (72, 257_u64), // first section no longer starts after descriptors
            (80, 15_u64),  // two rows need 16 spine bytes
        ] {
            let mut bytes = document_file();
            let width = if offset == 10 { 2 } else { 8 };
            put(&mut bytes, offset, &value.to_le_bytes()[..width]);
            let verifier = verifier(&bytes);
            assert!(matches!(
                Table::open(
                    &verifier,
                    bytes.len() as u64,
                    b"YTS9DOC\0",
                    &DOCUMENT_WIDTHS,
                    2,
                    7,
                    81
                ),
                Err(FastFieldError::Format)
            ));
        }
    }

    #[test]
    fn bound_generation_serves_document_spine_and_rejects_bad_video_id() {
        use super::super::{
            builder::{DirectoryHeader, finalize_directory},
            footer::OpenedGeneration,
        };

        for bad_video in [false, true] {
            let temporary = tempfile::tempdir().unwrap();
            let mut documents = document_file();
            if bad_video {
                let second_video = HEADER_BYTES + DOCUMENT_WIDTHS.len() * DESCRIPTOR_BYTES + 8;
                put(&mut documents, second_video, &1_u32.to_le_bytes());
            }
            for kind in FileKind::ALL.into_iter().filter(|kind| kind.required()) {
                let contents = match kind {
                    FileKind::Documents => documents.clone(),
                    FileKind::Videos => table_file(b"YTS9VID\0", &VIDEO_WIDTHS, 1),
                    _ => vec![kind as u8],
                };
                std::fs::write(temporary.path().join(kind.file_name()), contents).unwrap();
            }
            let binding = finalize_directory(
                temporary.path(),
                DirectoryHeader {
                    normalization_version: 2,
                    rank_version: 2,
                    generation: 7,
                    mutation_sequence: 81,
                    corpus_digest: [9; 32],
                    document_count: 2,
                    video_count: 1,
                    term_count: 1,
                    bitmap_budget_bytes: 8 * 1024 * 1024,
                },
                2 * 1024 * 1024,
            )
            .unwrap();
            let generation = OpenedGeneration::open_bound(temporary.path(), &binding).unwrap();
            let fields = FastFields::open(&generation).unwrap();
            assert_eq!(fields.document_spine(DocId::from_raw(0)).unwrap().1, 123);
            if bad_video {
                assert_eq!(
                    fields.document_spine(DocId::from_raw(1)),
                    Err(FastFieldError::Video)
                );
            } else {
                assert_eq!(
                    fields.document_spine(DocId::from_raw(1)),
                    Ok((VideoId::from_raw(0), 456))
                );
            }
        }
    }
}
