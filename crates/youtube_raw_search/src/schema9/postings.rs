//! Provisional delta-varint posting blocks and structural skip directories.
//!
//! Positions are stored separately. The production file descriptor and codec
//! choice remain P01/P03 gates; this module proves the cursor contract.

use super::sets::{DocId, SetId, Universe};

const BLOCK_SIZE: usize = 8;
const LEVELS: [usize; 4] = [4096, 512, 64, 8];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Posting {
    pub(crate) doc: DocId,
    pub(crate) frequency: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Checkpoint {
    first_doc: u32,
    last_doc: u32,
    ordinal: u64,
    byte_offset: u64,
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PostingError {
    #[error("postings are not strictly increasing at ordinal {0}")]
    Unsorted(u64),
    #[error("posting {doc} is outside universe of length {len}")]
    OutOfRange { doc: u32, len: u64 },
    #[error("posting frequency is zero at ordinal {0}")]
    ZeroFrequency(u64),
    #[error("posting stream is truncated or malformed")]
    Malformed,
}

pub(crate) struct PostingList {
    universe: Universe<DocId>,
    bytes: Vec<u8>,
    // Descending spans. Every entry bounds all IDs in its covered group.
    directories: [Vec<Checkpoint>; LEVELS.len()],
    count: u64,
}

impl PostingList {
    pub(crate) fn encode(
        universe: Universe<DocId>,
        postings: &[(u32, u32)],
    ) -> Result<Self, PostingError> {
        let mut bytes = Vec::new();
        let mut blocks = Vec::new();
        let mut previous = None;
        for (ordinal, block) in postings.chunks(BLOCK_SIZE).enumerate() {
            let first_doc = block[0].0;
            let ordinal = (ordinal * BLOCK_SIZE) as u64;
            let byte_offset = bytes.len() as u64;
            bytes.push(block.len() as u8);
            bytes.extend_from_slice(&first_doc.to_le_bytes());
            for (position, &(doc, frequency)) in block.iter().enumerate() {
                let item_ordinal = ordinal + position as u64;
                if u64::from(doc) >= universe.len() {
                    return Err(PostingError::OutOfRange {
                        doc,
                        len: universe.len(),
                    });
                }
                if previous.is_some_and(|prior| prior >= doc) {
                    return Err(PostingError::Unsorted(item_ordinal));
                }
                if frequency == 0 {
                    return Err(PostingError::ZeroFrequency(item_ordinal));
                }
                if position > 0 {
                    write_varint(
                        doc - previous.expect("position follows first posting"),
                        &mut bytes,
                    );
                }
                write_varint(frequency, &mut bytes);
                previous = Some(doc);
            }
            blocks.push(Checkpoint {
                first_doc,
                last_doc: previous.expect("block is nonempty"),
                ordinal,
                byte_offset,
            });
        }
        let directories = std::array::from_fn(|level| {
            let blocks_per_group = LEVELS[level] / BLOCK_SIZE;
            blocks
                .chunks(blocks_per_group)
                .map(|group| Checkpoint {
                    first_doc: group[0].first_doc,
                    last_doc: group.last().expect("group is nonempty").last_doc,
                    ordinal: group[0].ordinal,
                    byte_offset: group[0].byte_offset,
                })
                .collect()
        });
        Ok(Self {
            universe,
            bytes,
            directories,
            count: postings.len() as u64,
        })
    }

    pub(crate) fn len(&self) -> u64 {
        self.count
    }

    pub(crate) fn encoded_bytes(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn cursor(&self) -> PostingCursor<'_> {
        PostingCursor {
            list: self,
            block_index: None,
            decoded: [None; BLOCK_SIZE],
            position: 0,
            exhausted: false,
            faulted: false,
            blocks_decoded: 0,
            postings_decoded: 0,
        }
    }

    /// Finds the first block whose final ID is at or beyond the target.
    fn block_for(&self, target: u32) -> Option<usize> {
        if self.count == 0 {
            return None;
        }
        let mut start = 0_u64;
        let mut end = self.count;
        for (level, directory) in LEVELS.iter().zip(&self.directories) {
            let first = directory.partition_point(|item| item.ordinal < start);
            let past = directory.partition_point(|item| item.ordinal < end);
            let group = &directory[first..past];
            let relative = group.partition_point(|item| item.last_doc < target);
            let selected = group.get(relative)?;
            start = selected.ordinal;
            end = (start + *level as u64).min(end);
        }
        Some((start / BLOCK_SIZE as u64) as usize)
    }

    fn decode_block(
        &self,
        block_index: usize,
    ) -> Result<[Option<Posting>; BLOCK_SIZE], PostingError> {
        let blocks = &self.directories[3];
        let checkpoint = blocks.get(block_index).ok_or(PostingError::Malformed)?;
        let start = usize::try_from(checkpoint.byte_offset).map_err(|_| PostingError::Malformed)?;
        let end = blocks
            .get(block_index + 1)
            .map(|next| usize::try_from(next.byte_offset).map_err(|_| PostingError::Malformed))
            .transpose()?
            .unwrap_or(self.bytes.len());
        let bytes = self.bytes.get(start..end).ok_or(PostingError::Malformed)?;
        if bytes.len() < 5 || bytes[0] == 0 || bytes[0] as usize > BLOCK_SIZE {
            return Err(PostingError::Malformed);
        }
        let count = bytes[0] as usize;
        let expected_next_ordinal = blocks
            .get(block_index + 1)
            .map_or(self.count, |next| next.ordinal);
        if checkpoint.ordinal + count as u64 != expected_next_ordinal {
            return Err(PostingError::Malformed);
        }
        let mut doc = u32::from_le_bytes(
            bytes[1..5]
                .try_into()
                .map_err(|_| PostingError::Malformed)?,
        );
        if doc != checkpoint.first_doc {
            return Err(PostingError::Malformed);
        }
        let mut cursor = 5;
        let mut output = [None; BLOCK_SIZE];
        for slot in output.iter_mut().take(count) {
            if cursor != 5 {
                let delta = read_varint(bytes, &mut cursor)?;
                if delta == 0 {
                    return Err(PostingError::Malformed);
                }
                doc = doc.checked_add(delta).ok_or(PostingError::Malformed)?;
            }
            let frequency = read_varint(bytes, &mut cursor)?;
            if frequency == 0 || u64::from(doc) >= self.universe.len() {
                return Err(PostingError::Malformed);
            }
            *slot = Some(Posting {
                doc: DocId::from_raw(doc),
                frequency,
            });
        }
        if cursor != bytes.len() || doc != checkpoint.last_doc {
            return Err(PostingError::Malformed);
        }
        Ok(output)
    }
}

fn write_varint(mut value: u32, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn read_varint(bytes: &[u8], offset: &mut usize) -> Result<u32, PostingError> {
    let mut value = 0_u32;
    for shift in (0..=28).step_by(7) {
        let byte = *bytes.get(*offset).ok_or(PostingError::Malformed)?;
        *offset += 1;
        if shift == 28 && byte > 0x0f {
            return Err(PostingError::Malformed);
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            if shift > 0 && byte == 0 {
                return Err(PostingError::Malformed);
            }
            return Ok(value);
        }
    }
    Err(PostingError::Malformed)
}

pub(crate) struct PostingCursor<'a> {
    list: &'a PostingList,
    block_index: Option<usize>,
    decoded: [Option<Posting>; BLOCK_SIZE],
    position: usize,
    exhausted: bool,
    faulted: bool,
    blocks_decoded: u64,
    postings_decoded: u64,
}

impl PostingCursor<'_> {
    fn load(&mut self, index: usize) -> Result<(), PostingError> {
        let decoded = match self.list.decode_block(index) {
            Ok(decoded) => decoded,
            Err(error) => {
                // A corrupt block ends this cursor. In particular, next() may
                // have consumed the last slot of the previous block already.
                self.faulted = true;
                self.block_index = None;
                self.decoded = [None; BLOCK_SIZE];
                self.position = 0;
                return Err(error);
            }
        };
        self.decoded = decoded;
        self.block_index = Some(index);
        self.position = 0;
        self.blocks_decoded += 1;
        self.postings_decoded += self.decoded.iter().filter(|item| item.is_some()).count() as u64;
        Ok(())
    }

    pub(crate) fn current(&mut self) -> Result<Option<Posting>, PostingError> {
        if self.faulted {
            return Err(PostingError::Malformed);
        }
        if self.exhausted {
            return Ok(None);
        }
        if self.block_index.is_none() {
            if self.list.count == 0 {
                self.exhausted = true;
                return Ok(None);
            }
            self.load(0)?;
        }
        Ok(self.decoded[self.position])
    }

    pub(crate) fn advance(&mut self, target: DocId) -> Result<Option<Posting>, PostingError> {
        if self.faulted {
            return Err(PostingError::Malformed);
        }
        if self.exhausted {
            return Ok(None);
        }
        if self.block_index.is_some()
            && let Some(current) = self.decoded[self.position]
            && current.doc >= target
        {
            return Ok(Some(current));
        }
        let Some(block) = self.list.block_for(target.raw()) else {
            self.exhausted = true;
            return Ok(None);
        };
        if self.block_index != Some(block) {
            self.load(block)?;
        }
        self.position = self
            .decoded
            .partition_point(|item| item.is_some_and(|posting| posting.doc < target));
        Ok(self.decoded[self.position])
    }

    pub(crate) fn next(&mut self) -> Result<Option<Posting>, PostingError> {
        let Some(posting) = self.current()? else {
            return Ok(None);
        };
        self.position += 1;
        if self.position == BLOCK_SIZE || self.decoded[self.position].is_none() {
            let next_block = self.block_index.expect("current posting has a block") + 1;
            if next_block == self.list.directories[3].len() {
                self.exhausted = true;
            } else {
                self.load(next_block)?;
            }
        }
        Ok(Some(posting))
    }

    pub(crate) fn decoded_counts(&self) -> (u64, u64) {
        (self.blocks_decoded, self.postings_decoded)
    }
}

#[cfg(test)]
#[path = "tests/postings.rs"]
mod tests;
