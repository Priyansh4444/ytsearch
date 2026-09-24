//! Versioned `videos.bin` encoder from audited canonical source metadata.

use std::collections::BTreeMap;

use super::{
    fast_fields::utc_year,
    source_spool::{AuditedSource, VideoSource},
};

const HEADER_BYTES: usize = 64;
const DESCRIPTOR_BYTES: usize = 32;
const WIDTHS: [u32; 14] = [4, 1, 4, 4, 8, 8, 8, 8, 1, 8, 4, 0, 4, 4];

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum VideoWriteError {
    #[error("video count, document count, or file size exceeds schema-9 limits")]
    Size,
    #[error("unknown creator for video {0}")]
    Creator(String),
    #[error("video IDs or document ranges are not strictly ordered")]
    Order,
    #[error("video metadata cannot be represented in schema-9 fast fields")]
    Value,
}

/// Build the small per-video fast-field table. This does not finalize a
/// generation: the document table, posting/facet files, and complete audit are
/// separate requirements.
pub(crate) fn encode_videos(
    source: &AuditedSource,
    creators: &BTreeMap<String, u32>,
    document_count: u32,
    generation: u64,
    mutation_sequence: u64,
) -> Result<Vec<u8>, VideoWriteError> {
    encode_video_rows(
        source.videos(),
        creators,
        document_count,
        generation,
        mutation_sequence,
    )
}

