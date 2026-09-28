//! Lazily built seek checkpoints for long positional posting lists.
//!
//! Phrase and proximity verification walks every query token's posting,
//! first-position, and remaining-position streams to reach a sparse set of
//! target documents. For a word such as "the" that decodes millions of
//! postings to verify a few thousand targets. A checkpoint records the three
//! stream offsets after every `SKIP_INTERVAL` postings, so a seek decodes at
//! most one interval. Checkpoints are built on first use of a term/field,
//! with the same validation as a full walk, and bounded by a byte budget.
//! A schema-9 generation stores transcript checkpoints in `accelerators.bin`;
//! they are validated against the term's streams and used instead.

use super::*;
use std::sync::{Mutex, OnceLock};

pub(super) const SKIP_INTERVAL: u32 = 64;
// Shorter lists are cheap to walk; a checkpoint would not pay for itself.
pub(super) const SKIP_MIN_POSTINGS: u32 = 4_096;
const SKIP_BUDGET_BYTES: usize = 64 * 1024 * 1024;

/// Cursor state after `decoded` postings, all with IDs at most `previous_id`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SkipEntry {
    pub(super) previous_id: u32,
    pub(super) decoded: u32,
    pub(super) posting_cursor: usize,
    pub(super) first_cursor: usize,
    pub(super) remaining_cursor: usize,
}

type SkipSlot = Arc<OnceLock<Option<Arc<[SkipEntry]>>>>;

#[derive(Default)]
pub(super) struct SkipDirectory {
    slots: Mutex<HashMap<(u32, IndexedField), SkipSlot>>,
    bytes: std::sync::atomic::AtomicUsize,
}

impl SkipDirectory {
    /// Checkpoints for one term/field, built once per generation. `None`
    /// means walk without them: the list is short, the budget is spent, or
    /// the build failed (the walk then reports the same corruption).
    pub(super) fn get_or_build(
        &self,
        term: u32,
        field: IndexedField,
        streams: PositionedStreams<'_>,
        sidecar: Option<&super::accelerators::Accelerators>,
    ) -> Option<Arc<[SkipEntry]>> {
        let sidecar =
            sidecar.filter(|sidecar| field == IndexedField::Text && sidecar.contains(term));
        if sidecar.is_none() && streams.expected < SKIP_MIN_POSTINGS {
            return None;
        }
        let slot = {
            let mut slots = self.slots.lock().ok()?;
            Arc::clone(slots.entry((term, field)).or_default())
        };
        slot.get_or_init(|| {
            let estimate =
                (streams.expected / SKIP_INTERVAL) as usize * std::mem::size_of::<SkipEntry>();
            let reserved = self
                .bytes
                .fetch_add(estimate, std::sync::atomic::Ordering::Relaxed);
            let release = || {
                self.bytes
                    .fetch_sub(estimate, std::sync::atomic::Ordering::Relaxed);
            };
            if reserved.saturating_add(estimate) > SKIP_BUDGET_BYTES {
                release();
                return None;
            }
            if let Some(sidecar) = sidecar {
                match sidecar.checkpoints(term, streams) {
                    Ok(Some(entries)) => return Some(entries),
                    Ok(None) => {}
                    Err(error) => {
                        release();
                        tracing::warn!(%error, term, "raw accelerator checkpoints rejected");
                        return None;
                    }
                }
            }
            match build_skip_entries(streams, SKIP_INTERVAL) {
                Ok(entries) => Some(entries.into()),
                Err(error) => {
                    release();
                    tracing::warn!(%error, term, "raw posting skip build failed");
                    None
                }
            }
        })
        .clone()
    }
}

#[derive(Clone, Copy)]
pub(super) struct PositionedStreams<'a> {
    pub(super) postings: &'a [u8],
    pub(super) first_positions: &'a [u8],
    pub(super) remaining_positions: &'a [u8],
    pub(super) expected: u32,
}

/// Decodes a complete list with `PositionedPostingCursor`'s validation and
/// records its state every `interval` postings.
pub(super) fn build_skip_entries(
    streams: PositionedStreams<'_>,
    interval: u32,
) -> Result<Vec<SkipEntry>> {
    let mut cursor = PositionedPostingCursor::new(
        streams.postings,
        streams.first_positions,
        streams.remaining_positions,
        streams.expected,
        None,
    );
    let mut entries = Vec::with_capacity((streams.expected / interval.max(1)) as usize);
    let mut scratch = Vec::new();
    let mut target = 0;
    loop {
        if cursor.decoded > 0 && cursor.decoded.is_multiple_of(interval) {
            entries.push(cursor.skip_entry());
        }
        let Some(id) = cursor.next_at_least(target, &mut scratch)? else {
            return Ok(entries);
        };
        target = id
            .checked_add(1)
            .ok_or_else(|| Error::Corrupt("posting identifier overflows u32".to_string()))?;
    }
}
