//! P01 measurement: exact grouped totals from term bitsets.
//!
//! Schema 9 plans to answer `found` and `total_moments` from exact document
//! sets (dense bitmaps for high-DF terms) plus per-document fast fields,
//! instead of scoring every matching posting. This diagnostic builds those
//! sets from a real generation with the engine's own term plans, times the
//! query-time set work separately from set construction, and requires the
//! totals to equal the current engine response.
//!
//! Run against a local generation:
//! `RAW_TOTALS_INDEX=/abs/path/to/index cargo test --release -p youtube_raw_search --lib measure_exact_totals_from_sets -- --ignored --nocapture`

use super::*;
use std::time::Instant;

const DEFAULT_QUERIES: &[&str] = &[
    "and the",
    "the",
    "in the",
    "this is the",
    "this",
    "for the",
    "you know",
    "fire",
    "react",
    "this is the end",
    "the problem is that",
];

struct Bits(Vec<u64>);

impl Bits {
    fn new(documents: u32) -> Self {
        Self(vec![0; (documents as usize).div_ceil(64)])
    }

    fn set(&mut self, document: u32) {
        self.0[(document / 64) as usize] |= 1 << (document % 64);
    }

    fn or_assign(&mut self, other: &Self) {
        for (word, other) in self.0.iter_mut().zip(&other.0) {
            *word |= other;
        }
    }

    fn and_assign(&mut self, other: &Self) {
        for (word, other) in self.0.iter_mut().zip(&other.0) {
            *word &= other;
        }
    }

    fn contains(&self, document: u32) -> bool {
        self.0[(document / 64) as usize] & (1 << (document % 64)) != 0
    }

    fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().enumerate().flat_map(|(index, &word)| {
            let mut word = word;
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let bit = word.trailing_zeros();
                word &= word - 1;
                Some(index as u32 * 64 + bit)
            })
        })
    }
}

/// Per term plan: documents matched through transcript text, and documents
/// whose video title matches. Built once per term in schema 9.
fn plan_sets(engine: &RawSearchEngine, plan: &conjunction::TermPlan) -> (Bits, Bits) {
    let documents = engine.inner.manifest.documents;
    let mut text = Bits::new(documents);
    let mut title = Bits::new(documents);
    for alternative in &plan.alternatives {
        let entry = alternative.entry;
        if alternative.fields.text {
            let bytes = engine
                .postings_range(entry.text_offset, entry.text_length)
                .expect("text postings");
            let (mut cursor, mut document) = (0, 0_u32);
            while cursor < bytes.len() {
                document += read_varint(bytes, &mut cursor, bytes.len()).expect("doc delta");
                read_varint(bytes, &mut cursor, bytes.len()).expect("frequency");
                text.set(document);
            }
        }
        if alternative.fields.title {
            let bytes = engine
                .postings_range(entry.title_offset, entry.title_length)
                .expect("title postings");
            let (mut cursor, mut video) = (0, 0_u32);
            while cursor < bytes.len() {
                video += read_varint(bytes, &mut cursor, bytes.len()).expect("video delta");
                read_varint(bytes, &mut cursor, bytes.len()).expect("frequency");
                for &document in &engine.inner.videos[video as usize].documents {
                    title.set(document);
                }
            }
        }
    }
    (text, title)
}

