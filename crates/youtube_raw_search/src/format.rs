use crate::{Error, Result};
use memmap2::{Mmap, MmapOptions};
use std::{fs::File, path::Path};

pub const LEXICON_MAGIC: &[u8; 8] = b"RAWLEX01";
pub const POSTINGS_MAGIC: &[u8; 8] = b"RAWPOS01";
pub const POSITIONS_MAGIC: &[u8; 8] = b"RAWLOC01";
pub const FULL_POSITIONS_MAGIC: &[u8; 8] = b"RAWFUL01";
pub const POSITION_SPOOL_MAGIC: &[u8; 8] = b"RAWSPL01";
pub const DOCUMENTS_MAGIC: &[u8; 8] = b"RAWDOC01";
pub const METADATA_MAGIC: &[u8; 8] = b"RAWMET02";
pub const FUZZY_MAGIC: &[u8; 8] = b"RAWFUZ01";
pub const HEADER_BYTES: usize = 16;
pub const DOCUMENT_RECORD_BYTES: usize = 24;
pub const BLOCK_RECORD_BYTES: usize = 16;

const PREFETCH_PAGE_BYTES: usize = 4096;

pub struct MappedFile {
    bytes: Mmap,
}

impl MappedFile {
    pub fn open(path: &Path, magic: &[u8; 8]) -> Result<Self> {
        let file = File::open(path)?;
        if file.metadata()?.len() < HEADER_BYTES as u64 {
            return Err(Error::Corrupt(format!(
                "{} is shorter than its header",
                path.display()
            )));
        }
        // SAFETY: experiment indexes are immutable after an atomic build and the server exposes
        // no write API. The mapping is read-only and every typed read below performs bounds checks.
        let bytes = unsafe { MmapOptions::new().map(&file)? };
        if bytes.get(..8) != Some(magic.as_slice()) {
            return Err(Error::Corrupt(format!(
                "{} has the wrong magic",
                path.display()
            )));
        }
        Ok(Self { bytes })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Reads the mapping into the page cache ahead of first use by touching
    /// one byte per page; sequential faults use the kernel's mmap readahead.
    /// `MADV_WILLNEED` is not enough: on the production kernel it returned at
    /// once and left a newly published generation cold.
    pub fn prefetch(&self) {
        for page in self.bytes.chunks(PREFETCH_PAGE_BYTES) {
            std::hint::black_box(page[0]);
        }
    }

    /// Marks the mapping as randomly accessed so a fault reads only the
    /// touched pages instead of the kernel's read-around window.
    pub fn advise_random(&self) {
        let _ = self.bytes.advise(memmap2::Advice::Random);
    }
}

pub fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| Error::Corrupt("u16 offset overflows".to_string()))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| Error::Corrupt("u16 extends beyond file".to_string()))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

pub fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| Error::Corrupt("u32 offset overflows".to_string()))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| Error::Corrupt("u32 extends beyond file".to_string()))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

pub fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| Error::Corrupt("u64 offset overflows".to_string()))?;
    let value = bytes
        .get(offset..end)
        .ok_or_else(|| Error::Corrupt("u64 extends beyond file".to_string()))?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

pub fn write_header(output: &mut Vec<u8>, magic: &[u8; 8], count: u32) {
    output.extend_from_slice(magic);
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&1_u32.to_le_bytes());
}

pub fn write_varint(mut value: u32, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

#[inline(always)]
pub fn read_varint(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u32> {
    let available = bytes
        .get(*cursor..end)
        .ok_or_else(|| Error::Corrupt("postings range extends beyond file".to_string()))?;
    let first = *available
        .first()
        .ok_or_else(|| Error::Corrupt("invalid postings varint".to_string()))?;
    if first < 0x80 {
        *cursor += 1;
        return Ok(u32::from(first));
    }

    let second = *available
        .get(1)
        .ok_or_else(|| Error::Corrupt("invalid postings varint".to_string()))?;
    let mut value = u32::from(first & 0x7f) | (u32::from(second & 0x7f) << 7);
    if second < 0x80 {
        *cursor += 2;
        return Ok(value);
    }

    let third = *available
        .get(2)
        .ok_or_else(|| Error::Corrupt("invalid postings varint".to_string()))?;
    value |= u32::from(third & 0x7f) << 14;
    if third < 0x80 {
        *cursor += 3;
        return Ok(value);
    }

    let fourth = *available
        .get(3)
        .ok_or_else(|| Error::Corrupt("invalid postings varint".to_string()))?;
    value |= u32::from(fourth & 0x7f) << 21;
    if fourth < 0x80 {
        *cursor += 4;
        return Ok(value);
    }

    let fifth = *available
        .get(4)
        .ok_or_else(|| Error::Corrupt("invalid postings varint".to_string()))?;
    if fifth > 0x0f {
        return Err(Error::Corrupt("postings varint overflows u32".to_string()));
    }
    *cursor += 5;
    Ok(value | (u32::from(fifth) << 28))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip_boundaries() {
        for expected in [0, 1, 127, 128, 16_384, u32::MAX] {
            let mut encoded = Vec::new();
            write_varint(expected, &mut encoded);
            let mut cursor = 0;
            assert_eq!(
                read_varint(&encoded, &mut cursor, encoded.len()).expect("decode"),
                expected
            );
            assert_eq!(cursor, encoded.len());
        }
    }

    #[test]
    fn malformed_varints_and_overflowing_offsets_are_rejected() {
        for encoded in [
            vec![0x80],
            vec![0x80, 0x80, 0x80, 0x80, 0x10],
            vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x00],
        ] {
            let mut cursor = 0;
            assert!(read_varint(&encoded, &mut cursor, encoded.len()).is_err());
        }
        assert!(read_u16(&[0; 8], usize::MAX).is_err());
        assert!(read_u32(&[0; 8], usize::MAX).is_err());
        assert!(read_u64(&[0; 8], usize::MAX).is_err());
    }
}
