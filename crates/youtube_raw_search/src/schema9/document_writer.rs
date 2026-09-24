//! Versioned `documents.bin` encoder from globally audited source IDs.

use super::{
    fast_fields::DOCUMENT_WIDTHS,
    source_spool::{AuditedSource, SpoolError},
    table_writer::encode_table,
};

/// Emit the document spine, indexed text token count, title-owner flag, and
/// exact source IDs. Payload and posting emission must consume the same dense
/// stream before the candidate can be published.
pub(crate) fn encode_documents(
    source: &mut AuditedSource,
    generation: u64,
    mutation_sequence: u64,
) -> Result<Vec<u8>, SpoolError> {
    let groups: Vec<_> = source
        .videos()
        .iter()
        .enumerate()
        .map(|(index, video)| {
            Ok((
                video.external_video_id.clone(),
                u32::try_from(index).map_err(|_| SpoolError::Size)?,
                video.documents.clone(),
            ))
        })
        .collect::<Result<_, SpoolError>>()?;
    let mut sections: [Vec<u8>; 6] = std::array::from_fn(|_| Vec::new());
    let mut group_index = 0_usize;
    let document_count = source.visit_sorted(|dense_id, document| {
        while groups
            .get(group_index)
            .is_some_and(|(_, _, range)| dense_id >= range.end)
        {
            group_index += 1;
        }
        let (video_id, dense_video_id, range) =
            groups.get(group_index).ok_or(SpoolError::Mismatch)?;
        if document.external_video_id != *video_id || !range.contains(&dense_id) {
            return Err(SpoolError::Mismatch);
        }
        sections[0].extend_from_slice(&dense_video_id.to_le_bytes());
        sections[0].extend_from_slice(&document.t_start_ms.to_le_bytes());
        let token_count = count_tokens(&document.text)?;
        sections[1].extend_from_slice(&token_count.to_le_bytes());
        sections[2].push(if dense_id == range.start { 0b11 } else { 0b01 });
        let offset = u64::try_from(sections[5].len()).map_err(|_| SpoolError::Size)?;
        let length = u32::try_from(document.external_id.len()).map_err(|_| SpoolError::Size)?;
        sections[3].extend_from_slice(&offset.to_le_bytes());
        sections[4].extend_from_slice(&length.to_le_bytes());
        sections[5].extend_from_slice(document.external_id.as_bytes());
        Ok(())
    })?;
    let expected = groups
        .last()
        .map_or(0, |(_, _, range)| u64::from(range.end));
    if document_count != expected {
        return Err(SpoolError::Mismatch);
    }
    let count = u32::try_from(document_count).map_err(|_| SpoolError::Size)?;
    encode_table(
        b"YTS9DOC\0",
        &DOCUMENT_WIDTHS,
        sections,
        count,
        generation,
        mutation_sequence,
    )
    .map_err(|_| SpoolError::Size)
}

fn count_tokens(value: &str) -> Result<u32, SpoolError> {
    let mut count = 0_u32;
    let mut inside = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if !inside {
                count = count.checked_add(1).ok_or(SpoolError::Size)?;
                inside = true;
            }
        } else {
            inside = false;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::super::source_spool::SourceSpool;
    use super::*;

    #[test]
    fn sorted_source_assigns_video_spine_flags_and_external_id_blob() {
        let directory = tempfile::tempdir().unwrap();
        let mut spool = SourceSpool::create(directory.path()).unwrap();
        for row in [
            serde_json::json!({"id":"b","video_id":"v","youtuber_id":"c","title":"title","text":"two words","t_start_ms":20,"t_end_ms":30}),
            serde_json::json!({"id":"a","video_id":"v","youtuber_id":"c","title":"title","text":"one","t_start_ms":0,"t_end_ms":10}),
        ] {
            spool.append_line(&row.to_string()).unwrap();
        }
        let mut audited = spool.finish().unwrap().audit_videos().unwrap();
        let bytes = encode_documents(&mut audited, 7, 81).unwrap();
        assert_eq!(&bytes[..8], b"YTS9DOC\0");
        assert_eq!(u64::from_le_bytes(bytes[16..24].try_into().unwrap()), 2);
        let offset = |section: usize| {
            u64::from_le_bytes(
                bytes[64 + section * 32 + 8..64 + section * 32 + 16]
                    .try_into()
                    .unwrap(),
            ) as usize
        };
        assert_eq!(&bytes[offset(5)..offset(5) + 2], b"ab");
        assert_eq!(&bytes[offset(2)..offset(2) + 2], &[0b11, 0b01]);
        assert_eq!(
            u32::from_le_bytes(bytes[offset(1)..offset(1) + 4].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_le_bytes(bytes[offset(1) + 4..offset(1) + 8].try_into().unwrap()),
            2
        );
    }
}
