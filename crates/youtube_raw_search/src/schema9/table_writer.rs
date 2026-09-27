//! Shared version-1 schema-9 fast-field table encoder.

use super::fast_fields::{DESCRIPTOR_BYTES, HEADER_BYTES};

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum TableWriteError {
    #[error("fast-field section width, offset, or allocation exceeds format limits")]
    Size,
}

pub(crate) fn encode_table<const N: usize>(
    magic: &[u8; 8],
    widths: &[u32; N],
    sections: [Vec<u8>; N],
    row_count: u32,
    generation: u64,
    mutation_sequence: u64,
) -> Result<Vec<u8>, TableWriteError> {
    let descriptor_end = HEADER_BYTES
        .checked_add(
            N.checked_mul(DESCRIPTOR_BYTES)
                .ok_or(TableWriteError::Size)?,
        )
        .ok_or(TableWriteError::Size)?;
    let mut output = vec![0_u8; descriptor_end];
    output[..8].copy_from_slice(magic);
    output[8..10].copy_from_slice(&9_u16.to_le_bytes());
    output[10..12].copy_from_slice(&1_u16.to_le_bytes());
    output[12..14].copy_from_slice(&(HEADER_BYTES as u16).to_le_bytes());
    output[14..16].copy_from_slice(&(DESCRIPTOR_BYTES as u16).to_le_bytes());
    output[16..24].copy_from_slice(&u64::from(row_count).to_le_bytes());
    output[24..28].copy_from_slice(
        &u32::try_from(N)
            .map_err(|_| TableWriteError::Size)?
            .to_le_bytes(),
    );
    output[32..40].copy_from_slice(&generation.to_le_bytes());
    output[40..48].copy_from_slice(&mutation_sequence.to_le_bytes());
    for (index, section) in sections.iter().enumerate() {
        let descriptor = HEADER_BYTES + index * DESCRIPTOR_BYTES;
        let offset = u64::try_from(output.len()).map_err(|_| TableWriteError::Size)?;
        let length = u64::try_from(section.len()).map_err(|_| TableWriteError::Size)?;
        if widths[index] != 0 && length != u64::from(row_count) * u64::from(widths[index]) {
            return Err(TableWriteError::Size);
        }
        let _ = offset.checked_add(length).ok_or(TableWriteError::Size)?;
        output[descriptor..descriptor + 2].copy_from_slice(
            &u16::try_from(index + 1)
                .map_err(|_| TableWriteError::Size)?
                .to_le_bytes(),
        );
        output[descriptor + 4..descriptor + 8].copy_from_slice(&widths[index].to_le_bytes());
        output[descriptor + 8..descriptor + 16].copy_from_slice(&offset.to_le_bytes());
        output[descriptor + 16..descriptor + 24].copy_from_slice(&length.to_le_bytes());
        output
            .try_reserve(section.len())
            .map_err(|_| TableWriteError::Size)?;
        output.extend_from_slice(section);
    }
    Ok(output)
}
