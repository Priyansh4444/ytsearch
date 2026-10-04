//! Document partitioning for broad grouped scans.
//!
//! The reader serves one backend search at a time, so a broad scan can use
//! every core: one partition per available CPU, up to `MAX_PARTITIONS`.
//! Adding vCPUs therefore shortens broad scans as the corpus grows.
//! Partitions split only where no video's
//! documents straddle the boundary: each video is grouped and moment-counted
//! by exactly one scan, in its original document order, which keeps responses
//! identical to a single scan.

use super::*;
use std::num::NonZeroUsize;

// Below this size thread setup outweighs the saved scan time.
const PARTITIONED_SCAN_MIN_DOCUMENTS: u64 = 1_000_000;
// Seek checkpoints remove each later partition's decode-only prefix, so
// equal document shares balance.
const MAX_PARTITIONS: usize = 8;
// Shorter posting lists are cheap to seek by decoding.
const SEEK_CHECKPOINT_MIN_POSTINGS: u32 = 20_000;

/// Text cursor state after every posting below a partition start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TextSeekCheckpoint {
    pub(super) posting_cursor: usize,
    /// Absent when the term's first-position stream could not be aligned;
    /// position-aware cursors then seek by decoding.
    pub(super) position_cursor: Option<usize>,
    pub(super) previous_document: u32,
    pub(super) decoded: u32,
}

pub(super) struct DocumentPartition {
    /// Ascending starts of every partition after the first.
    pub(super) splits: Vec<u32>,
    /// Per split, keyed by term: where a text cursor resumes.
    pub(super) text_checkpoints: Vec<HashMap<u32, TextSeekCheckpoint>>,
}

impl DocumentPartition {
    /// Plans the partitions and sets them for later scans. Seek checkpoints
    /// decode every long posting list up to each split, so this runs after
    /// `prefetch_hot_mappings`: on a cold volume it took 21-39 s.
    pub(super) fn plan_now(inner: &EngineInner) {
        let plan = Self::plan(
            &inner.videos,
            inner.manifest.documents,
            &inner.lexicon,
            &inner.postings,
            &inner.positions,
        )
        .unwrap_or_else(|error| {
            // Scans validate the same posting streams and report corruption
            // on the query path.
            tracing::warn!(%error, "raw document partition planning failed");
            None
        });
        let _ = inner.partition.set(plan);
    }

    fn parts() -> usize {
        std::thread::available_parallelism()
            .map_or(1, NonZeroUsize::get)
            .min(MAX_PARTITIONS)
    }

    fn worthwhile(documents: u32) -> bool {
        Self::parts() >= 2 && u64::from(documents) >= PARTITIONED_SCAN_MIN_DOCUMENTS
    }

    fn plan(
        videos: &[VideoMeta],
        documents: u32,
        lexicon: &Lexicon,
        postings: &MappedFile,
        positions: &PositionIndex,
    ) -> Result<Option<Self>> {
        if !Self::worthwhile(documents) {
            return Ok(None);
        }
        let parts = Self::parts() as u64;
        let boundaries = video_disjoint_boundaries(videos);
        let mut splits = Vec::with_capacity(parts as usize - 1);
        for part in 1..parts {
            let target = u32::try_from(u64::from(documents) * part / parts)
                .map_err(|_| Error::Corrupt("partition target overflows".to_string()))?;
            if let Some(split) = nearest_boundary(&boundaries, documents, target)
                && splits.last().is_none_or(|last| *last < split)
            {
                splits.push(split);
            }
        }
        if splits.is_empty() {
            return Ok(None);
        }
        let started = std::time::Instant::now();
        let text_checkpoints = text_seek_checkpoints(
            lexicon,
            postings.bytes(),
            positions,
            documents,
            &splits,
            SEEK_CHECKPOINT_MIN_POSTINGS,
        )?;
        tracing::info!(
            ?splits,
            checkpoints = text_checkpoints.first().map_or(0, HashMap::len),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "planned raw document partition"
        );
        Ok(Some(Self {
            splits,
            text_checkpoints,
        }))
    }
}

/// Document IDs `b` such that every video's documents lie entirely below `b`
/// or entirely at or above it. Most videos own one contiguous run, but source
/// order can interleave a few, so boundaries inside those spans are excluded.
pub(super) fn video_disjoint_boundaries(videos: &[VideoMeta]) -> Vec<u32> {
    let mut spans = videos
        .iter()
        .filter_map(|video| Some((*video.documents.first()?, *video.documents.last()?)))
        .collect::<Vec<_>>();
    spans.sort_unstable();
    let mut boundaries = Vec::new();
    let mut covered_through: Option<u32> = None;
    for (first, last) in spans {
        if covered_through.is_some_and(|end| first > end) {
            boundaries.push(first);
        }
        covered_through = Some(covered_through.map_or(last, |end| end.max(last)));
    }
    boundaries
}

fn nearest_boundary(boundaries: &[u32], documents: u32, target: u32) -> Option<u32> {
    let index = boundaries.partition_point(|boundary| *boundary < target);
    [index.checked_sub(1), Some(index)]
        .into_iter()
        .flatten()
        .filter_map(|index| boundaries.get(index).copied())
        .filter(|boundary| (1..documents).contains(boundary))
        .min_by_key(|boundary| boundary.abs_diff(target))
}

