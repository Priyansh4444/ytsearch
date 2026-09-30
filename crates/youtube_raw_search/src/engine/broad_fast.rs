//! Schema-9 fast path for broad grouped searches.
//!
//! The schema-7 path scores every matched document to find a page of video
//! groups. With schema-9 accelerators the same response comes from:
//!
//! 1. Exact totals from document sets. A document matches a term plan when
//!    one of its text alternatives has a posting there or one of its title
//!    alternatives matches the document's video; `found`, `total_moments`,
//!    and the matched-document count follow from walking the conjunction.
//! 2. The groups that can reach the page. For relevance, every window of
//!    window of document IDs (its size set by `plan-bounds.bin`) has an upper bound on its best rank: the exact
//!    mask its exact alternatives allow, and the sum of each alternative's
//!    maximum contribution in the window. Windows whose bound cannot beat the
//!    current `required`-th group are never scored. For field sorts, the
//!    groups are the leading matched videos by that field, ties included.
//! 3. Those groups completed from all of their documents with the scan's own
//!    streams, so scores, hits, and moments are the scan's.
//!
//! Exactness argument for (2): a group's best rank over scored windows never
//! exceeds its true rank, so the `required`-th best partial rank θ never
//! exceeds the true one. A skipped window's documents rank below θ, so no
//! group reaching θ has an unscored document that changes its rank, and no
//! group below θ can rise to it.

use super::conjunction::TermPlan;
use super::*;

// Windows scored before the first threshold, chosen by descending bound.
const SEED_WINDOWS: usize = 64;
const THRESHOLD_REFRESH_WINDOWS: usize = 32;
// Bounds are f64 sums of f32 maxima; scores are f32 sums in another order.
const BOUND_MARGIN: f64 = 1.0 + 1e-4;
// A field sort whose boundary ties more groups than this uses the scan.
const MAX_FIELD_SORT_GROUPS: usize = 4_096;

#[cfg(test)]
thread_local! {
    /// Fixture tests shrink the seed batch and refresh interval so a few
    /// hundred documents exercise pruning.
    pub(super) static TEST_WINDOWS: std::cell::Cell<Option<(usize, usize)>> =
        const { std::cell::Cell::new(None) };
    /// (scored, candidate) windows of this thread's last relevance ranking.
    pub(super) static LAST_RELEVANCE_WINDOWS: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
}

fn seed_parameters() -> (usize, usize) {
    #[cfg(test)]
    if let Some(parameters) = TEST_WINDOWS.get() {
        return parameters;
    }
    (SEED_WINDOWS, THRESHOLD_REFRESH_WINDOWS)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GroupOrder {
    Relevance,
    Field,
}

/// The group order a sort uses, or `None` when the fast path does not
/// support it (engagement blends relevance with per-video counters).
pub(super) fn accelerated_group_order(sort_by: Option<&str>) -> Option<GroupOrder> {
    match sort_by {
        Some(
            "published_at_timestamp:desc"
            | "published_at_timestamp:asc"
            | "view_count:desc"
            | "like_count:desc"
            | "comment_count:desc",
        ) => Some(GroupOrder::Field),
        Some("engagement_score:desc") => None,
        // `compare_video_sort` ranks every other value by relevance.
        _ => Some(GroupOrder::Relevance),
    }
}

struct DocumentBits {
    words: Vec<u64>,
}

impl DocumentBits {
    fn new(documents: u32) -> Self {
        Self {
            words: vec![0; (documents as usize).div_ceil(64)],
        }
    }

    fn set(&mut self, document: u32) {
        self.words[(document / 64) as usize] |= 1 << (document % 64);
    }

    fn set_range(&mut self, documents: std::ops::Range<u32>) {
        let (mut start, end) = (documents.start, documents.end);
        while start < end {
            let offset = start % 64;
            let run = (64 - offset).min(end - start);
            let mask = if run == 64 {
                u64::MAX
            } else {
                ((1_u64 << run) - 1) << offset
            };
            self.words[(start / 64) as usize] |= mask;
            start += run;
        }
    }

    fn contains(&self, document: u32) -> bool {
        self.words[(document / 64) as usize] & (1 << (document % 64)) != 0
    }

    fn or_assign(&mut self, other: &Self) {
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            *word |= other;
        }
    }

    fn and_assign(&mut self, other: &Self) {
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            *word &= other;
        }
    }

    fn any_in(&self, documents: std::ops::Range<u32>) -> bool {
        self.ones_in(documents).next().is_some()
    }

    /// Set documents in `documents`, ascending.
    fn ones_in(&self, documents: std::ops::Range<u32>) -> impl Iterator<Item = u32> + '_ {
        let (start, end) = (documents.start, documents.end);
        let first = (start / 64) as usize;
        let last = (end as usize).div_ceil(64).min(self.words.len());
        (first..last).flat_map(move |index| {
            let base = index as u32 * 64;
            let mut word = self.words[index];
            if base < start {
                word &= u64::MAX << (start - base);
            }
            if end - base.min(end) < 64 {
                word &= (1_u64 << (end - base)) - 1;
            }
            std::iter::from_fn(move || {
                (word != 0).then(|| {
                    let bit = word.trailing_zeros();
                    word &= word - 1;
                    base + bit
                })
            })
        })
    }
}