#[test]
#[ignore = "diagnostic: needs RAW_TOTALS_INDEX pointing at a real generation"]
fn measure_exact_totals_from_sets() {
    let index = std::env::var("RAW_TOTALS_INDEX").expect("set RAW_TOTALS_INDEX");
    let engine = RawSearchEngine::open(Path::new(&index)).expect("open generation");
    let fields = query_fields(Some("text,title"), true).expect("query fields");
    let queries = std::env::var("RAW_TOTALS_QUERIES")
        .map(|value| value.split('|').map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_else(|_| {
            DEFAULT_QUERIES
                .iter()
                .map(|query| query.to_string())
                .collect()
        });
    for query in queries {
        let terms = parse_query(&query).expect("parse").terms;
        let (plans, _) = engine
            .conjunction_plans(&terms, fields, true, 0)
            .expect("plans");

        let started = Instant::now();
        let sets = plans
            .iter()
            .map(|plan| plan_sets(&engine, plan))
            .collect::<Vec<_>>();
        let build_ms = started.elapsed().as_secs_f64() * 1e3;

        let started = Instant::now();
        let mut matched: Option<Bits> = None;
        let mut text_any = Bits::new(engine.inner.manifest.documents);
        for (text, title) in &sets {
            let mut term = Bits(text.0.clone());
            term.or_assign(title);
            match matched.as_mut() {
                Some(matched) => matched.and_assign(&term),
                None => matched = Some(term),
            }
            text_any.or_assign(text);
        }
        let matched = matched.expect("at least one plan");
        let set_ms = started.elapsed().as_secs_f64() * 1e3;

        // Video-indexed counters, as the engine's group slots are.
        let started = Instant::now();
        let mut moments: Vec<Option<MomentCounter>> = Vec::new();
        moments.resize_with(engine.inner.videos.len(), || None);
        let mut matched_documents = 0_u64;
        for document in matched.ones() {
            matched_documents += 1;
            let record = engine
                .inner
                .metadata
                .search_document(document)
                .expect("document record");
            let counter = moments[record.video as usize].get_or_insert_with(MomentCounter::default);
            if text_any.contains(document) {
                counter.record(record.t_start_ms);
            }
        }
        let found = moments.iter().flatten().count();
        let total_moments = moments
            .iter()
            .flatten()
            .map(MomentCounter::visible)
            .sum::<usize>();
        let totals_ms = started.elapsed().as_secs_f64() * 1e3;

        let started = Instant::now();
        let response = engine
            .search(&grouped_query(&query))
            .expect("engine search");
        let engine_ms = started.elapsed().as_secs_f64() * 1e3;
        let matches = response["found"] == found && response["total_moments"] == total_moments;
        println!(
            "{{\"query\":{:?},\"matched_documents\":{matched_documents},\"found\":{found},\"total_moments\":{total_moments},\"engine_found\":{},\"engine_total_moments\":{},\"equal\":{matches},\"set_build_ms\":{build_ms:.1},\"set_combine_ms\":{set_ms:.1},\"totals_scan_ms\":{totals_ms:.1},\"engine_search_ms\":{engine_ms:.1}}}",
            query, response["found"], response["total_moments"]
        );
        assert!(matches, "totals differ for {query:?}");
    }
}

/// Rank key of one matched document or of a group's best document.
#[derive(Clone, Copy)]
struct Rank {
    exact_tokens: u8,
    exact_mask: u8,
    ordered: bool,
    score: f32,
}

impl Rank {
    fn of(candidate: &DocMatch) -> Self {
        Self {
            exact_tokens: candidate.exact_tokens(),
            exact_mask: candidate.exact_mask(),
            ordered: candidate.ordered_phrase(),
            score: candidate.score,
        }
    }

    fn tier(self) -> (u8, u8, bool) {
        (self.exact_tokens, self.exact_mask, self.ordered)
    }

    /// Descending relevance, as `compare_video_sort` without a sort.
    fn better(self, other: Self) -> bool {
        self.tier()
            .cmp(&other.tier())
            .then_with(|| self.score.total_cmp(&other.score))
            == std::cmp::Ordering::Greater
    }
}

fn ranked_videos(engine: &RawSearchEngine, groups: &[Option<Rank>], limit: usize) -> Vec<u32> {
    let mut ranked = groups
        .iter()
        .enumerate()
        .filter_map(|(video, rank)| rank.map(|rank| (video as u32, rank)))
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|(left_video, left), (right_video, right)| {
        right
            .tier()
            .cmp(&left.tier())
            .then_with(|| right.score.total_cmp(&left.score))
            .then_with(|| {
                engine.inner.videos[*left_video as usize]
                    .id
                    .cmp(&engine.inner.videos[*right_video as usize].id)
            })
    });
    ranked.truncate(limit);
    ranked.into_iter().map(|(video, _)| video).collect()
}

const BLOCK_DOCUMENTS: u32 = 128;

/// A block's maximum tier, summed per-plan score bound, and matched indexes.
type BlockBound = ((u8, u8, bool), f64, Vec<usize>);
const TOP_GROUPS: usize = 21;