/// Decodes each long text posting list once, with the same validation as
/// `TextAlternativeCursor::advance_to`, recording where a cursor resumes at
/// each of the ascending `splits`.
pub(super) fn text_seek_checkpoints(
    lexicon: &Lexicon,
    postings: &[u8],
    positions: &PositionIndex,
    documents: u32,
    splits: &[u32],
    min_postings: u32,
) -> Result<Vec<HashMap<u32, TextSeekCheckpoint>>> {
    let mut checkpoints = vec![HashMap::new(); splits.len()];
    for index in 0..lexicon.count {
        let (_, entry) = lexicon.entry_at(index)?;
        if entry.text_frequency < min_postings {
            continue;
        }
        let end = entry
            .text_offset
            .checked_add(entry.text_length)
            .ok_or_else(|| Error::Corrupt("postings range overflows".to_string()))?;
        let bytes = postings
            .get(entry.text_offset..end)
            .ok_or_else(|| Error::Corrupt("postings range extends beyond file".to_string()))?;
        let first_positions = positions.range(entry.term, IndexedField::Text)?;
        let mut position_cursor = Some(0_usize);
        let mut checkpoint = TextSeekCheckpoint {
            posting_cursor: 0,
            position_cursor: Some(0),
            previous_document: 0,
            decoded: 0,
        };
        let mut next_split = 0;
        let mut cursor = 0;
        while cursor < bytes.len() && next_split < splits.len() {
            let delta = read_varint(bytes, &mut cursor, bytes.len())?;
            if checkpoint.decoded > 0 && delta == 0 {
                return Err(Error::Corrupt(
                    "posting identifiers must be strictly increasing".to_string(),
                ));
            }
            let document = checkpoint
                .previous_document
                .checked_add(delta)
                .ok_or_else(|| Error::Corrupt("posting identifier overflows u32".to_string()))?;
            if document >= documents {
                return Err(Error::Corrupt(
                    "text posting document is out of bounds".to_string(),
                ));
            }
            while next_split < splits.len() && document >= splits[next_split] {
                checkpoints[next_split].insert(entry.term, checkpoint);
                next_split += 1;
            }
            read_varint(bytes, &mut cursor, bytes.len())?;
            position_cursor = position_cursor.and_then(|mut position| {
                read_varint(first_positions, &mut position, first_positions.len())
                    .ok()
                    .map(|_| position)
            });
            checkpoint = TextSeekCheckpoint {
                posting_cursor: cursor,
                position_cursor,
                previous_document: document,
                decoded: checkpoint
                    .decoded
                    .checked_add(1)
                    .ok_or_else(|| Error::Corrupt("posting count overflows u32".to_string()))?,
            };
            if checkpoint.decoded > entry.text_frequency {
                return Err(Error::Corrupt(
                    "posting count exceeds the lexicon frequency".to_string(),
                ));
            }
        }
        // Splits past the list's last posting resume after all of it.
        for split_checkpoints in &mut checkpoints[next_split..] {
            split_checkpoints.insert(entry.term, checkpoint);
        }
    }
    Ok(checkpoints)
}

pub(super) fn merge_disjoint_groups(
    groups: &mut [Option<VideoMatches>],
    partition: Vec<Option<VideoMatches>>,
) -> Result<()> {
    if groups.len() != partition.len() {
        return Err(Error::Corrupt(
            "partitioned group slots do not match".to_string(),
        ));
    }
    for (slot, group) in groups.iter_mut().zip(partition) {
        let Some(group) = group else {
            continue;
        };
        if slot.replace(group).is_some() {
            return Err(Error::Corrupt(
                "a video's documents span two search partitions".to_string(),
            ));
        }
    }
    Ok(())
}

impl RawSearchEngine {
    /// Runs `scan` over the planned partitions, in parallel, and returns the
    /// results in document order. Without a plan, `scan` covers every document.
    pub(super) fn scan_document_partitions<T, F>(&self, scan: F) -> Result<Vec<T>>
    where
        T: Send,
        F: Fn(std::ops::Range<u32>) -> Result<T> + Sync,
    {
        let documents = self.inner.manifest.documents;
        let Some(partition) = self.inner.partition.get().and_then(Option::as_ref) else {
            return Ok(vec![scan(0..documents)?]);
        };
        let starts = std::iter::once(0).chain(partition.splits.iter().copied());
        let ends = partition
            .splits
            .iter()
            .copied()
            .chain(std::iter::once(documents));
        let ranges = starts
            .zip(ends)
            .map(|(start, end)| start..end)
            .collect::<Vec<_>>();
        let (first, rest) = ranges
            .split_first()
            .ok_or_else(|| Error::Corrupt("document partition is empty".to_string()))?;
        std::thread::scope(|scope| {
            let spawned = rest
                .iter()
                .map(|range| {
                    let scan = &scan;
                    scope.spawn(move || scan(range.clone()))
                })
                .collect::<Vec<_>>();
            let mut results = Vec::with_capacity(ranges.len());
            results.push(scan(first.clone()));
            for handle in spawned {
                results.push(
                    handle
                        .join()
                        .map_err(|_| Error::Corrupt("partitioned search panicked".to_string()))?,
                );
            }
            results.into_iter().collect()
        })
    }

    /// Checkpoints that let a scan starting at `start` skip decoding.
    pub(super) fn text_checkpoints_at(
        &self,
        start: u32,
    ) -> Option<&HashMap<u32, TextSeekCheckpoint>> {
        self.inner
            .partition
            .get()
            .and_then(Option::as_ref)
            .and_then(|partition| {
                let index = partition.splits.binary_search(&start).ok()?;
                partition.text_checkpoints.get(index)
            })
    }
}