/// Marks `documents` (ascending), filling contiguous runs by word.
fn set_video_documents(bits: &mut DocumentBits, documents: &[u32]) {
    match (documents.first(), documents.last()) {
        (Some(&first), Some(&last)) if (last - first) as usize + 1 == documents.len() => {
            bits.set_range(first..last + 1);
        }
        _ => documents.iter().for_each(|&document| bits.set(document)),
    }
}

pub(super) struct BroadTotals {
    pub(super) documents: usize,
    pub(super) found: usize,
    pub(super) total_moments: usize,
}

struct PlanAnalysis {
    text: DocumentBits,
    /// Text or title matches.
    matched: DocumentBits,
    window_masks: Vec<u8>,
    window_bounds: Vec<f64>,
}

pub(super) struct BroadAnalysis {
    pub(super) totals: BroadTotals,
    matched: DocumentBits,
    /// Matched videos, ascending.
    videos: Vec<u32>,
    window_documents: u32,
    pub(super) window_masks: Vec<u8>,
    pub(super) window_bounds: Vec<f64>,
}

#[derive(Clone, Copy)]
struct GroupRank {
    exact_tokens: u8,
    exact_mask: u8,
    score: f32,
}

impl GroupRank {
    fn of(candidate: DocMatch) -> Self {
        Self {
            exact_tokens: candidate.exact_tokens(),
            exact_mask: candidate.exact_mask(),
            score: candidate.score,
        }
    }

    /// Order-preserving for the non-negative scores ranks carry; zero is
    /// reserved for "no rank".
    fn encode(self) -> u64 {
        (u64::from(self.exact_tokens) << 40)
            | (u64::from(self.exact_mask) << 32)
            | u64::from(self.score.max(f32::MIN_POSITIVE).to_bits())
    }

    fn decode(value: u64) -> Option<Self> {
        (value != 0).then(|| Self {
            exact_tokens: (value >> 40) as u8,
            exact_mask: (value >> 32) as u8,
            score: f32::from_bits(value as u32),
        })
    }

    /// Descending relevance without the video tie-break; the broad scan
    /// never sets the ordered-phrase flag.
    fn cmp(self, other: Self) -> std::cmp::Ordering {
        (self.exact_tokens, self.exact_mask)
            .cmp(&(other.exact_tokens, other.exact_mask))
            .then_with(|| self.score.total_cmp(&other.score))
    }
}