#[test]
#[ignore = "diagnostic: needs RAW_TOTALS_INDEX pointing at a real generation"]
fn simulate_block_priority_group_ranking() {
    let index = std::env::var("RAW_TOTALS_INDEX").expect("set RAW_TOTALS_INDEX");
    let engine = RawSearchEngine::open(Path::new(&index)).expect("open generation");
    let fields = query_fields(Some("text,title"), true).expect("query fields");
    let documents = engine.inner.manifest.documents;
    let queries = std::env::var("RAW_TOTALS_QUERIES")
        .map(|value| value.split('|').map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_else(|_| {
            [
                "and the",
                "in the",
                "this is the",
                "for the",
                "you know",
                "this is the end",
            ]
            .iter()
            .map(|query| query.to_string())
            .collect()
        });
    for query in queries {
        let terms = parse_query(&query).expect("parse").terms;
        let (plans, _) = engine
            .conjunction_plans(&terms, fields, true, 0)
            .expect("plans");
        if plans.len() < 2 {
            continue;
        }
        // Per-plan scores give a realistic per-block bound: the sum over
        // plans of each plan's block maximum, as per-term block maxima would.
        let mut plan_scores = Vec::new();
        for plan in &plans {
            let mut scores = vec![f32::NEG_INFINITY; documents as usize];
            engine
                .visit_term_plan(plan, |candidate, _| {
                    scores[candidate.document() as usize] = candidate.score;
                    Ok(())
                })
                .expect("plan scan");
            plan_scores.push(scores);
        }
        let mut matched = Vec::new();
        engine
            .visit_plan_conjunction(&plans, |candidate, record| {
                matched.push((candidate.document(), record.video, Rank::of(&candidate)));
                Ok(())
            })
            .expect("conjunction scan");

        // Ground truth: every matched document scored.
        let mut all = vec![None::<Rank>; engine.inner.videos.len()];
        for &(_, video, rank) in &matched {
            let best = &mut all[video as usize];
            if best.is_none_or(|best| rank.better(best)) {
                *best = Some(rank);
            }
        }
        let expected = ranked_videos(&engine, &all, TOP_GROUPS);

        // Blocks with bounds: true maximum tier (a per-term exact flag gives
        // the same or a looser tier) and the summed per-plan score maxima.
        let mut blocks: HashMap<u32, BlockBound> = HashMap::new();
        for (index, &(document, _, rank)) in matched.iter().enumerate() {
            let block = blocks.entry(document / BLOCK_DOCUMENTS).or_insert((
                (0, 0, false),
                0.0,
                Vec::new(),
            ));
            block.0 = block.0.max(rank.tier());
            block.2.push(index);
        }
        for (block, bound) in blocks.iter_mut() {
            let start = (block * BLOCK_DOCUMENTS) as usize;
            let end = ((block + 1) * BLOCK_DOCUMENTS).min(documents) as usize;
            bound.1 = plan_scores
                .iter()
                .map(|scores| {
                    scores[start..end]
                        .iter()
                        .copied()
                        .filter(|score| score.is_finite())
                        .fold(0.0_f32, f32::max) as f64
                })
                .sum::<f64>()
                * (1.0 + 1e-5);
        }
        let mut order = blocks.into_values().collect::<Vec<_>>();
        order.sort_unstable_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.total_cmp(&left.1))
        });

        // Block-priority: score blocks until none can beat the 21st group.
        let mut groups = vec![None::<Rank>; engine.inner.videos.len()];
        let mut scored_documents = 0_usize;
        let mut scored_blocks = 0_usize;
        let mut threshold: Option<Rank> = None;
        for (block_index, (tier, bound, members)) in order.iter().enumerate() {
            if let Some(threshold) = threshold
                && (*tier < threshold.tier()
                    || (*tier == threshold.tier() && *bound < threshold.score as f64))
            {
                break;
            }
            scored_blocks += 1;
            for &index in members {
                let (_, video, rank) = matched[index];
                scored_documents += 1;
                let best = &mut groups[video as usize];
                if best.is_none_or(|best| rank.better(best)) {
                    *best = Some(rank);
                }
            }
            if block_index % 16 == 15 {
                let mut bests = groups.iter().flatten().copied().collect::<Vec<_>>();
                if bests.len() >= TOP_GROUPS {
                    bests.select_nth_unstable_by(TOP_GROUPS - 1, |left, right| {
                        right
                            .tier()
                            .cmp(&left.tier())
                            .then_with(|| right.score.total_cmp(&left.score))
                    });
                    threshold = Some(bests[TOP_GROUPS - 1]);
                }
            }
        }
        // Finalize the chosen groups from all of their documents.
        let chosen = ranked_videos(&engine, &groups, TOP_GROUPS);
        let mut final_groups = vec![None::<Rank>; engine.inner.videos.len()];
        for &(_, video, rank) in &matched {
            if chosen.contains(&video) {
                let best = &mut final_groups[video as usize];
                if best.is_none_or(|best| rank.better(best)) {
                    *best = Some(rank);
                }
            }
        }
        let pruned = ranked_videos(&engine, &final_groups, TOP_GROUPS);
        println!(
            "{{\"query\":{:?},\"matched_documents\":{},\"blocks\":{},\"scored_blocks\":{scored_blocks},\"scored_documents\":{scored_documents},\"scored_fraction\":{:.4},\"top21_equal\":{}}}",
            query,
            matched.len(),
            order.len(),
            scored_documents as f64 / matched.len().max(1) as f64,
            pruned == expected
        );
        assert_eq!(pruned, expected, "pruned ranking differs for {query:?}");
    }
}

fn varint_len(value: u32) -> usize {
    match value {
        0..0x80 => 1,
        0x80..0x4000 => 2,
        0x4000..0x20_0000 => 3,
        0x20_0000..0x1000_0000 => 4,
        _ => 5,
    }
}

fn bit_width(value: u32) -> usize {
    (u32::BITS - value.leading_zeros()) as usize
}