fn encode_video_rows(
    videos: &[VideoSource],
    creators: &BTreeMap<String, u32>,
    document_count: u32,
    generation: u64,
    mutation_sequence: u64,
) -> Result<Vec<u8>, VideoWriteError> {
    let row_count = u32::try_from(videos.len()).map_err(|_| VideoWriteError::Size)?;
    let mut sections: [Vec<u8>; 14] = std::array::from_fn(|_| Vec::new());
    let mut next_document = 0_u32;
    let mut previous_video: Option<&str> = None;
    for video in videos {
        let id = video.external_video_id.as_str();
        if id.is_empty() {
            return Err(VideoWriteError::Value);
        }
        if previous_video.is_some_and(|previous| previous.as_bytes() >= id.as_bytes())
            || video.documents.start != next_document
            || video.documents.start >= video.documents.end
            || video.documents.end > document_count
        {
            return Err(VideoWriteError::Order);
        }
        previous_video = Some(id);
        next_document = video.documents.end;
        let creator = creators
            .get(&video.creator_id)
            .ok_or_else(|| VideoWriteError::Creator(id.to_owned()))?;
        sections[0].extend_from_slice(&creator.to_le_bytes());
        let kind = match video.duration_ms {
            None => 0_u8,
            Some(0..=60_000) => 1,
            Some(_) => 2,
        };
        sections[1].push(kind);
        let year = video
            .published_at_s
            .map(|seconds| {
                utc_year(seconds)
                    .filter(|year| *year != 0)
                    .ok_or(VideoWriteError::Value)
            })
            .transpose()?;
        sections[2].extend_from_slice(&year.unwrap_or(0).to_le_bytes());
        sections[3].extend_from_slice(&video.duration_ms.unwrap_or(0).to_le_bytes());
        sections[4].extend_from_slice(&video.published_at_s.unwrap_or(0).to_le_bytes());
        for (section, value) in [(5, video.views), (6, video.likes), (7, video.comments)] {
            sections[section].extend_from_slice(&value.unwrap_or(0).to_le_bytes());
        }
        let mut mask = 0_u8;
        for (bit, absent) in [
            video.duration_ms.is_none(),
            year.is_none(),
            video.duration_ms.is_none(),
            video.published_at_s.is_none(),
            video.views.is_none(),
            video.likes.is_none(),
            video.comments.is_none(),
        ]
        .into_iter()
        .enumerate()
        {
            if absent {
                mask |= 1 << bit;
            }
        }
        sections[8].push(mask);
        let blob_offset = u64::try_from(sections[11].len()).map_err(|_| VideoWriteError::Size)?;
        let id_length = u32::try_from(id.len()).map_err(|_| VideoWriteError::Size)?;
        sections[9].extend_from_slice(&blob_offset.to_le_bytes());
        sections[10].extend_from_slice(&id_length.to_le_bytes());
        sections[11].extend_from_slice(id.as_bytes());
        sections[12].extend_from_slice(&video.documents.start.to_le_bytes());
        sections[13]
            .extend_from_slice(&(video.documents.end - video.documents.start).to_le_bytes());
    }
    if next_document != document_count {
        return Err(VideoWriteError::Order);
    }
    let descriptor_end = HEADER_BYTES + WIDTHS.len() * DESCRIPTOR_BYTES;
    let mut output = vec![0_u8; descriptor_end];
    output[..8].copy_from_slice(b"YTS9VID\0");
    output[8..10].copy_from_slice(&9_u16.to_le_bytes());
    output[10..12].copy_from_slice(&1_u16.to_le_bytes());
    output[12..14].copy_from_slice(&(HEADER_BYTES as u16).to_le_bytes());
    output[14..16].copy_from_slice(&(DESCRIPTOR_BYTES as u16).to_le_bytes());
    output[16..24].copy_from_slice(&u64::from(row_count).to_le_bytes());
    output[24..28].copy_from_slice(&(WIDTHS.len() as u32).to_le_bytes());
    output[32..40].copy_from_slice(&generation.to_le_bytes());
    output[40..48].copy_from_slice(&mutation_sequence.to_le_bytes());
    for (index, section) in sections.iter().enumerate() {
        let descriptor = HEADER_BYTES + index * DESCRIPTOR_BYTES;
        let offset = u64::try_from(output.len()).map_err(|_| VideoWriteError::Size)?;
        let length = u64::try_from(section.len()).map_err(|_| VideoWriteError::Size)?;
        if WIDTHS[index] != 0 && length != u64::from(row_count) * u64::from(WIDTHS[index]) {
            return Err(VideoWriteError::Size);
        }
        let _ = offset.checked_add(length).ok_or(VideoWriteError::Size)?;
        output[descriptor..descriptor + 2].copy_from_slice(&((index + 1) as u16).to_le_bytes());
        output[descriptor + 4..descriptor + 8].copy_from_slice(&WIDTHS[index].to_le_bytes());
        output[descriptor + 8..descriptor + 16].copy_from_slice(&offset.to_le_bytes());
        output[descriptor + 16..descriptor + 24].copy_from_slice(&length.to_le_bytes());
        output.extend_from_slice(section);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str, documents: std::ops::Range<u32>) -> VideoSource {
        VideoSource {
            external_video_id: id.into(),
            creator_id: "creator".into(),
            title: "title".into(),
            duration_ms: None,
            published_at_s: None,
            views: None,
            likes: None,
            comments: None,
            documents,
        }
    }

    #[test]
    fn preserves_missing_versus_known_zero_and_contiguous_ranges() {
        let creators = BTreeMap::from([("creator".into(), 7)]);
        let mut second = video("b", 1..3);
        second.duration_ms = Some(0);
        second.published_at_s = Some(0);
        second.views = Some(0);
        let bytes = encode_video_rows(&[video("a", 0..1), second], &creators, 3, 9, 10).unwrap();
        assert_eq!(&bytes[..8], b"YTS9VID\0");
        assert_eq!(u64::from_le_bytes(bytes[16..24].try_into().unwrap()), 2);
        let mask_offset =
            u64::from_le_bytes(bytes[64 + 8 * 32 + 8..64 + 8 * 32 + 16].try_into().unwrap())
                as usize;
        assert_eq!(bytes[mask_offset], 0x7f);
        assert_eq!(bytes[mask_offset + 1], 0b0110_0000);
    }

    #[test]
    fn rejects_missing_creator_and_broken_ranges() {
        let creators = BTreeMap::new();
        assert!(matches!(
            encode_video_rows(&[video("a", 0..1)], &creators, 1, 9, 10),
            Err(VideoWriteError::Creator(_))
        ));
        let creators = BTreeMap::from([("creator".into(), 0)]);
        assert_eq!(
            encode_video_rows(&[video("a", 1..2)], &creators, 2, 9, 10),
            Err(VideoWriteError::Order)
        );
    }
}
