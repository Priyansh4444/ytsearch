use crate::{
    Error, Result,
    format::{read_varint, write_varint},
};

/// Appends strictly increasing positions after the first position as deltas.
///
/// The first position remains in the existing hot sidecar. The cold stream
/// stores no per-posting count because the posting frequency already provides
/// it, avoiding a second integer and a second offset table per posting.
pub(crate) fn write_remaining_positions(positions: &[u32], output: &mut Vec<u8>) -> Result<()> {
    let Some((&first, remaining)) = positions.split_first() else {
        return Err(Error::InvalidInput(
            "a position list must not be empty".to_string(),
        ));
    };
    let mut previous = first;
    for &position in remaining {
        let delta = position
            .checked_sub(previous)
            .filter(|delta| *delta > 0)
            .ok_or_else(|| {
                Error::InvalidInput("positions must be strictly increasing".to_string())
            })?;
        write_varint(delta, output);
        previous = position;
    }
    Ok(())
}

/// Sequential decoder for one term/field cold-position range.
///
/// A cursor is intentionally tiny and borrows the memory-mapped bytes. Phrase
/// queries can reuse one caller-owned scratch vector, while ordinary queries
/// never construct a cursor or touch the cold mapping.
pub(crate) struct PositionCursor<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> PositionCursor<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    pub(crate) fn read_into(
        &mut self,
        first_position: u32,
        frequency: u32,
        output: &mut Vec<u32>,
    ) -> Result<()> {
        if frequency == 0 {
            return Err(Error::Corrupt(
                "a positional posting has zero frequency".to_string(),
            ));
        }
        let required = usize::try_from(frequency)
            .map_err(|_| Error::Corrupt("position frequency exceeds usize".to_string()))?;
        output.clear();
        output
            .try_reserve(required)
            .map_err(|_| Error::Corrupt("position scratch capacity overflows".to_string()))?;
        output.push(first_position);

        let mut previous = first_position;
        for _ in 1..frequency {
            let delta = read_varint(self.bytes, &mut self.cursor, self.bytes.len())?;
            if delta == 0 {
                return Err(Error::Corrupt(
                    "position deltas must be positive".to_string(),
                ));
            }
            previous = previous
                .checked_add(delta)
                .ok_or_else(|| Error::Corrupt("position delta overflows u32".to_string()))?;
            output.push(previous);
        }
        Ok(())
    }

    pub(crate) fn skip(&mut self, frequency: u32) -> Result<()> {
        if frequency == 0 {
            return Err(Error::Corrupt(
                "a positional posting has zero frequency".to_string(),
            ));
        }
        for _ in 1..frequency {
            let delta = read_varint(self.bytes, &mut self.cursor, self.bytes.len())?;
            if delta == 0 {
                return Err(Error::Corrupt(
                    "position deltas must be positive".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn offset(&self) -> usize {
        self.cursor
    }

    /// Resumes at a posting boundary previously reported by `offset`.
    pub(crate) fn seek(&mut self, offset: usize) {
        self.cursor = offset;
    }

    pub(crate) fn finish(&self) -> Result<()> {
        if self.cursor != self.bytes.len() {
            return Err(Error::Corrupt(
                "cold position range has trailing bytes".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn position_deltas_round_trip_into_reused_scratch() {
        let expected = [2, 3, 4, 120, 121];
        let mut encoded = Vec::new();
        write_remaining_positions(&expected, &mut encoded).unwrap();
        assert_eq!(encoded, [1, 1, 116, 1]);

        let mut scratch = Vec::with_capacity(8);
        let allocation = scratch.as_ptr();
        let mut cursor = PositionCursor::new(&encoded);
        cursor
            .read_into(expected[0], expected.len() as u32, &mut scratch)
            .unwrap();
        cursor.finish().unwrap();

        assert_eq!(scratch, expected);
        assert_eq!(
            scratch.as_ptr(),
            allocation,
            "scratch allocation was not reused"
        );
    }

    #[test]
    fn sequential_postings_share_one_stream_without_offset_entries() {
        let first = [1, 4, 9];
        let second = [20, 21];
        let mut encoded = Vec::new();
        write_remaining_positions(&first, &mut encoded).unwrap();
        write_remaining_positions(&second, &mut encoded).unwrap();

        let mut cursor = PositionCursor::new(&encoded);
        let mut scratch = Vec::new();
        cursor.read_into(first[0], 3, &mut scratch).unwrap();
        assert_eq!(scratch, first);
        cursor.read_into(second[0], 2, &mut scratch).unwrap();
        assert_eq!(scratch, second);
        cursor.finish().unwrap();
    }

    #[test]
    fn malformed_or_inconsistent_positions_fail_closed() {
        for positions in [&[][..], &[1, 1], &[2, 1]] {
            assert!(write_remaining_positions(positions, &mut Vec::new()).is_err());
        }

        let mut scratch = Vec::new();
        let mut truncated = PositionCursor::new(&[0x80]);
        assert!(truncated.read_into(1, 2, &mut scratch).is_err());

        let mut zero_delta = PositionCursor::new(&[0]);
        assert!(zero_delta.read_into(1, 2, &mut scratch).is_err());

        let mut overflow = PositionCursor::new(&[1]);
        assert!(overflow.read_into(u32::MAX, 2, &mut scratch).is_err());

        let trailing = PositionCursor::new(&[1]);
        assert!(trailing.finish().is_err());
    }

    #[test]
    fn skipping_validates_deltas_and_cursor_boundary() {
        let mut cursor = PositionCursor::new(&[1, 2, 3]);
        cursor.skip(3).unwrap();
        assert!(cursor.finish().is_err());

        let mut cursor = PositionCursor::new(&[1, 2, 3]);
        cursor.skip(4).unwrap();
        cursor.finish().unwrap();
    }

    #[test]
    fn decoder_state_stays_allocation_free_and_cache_small() {
        assert!(size_of::<PositionCursor<'_>>() <= 3 * size_of::<usize>());
    }
}