impl RawSearchEngine {
    /// Exact totals and per-window rank bounds for a broad plan set, or
    /// `None` without schema-9 accelerators.
    ///
    /// Relevance needs a tight bound for every plan: a plan with several
    /// alternatives uses its stored plan bound, and without one the scan is
    /// faster than pruning with summed alternative maxima.
    pub(super) fn analyze_broad_plans(
        &self,
        plans: &[TermPlan],
        order: GroupOrder,
    ) -> Result<Option<BroadAnalysis>> {
        let (Some(_), Some(_), Some(plan_bounds)) = (
            self.inner.accelerators.as_ref(),
            self.inner.bitmaps.as_ref(),
            self.inner.plan_bounds.as_ref(),
        ) else {
            return Ok(None);
        };
        let mut stored_bounds = Vec::with_capacity(plans.len());
        for plan in plans {
            let bound = (plan.alternatives.len() > 1)
                .then(|| {
                    plan.alternatives
                        .iter()
                        .find(|alternative| alternative.fields.exact_mask != 0)
                        .and_then(|exact| plan_bounds.get(exact.entry.term))
                        .filter(|bound| bound.covers(plan))
                })
                .flatten();
            if order == GroupOrder::Relevance && plan.alternatives.len() > 1 && bound.is_none() {
                return Ok(None);
            }
            stored_bounds.push(bound);
        }
        let documents = self.inner.manifest.documents;
        let window_documents = plan_bounds.window_documents();
        let windows = documents.div_ceil(window_documents) as usize;
        let mut window_masks = vec![0_u8; windows];
        let mut window_bounds = vec![0.0_f64; windows];
        let mut matched: Option<DocumentBits> = None;
        let mut text_any = DocumentBits::new(documents);

        // Plans are independent; build them in parallel and combine them in
        // plan order.
        let analyses = std::thread::scope(|scope| {
            let handles = plans
                .iter()
                .zip(&stored_bounds)
                .map(|(plan, stored_bound)| {
                    scope.spawn(move || {
                        self.analyze_plan(plan, stored_bound.as_ref(), window_documents)
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .map_err(|_| Error::Corrupt("plan analysis panicked".to_string()))?
                })
                .collect::<Result<Vec<_>>>()
        })?;
        for analysis in analyses {
            for (window, bound) in window_bounds.iter_mut().zip(&analysis.window_bounds) {
                *window += bound;
            }
            for (window, mask) in window_masks.iter_mut().zip(&analysis.window_masks) {
                *window |= mask;
            }
            text_any.or_assign(&analysis.text);
            match matched.as_mut() {
                Some(matched) => matched.and_assign(&analysis.matched),
                None => matched = Some(analysis.matched),
            }
        }
        let Some(matched) = matched else {
            return Ok(None);
        };

        let partitions = self.scan_document_partitions(|range| {
            let mut counters: Vec<Option<MomentCounter>> = Vec::new();
            counters.resize_with(self.inner.videos.len(), || None);
            let mut matched_documents = 0_usize;
            for document in matched.ones_in(range) {
                matched_documents += 1;
                let record = self.inner.metadata.search_document(document)?;
                let counter = counters
                    .get_mut(record.video as usize)
                    .ok_or_else(|| Error::Corrupt("document video is out of bounds".to_string()))?
                    .get_or_insert_with(MomentCounter::default);
                if text_any.contains(document) {
                    counter.record(record.t_start_ms);
                }
            }
            Ok((matched_documents, counters))
        })?;
        let mut totals = BroadTotals {
            documents: 0,
            found: 0,
            total_moments: 0,
        };
        let mut videos = Vec::new();
        for (matched_documents, counters) in partitions {
            totals.documents += matched_documents;
            for (video, counter) in counters.iter().enumerate() {
                if let Some(counter) = counter {
                    totals.found += 1;
                    totals.total_moments += counter.visible();
                    videos.push(video as u32);
                }
            }
        }
        videos.sort_unstable();
        videos.dedup();
        if videos.len() != totals.found {
            return Err(Error::Corrupt(
                "a video's documents span two scan partitions".to_string(),
            ));
        }
        Ok(Some(BroadAnalysis {
            totals,
            matched,
            videos,
            window_documents,
            window_masks,
            window_bounds,
        }))
    }

    /// One plan's text and match sets, and its window bound and mask
    /// contributions.
    fn analyze_plan(
        &self,
        plan: &TermPlan,
        stored_bound: Option<&super::plan_bounds::PlanBound<'_>>,
        window_documents: u32,
    ) -> Result<PlanAnalysis> {
        let (Some(accelerators), Some(bitmaps)) = (
            self.inner.accelerators.as_ref(),
            self.inner.bitmaps.as_ref(),
        ) else {
            return Err(Error::Corrupt(
                "schema-9 accelerators are missing".to_string(),
            ));
        };
        let documents = self.inner.manifest.documents;
        let average = self.inner.manifest.average_document_tokens;
        let windows = documents.div_ceil(window_documents) as usize;
        let mut window_masks = vec![0_u8; windows];
        let mut window_bounds = vec![0.0_f64; windows];
        let mut window_maxima = vec![0.0_f32; windows];
        let alternative_bounds = stored_bound.is_none();
        let mut plan_text = DocumentBits::new(documents);
        let mut plan_title = DocumentBits::new(documents);
        for alternative in &plan.alternatives {
            let entry = alternative.entry;
            let exact_mask = alternative.fields.exact_mask;
            if alternative.fields.text && entry.text_frequency > 0 {
                window_maxima.fill(0.0);
                let blocks = accelerators.blocks(entry.term)?;
                if let Some(blocks) = &blocks {
                    let mut first = 0_u32;
                    for block in blocks {
                        let (start, end) = (
                            (first / window_documents) as usize,
                            (block.last_document / window_documents) as usize,
                        );
                        for maximum in &mut window_maxima[start..=end] {
                            *maximum = maximum.max(block.max_factor);
                        }
                        first = block.last_document + 1;
                    }
                }
                let bitmap = bitmaps.words(entry.term, IndexedField::Text);
                let decode_postings = bitmap.is_none() || blocks.is_none();
                if let Some(words) = bitmap {
                    for (word, bits) in plan_text.words.iter_mut().zip(words) {
                        *word |= bits;
                    }
                }
                if decode_postings {
                    let postings = self.postings_range(entry.text_offset, entry.text_length)?;
                    let positions = self.inner.positions.range(entry.term, IndexedField::Text)?;
                    let (mut cursor, mut position_cursor, mut document) = (0, 0, 0_u32);
                    let mut decoded = 0_u32;
                    while cursor < postings.len() {
                        document = document
                            .checked_add(read_varint(postings, &mut cursor, postings.len())?)
                            .filter(|document| *document < documents)
                            .ok_or_else(|| {
                                Error::Corrupt("text posting document is out of bounds".to_string())
                            })?;
                        let frequency = read_varint(postings, &mut cursor, postings.len())?;
                        plan_text.set(document);
                        decoded += 1;
                        if blocks.is_none() {
                            let first_position =
                                read_varint(positions, &mut position_cursor, positions.len())?;
                            let tokens = self.inner.metadata.search_document(document)?.tokens;
                            let factor = accelerators::score_factor(
                                frequency,
                                tokens,
                                average,
                                first_position,
                            );
                            let window = &mut window_maxima[(document / window_documents) as usize];
                            *window = window.max(factor);
                        }
                    }
                    if decoded != entry.text_frequency {
                        return Err(Error::Corrupt(
                            "text posting count does not match lexicon".to_string(),
                        ));
                    }
                }
                let constant = f64::from(self.alternative_idf(entry))
                    * f64::from(TEXT_QUERY_WEIGHT)
                    * f64::from(alternative.score_multiplier);
                for (window, maximum) in window_maxima.iter().enumerate() {
                    if *maximum > 0.0 {
                        if alternative_bounds {
                            window_bounds[window] += constant * f64::from(*maximum);
                        }
                        window_masks[window] |= exact_mask;
                    }
                }
            }
            if alternative.fields.title {
                window_maxima.fill(0.0);
                for (video, score) in self.title_video_scores(alternative)? {
                    let video_documents = &self.inner.videos[video as usize].documents;
                    set_video_documents(&mut plan_title, video_documents);
                    let (Some(&first), Some(&last)) =
                        (video_documents.first(), video_documents.last())
                    else {
                        continue;
                    };
                    for maximum in &mut window_maxima
                        [(first / window_documents) as usize..=(last / window_documents) as usize]
                    {
                        *maximum = maximum.max(score);
                    }
                }
                for (window, maximum) in window_maxima.iter().enumerate() {
                    if *maximum > 0.0 {
                        if alternative_bounds {
                            window_bounds[window] += f64::from(*maximum);
                        }
                        window_masks[window] |= exact_mask;
                    }
                }
            }
        }
        if let Some(bound) = stored_bound {
            for (window, maximum) in window_bounds.iter_mut().zip(bound.maxima()) {
                *window += maximum;
            }
        }
        let mut matched = DocumentBits {
            words: plan_text.words.clone(),
        };
        matched.or_assign(&plan_title);
        Ok(PlanAnalysis {
            text: plan_text,
            matched,
            window_masks,
            window_bounds,
        })
    }

    /// The response page for a broad grouped search, identical to formatting
    /// the complete scan's groups.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn accelerated_broad_page(
        &self,
        params: &SearchParams,
        plans: &[TermPlan],
        analysis: &BroadAnalysis,
        order: GroupOrder,
        highlights: &[Vec<String>],
        hits_per_group: usize,
        dominant_exact_mask: u8,
        cache: &mut BlockCache,
    ) -> Result<Option<Value>> {
        let (offset, limit) = result_window(params);
        let required = offset.saturating_add(limit);
        let page_videos = match order {
            GroupOrder::Relevance => {
                let ranked = self.relevance_leading_videos(plans, analysis, required)?;
                ranked.into_iter().skip(offset).take(limit).collect()
            }
            GroupOrder::Field => {
                let Some(candidates) = self.field_candidate_videos(
                    &analysis.videos,
                    params.sort_by.as_deref(),
                    required,
                ) else {
                    return Ok(None);
                };
                candidates
            }
        };
        let mut groups =
            self.complete_groups(plans, &page_videos, hits_per_group, dominant_exact_mask)?;
        let page = match order {
            GroupOrder::Relevance => {
                let len = groups.len();
                self.sort_group_window(&mut groups, params.sort_by.as_deref(), len);
                groups
            }
            GroupOrder::Field => {
                self.sort_group_window(&mut groups, params.sort_by.as_deref(), required);
                groups.into_iter().skip(offset).take(limit).collect()
            }
        };
        self.format_group_page(
            analysis.totals.found,
            analysis.totals.total_moments,
            page,
            highlights,
            cache,
        )
        .map(Some)
    }

    /// The `required` best groups by relevance, in response order.
    fn relevance_leading_videos(
        &self,
        plans: &[TermPlan],
        analysis: &BroadAnalysis,
        required: usize,
    ) -> Result<Vec<u32>> {
        let documents = self.inner.manifest.documents;
        let window_documents = analysis.window_documents;
        let (seed_windows, refresh_windows) = seed_parameters();
        let window_range = |window: usize| {
            let start = window as u32 * window_documents;
            start..start.saturating_add(window_documents).min(documents)
        };
        let window_rank = |window: usize| {
            let mask = analysis.window_masks[window];
            (
                mask.count_ones() as u8,
                mask,
                analysis.window_bounds[window] * BOUND_MARGIN,
            )
        };
        let cannot_reach = |window: usize, threshold: GroupRank| {
            let (tokens, mask, bound) = window_rank(window);
            (tokens, mask)
                .cmp(&(threshold.exact_tokens, threshold.exact_mask))
                .then_with(|| bound.total_cmp(&f64::from(threshold.score)))
                .is_lt()
        };
        let mut seeds = (0..analysis.window_bounds.len())
            .filter(|&window| analysis.matched.any_in(window_range(window)))
            .collect::<Vec<_>>();
        seeds.sort_unstable_by(|&left, &right| {
            let (left, right) = (window_rank(left), window_rank(right));
            (right.0, right.1)
                .cmp(&(left.0, left.1))
                .then_with(|| right.2.total_cmp(&left.2))
        });
        seeds.truncate(seed_windows);
        seeds.sort_unstable();

        // Partitions hold disjoint videos, so each partition's `required`-th
        // best group is at most the global one and may prune every
        // partition. The shared value only grows.
        let shared = std::sync::atomic::AtomicU64::new(0);
        let publish = |rank: Option<GroupRank>| {
            if let Some(rank) = rank {
                shared.fetch_max(rank.encode(), std::sync::atomic::Ordering::Relaxed);
            }
        };
        let partitions = self.scan_document_partitions(|range| {
            let mut best: Vec<Option<GroupRank>> = vec![None; self.inner.videos.len()];
            let (mut scored, mut candidates) = (0_usize, 0_usize);
            if range.is_empty() {
                return Ok((best, scored, candidates));
            }
            let clip = |window: usize| {
                let documents = window_range(window);
                documents.start.max(range.start)..documents.end.min(range.end)
            };
            let first = (range.start / window_documents) as usize;
            let last = ((range.end - 1) / window_documents) as usize;
            let score = |streams: &mut [conjunction::StreamingTermPlan<'_>],
                         best: &mut [Option<GroupRank>],
                         documents: std::ops::Range<u32>|
             -> Result<()> {
                self.visit_seeking_range(streams, documents, |candidate, record| {
                    let rank = GroupRank::of(candidate);
                    let slot = &mut best[record.video as usize];
                    if slot.is_none_or(|current| rank.cmp(current).is_gt()) {
                        *slot = Some(rank);
                    }
                    Ok(())
                })
            };
            let own_seeds = seeds
                .iter()
                .copied()
                .filter(|window| (first..=last).contains(window))
                .collect::<Vec<_>>();
            let mut streams = self.seeking_streams(plans)?;
            for &window in &own_seeds {
                score(&mut streams, &mut best, clip(window))?;
                scored += 1;
            }
            let mut local = required_rank(&best, required);
            publish(local);
            let mut streams = self.seeking_streams(plans)?;
            let mut since_refresh = 0;
            for window in first..=last {
                let documents = clip(window);
                if !analysis.matched.any_in(documents.clone()) {
                    continue;
                }
                candidates += 1;
                if own_seeds.binary_search(&window).is_ok() {
                    continue;
                }
                let global = GroupRank::decode(shared.load(std::sync::atomic::Ordering::Relaxed));
                let threshold = match (local, global) {
                    (Some(local), Some(global)) => Some(if local.cmp(global).is_ge() {
                        local
                    } else {
                        global
                    }),
                    (local, global) => local.or(global),
                };
                if threshold.is_some_and(|threshold| cannot_reach(window, threshold)) {
                    continue;
                }
                score(&mut streams, &mut best, documents)?;
                scored += 1;
                since_refresh += 1;
                if since_refresh == refresh_windows {
                    local = required_rank(&best, required);
                    publish(local);
                    since_refresh = 0;
                }
            }
            Ok((best, scored, candidates))
        })?;
        let mut best: Vec<Option<GroupRank>> = vec![None; self.inner.videos.len()];
        let (mut scored_windows, mut candidate_windows) = (0, 0);
        for (partition_best, scored, candidates) in partitions {
            scored_windows += scored;
            candidate_windows += candidates;
            for (slot, rank) in best.iter_mut().zip(partition_best) {
                if let Some(rank) = rank {
                    if slot.is_some() {
                        return Err(Error::Corrupt(
                            "a video's documents span two scan partitions".to_string(),
                        ));
                    }
                    *slot = Some(rank);
                }
            }
        }
        tracing::debug!(
            scored = scored_windows,
            candidates = candidate_windows,
            "raw accelerated relevance windows"
        );
        #[cfg(test)]
        LAST_RELEVANCE_WINDOWS.set((scored_windows, candidate_windows));

        let mut ranked = best
            .iter()
            .enumerate()
            .filter_map(|(video, rank)| rank.map(|rank| (video as u32, rank)))
            .collect::<Vec<_>>();
        let compare = |left: &(u32, GroupRank), right: &(u32, GroupRank)| {
            right.1.cmp(left.1).then_with(|| {
                self.inner.videos[left.0 as usize]
                    .id
                    .cmp(&self.inner.videos[right.0 as usize].id)
            })
        };
        let required = required.min(ranked.len());
        if required < ranked.len() && required > 0 {
            ranked.select_nth_unstable_by(required, compare);
        }
        ranked.truncate(required);
        ranked.sort_unstable_by(compare);
        Ok(ranked.into_iter().map(|(video, _)| video).collect())
    }

    /// Matched videos that can occupy the first `required` places of a field
    /// sort: the leaders by that field and everything tied with the last.
    fn field_candidate_videos(
        &self,
        videos: &[u32],
        sort_by: Option<&str>,
        required: usize,
    ) -> Option<Vec<u32>> {
        let key = |video: u32| -> i128 {
            let video = &self.inner.videos[video as usize];
            match sort_by {
                Some("published_at_timestamp:desc") => i128::from(video.published_at_timestamp),
                Some("published_at_timestamp:asc") => -i128::from(video.published_at_timestamp),
                Some("view_count:desc") => i128::from(video.view_count),
                Some("like_count:desc") => i128::from(video.like_count),
                Some("comment_count:desc") => i128::from(video.comment_count),
                _ => 0,
            }
        };
        let mut ranked = videos.to_vec();
        ranked.sort_unstable_by_key(|&video| std::cmp::Reverse(key(video)));
        if required < ranked.len() {
            let boundary = key(ranked[required.saturating_sub(1)]);
            let end = ranked.partition_point(|&video| key(video) >= boundary);
            ranked.truncate(if required == 0 { 0 } else { end });
        }
        (ranked.len() <= MAX_FIELD_SORT_GROUPS.max(required)).then_some(ranked)
    }

    /// Groups for `videos` from all of their matched documents, recorded in
    /// document order as the scan records them.
    fn complete_groups(
        &self,
        plans: &[TermPlan],
        videos: &[u32],
        hits_per_group: usize,
        dominant_exact_mask: u8,
    ) -> Result<Vec<VideoMatches>> {
        let mut runs = Vec::new();
        for &video in videos {
            let documents = &self.inner.videos[video as usize].documents;
            let mut index = 0;
            while index < documents.len() {
                let start = documents[index];
                let mut end = start + 1;
                index += 1;
                while documents.get(index) == Some(&end) {
                    end += 1;
                    index += 1;
                }
                runs.push(start..end);
            }
        }
        runs.sort_unstable_by_key(|run| run.start);
        let mut slots = self.empty_group_slots();
        let mut streams = self.seeking_streams(plans)?;
        for run in runs {
            self.visit_seeking_range(&mut streams, run, |candidate, record| {
                self.record_group_match_values(
                    &mut slots,
                    candidate,
                    record.video,
                    record.t_start_ms,
                    hits_per_group,
                    Some(dominant_exact_mask),
                )
            })?;
        }
        videos
            .iter()
            .map(|&video| {
                slots[video as usize].take().ok_or_else(|| {
                    Error::Corrupt("a matched video has no scored document".to_string())
                })
            })
            .collect()
    }
}

/// The `required`-th best group rank so far, once that many groups exist.
fn required_rank(best: &[Option<GroupRank>], required: usize) -> Option<GroupRank> {
    if required == 0 {
        return None;
    }
    let mut ranks = best.iter().flatten().copied().collect::<Vec<_>>();
    if ranks.len() < required {
        return None;
    }
    let (_, rank, _) = ranks.select_nth_unstable_by(required - 1, |left, right| right.cmp(*left));
    Some(*rank)
}