/// Bytes of one posting list in the proposed schema-9 layout: delta varints
/// up to one block; otherwise 128-document blocks of bit-packed deltas and
/// (frequency - 1), each with an 8-byte header (first ID, two widths,
/// count), an 8-byte skip entry (last ID, offset), and a 4-byte block-max
/// score bound.
fn proposed_bytes(postings: &[(u32, u32)]) -> (usize, usize, usize) {
    const BLOCK: usize = BLOCK_DOCUMENTS as usize;
    if postings.len() <= BLOCK {
        let mut previous = 0;
        let bytes = postings
            .iter()
            .map(|&(document, frequency)| {
                let delta = document - previous;
                previous = document;
                varint_len(delta) + varint_len(frequency)
            })
            .sum();
        return (bytes, 0, 0);
    }
    let (mut data, mut skips, mut bounds) = (0, 0, 0);
    for block in postings.chunks(BLOCK) {
        let deltas = block.windows(2).map(|pair| pair[1].0 - pair[0].0);
        let delta_width = deltas.max().map_or(0, bit_width);
        let frequency_width = block
            .iter()
            .map(|&(_, frequency)| frequency.saturating_sub(1))
            .max()
            .map_or(0, bit_width);
        data += 8 + ((block.len() - 1) * delta_width + block.len() * frequency_width).div_ceil(8);
        skips += 8;
        bounds += 4;
    }
    (data, skips, bounds)
}

#[test]
#[ignore = "diagnostic: needs RAW_TOTALS_INDEX pointing at a real generation"]
fn measure_proposed_schema9_posting_bytes() {
    let index = std::env::var("RAW_TOTALS_INDEX").expect("set RAW_TOTALS_INDEX");
    let engine = RawSearchEngine::open(Path::new(&index)).expect("open generation");
    let documents = engine.inner.manifest.documents as usize;
    let videos = engine.inner.videos.len();
    let mut totals = [[0_usize; 4]; 2];
    let mut text_frequencies = Vec::new();
    let mut title_frequencies = Vec::new();
    for position in 0..engine.inner.lexicon.count {
        let (_, entry) = engine.inner.lexicon.entry_at(position).expect("entry");
        for (field, offset, length, frequency) in [
            (
                0,
                entry.text_offset,
                entry.text_length,
                entry.text_frequency,
            ),
            (
                1,
                entry.title_offset,
                entry.title_length,
                entry.title_frequency,
            ),
        ] {
            if frequency == 0 {
                continue;
            }
            let bytes = engine.postings_range(offset, length).expect("postings");
            let (mut cursor, mut id) = (0, 0_u32);
            let mut postings = Vec::with_capacity(frequency as usize);
            while cursor < bytes.len() {
                id += read_varint(bytes, &mut cursor, bytes.len()).expect("delta");
                let term_frequency = read_varint(bytes, &mut cursor, bytes.len()).expect("tf");
                postings.push((id, term_frequency));
            }
            let (data, skips, bounds) = proposed_bytes(&postings);
            totals[field][0] += length;
            totals[field][1] += data;
            totals[field][2] += skips;
            totals[field][3] += bounds;
            if field == 0 {
                text_frequencies.push(frequency);
            } else {
                title_frequencies.push(frequency);
            }
        }
    }
    text_frequencies.sort_unstable_by(|left, right| right.cmp(left));
    title_frequencies.sort_unstable_by(|left, right| right.cmp(left));
    let text_bitmap = documents.div_ceil(8);
    let budget = 64 * 1024 * 1024;
    let text_bitmaps = (budget / text_bitmap).min(text_frequencies.len());
    let title_bitmap = videos.div_ceil(8);
    let title_bitmaps = title_frequencies
        .iter()
        .take_while(|&&frequency| frequency as usize * 1000 >= videos)
        .count();
    for (name, field, bitmaps, bitmap_bytes, threshold) in [
        (
            "text",
            0,
            text_bitmaps,
            text_bitmap,
            text_frequencies.get(text_bitmaps.saturating_sub(1)),
        ),
        (
            "title",
            1,
            title_bitmaps,
            title_bitmap,
            title_frequencies.get(title_bitmaps.saturating_sub(1)),
        ),
    ] {
        let [schema6, data, skips, bounds] = totals[field];
        println!(
            "{{\"field\":{name:?},\"schema6_posting_bytes\":{schema6},\"schema9_block_bytes\":{data},\"skip_bytes\":{skips},\"blockmax_bytes\":{bounds},\"schema9_total_without_bitmaps\":{},\"bitmaps\":{bitmaps},\"bitmap_bytes\":{},\"bitmap_min_df\":{}}}",
            data + skips + bounds,
            bitmaps * bitmap_bytes,
            threshold.copied().unwrap_or(0)
        );
    }
}
