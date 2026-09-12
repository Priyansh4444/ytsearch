//! Throwaway, corpus-shaped measurements for lazy-exact schema decisions.
//!
//! This example deliberately does not read or write a Raw index. It compares
//! structural representations over one deterministic synthetic corpus, emits
//! JSON, and can be deleted once the schema-9 choices are made.

use serde::Serialize;
use std::{env, hint::black_box, process::ExitCode, time::Instant};

const DEFAULT_DOCUMENTS: u32 = 1_000_000;
const DEFAULT_ROUNDS: usize = 9;
const DOCUMENTS_PER_VIDEO: u32 = 240;
const MOMENT_WINDOW_MS: u32 = 20_000;
const PARTS_PER_MILLION: u64 = 1_000_000;
const DF_BANDS_PPM: [u32; 7] = [0, 1_000, 10_000, 100_000, 500_000, 900_000, 1_000_000];

#[derive(Clone, Copy)]
struct Config {
    documents: u32,
    rounds: usize,
}

#[derive(Serialize)]
struct Report {
    format_version: u8,
    outcome: &'static str,
    throwaway_schema9_prototype: bool,
    prototype_base_commit: &'static str,
    prototype: &'static str,
    corpus: CorpusReport,
    rounds: usize,
    exact_set_count: Vec<SetCountReport>,
    exact_moment_aggregation: Vec<MomentReport>,
    phrase_candidates: Vec<PhraseReport>,
}

#[derive(Serialize)]
struct CorpusReport {
    documents: u32,
    videos: u32,
    documents_per_video: u32,
    moment_window_ms: u32,
    deterministic_seed: &'static str,
}

#[derive(Serialize)]
struct SetCountReport {
    timing_eligible: bool,
    exclusion_reason: Option<&'static str>,
    target_df_ppm: u32,
    left_documents: usize,
    right_documents: usize,
    exact_intersection: u64,
    bitmap_bytes: usize,
    sparse_bytes: usize,
    bitmap_construction: Timing,
    sparse_construction: Timing,
    bitmap_query_scratch_bytes: usize,
    sparse_query_scratch_bytes: usize,
    bitmap_and_popcount: Timing,
    sparse_skip_leapfrog: Timing,
}

#[derive(Serialize)]
struct MomentReport {
    timing_eligible: bool,
    exclusion_reason: Option<&'static str>,
    target_df_ppm: u32,
    matching_documents: usize,
    matching_videos: u64,
    exact_moments: u64,
    bitmap_bytes: usize,
    sparse_bytes: usize,
    dense_set_bit_scan: Timing,
    sparse_posting_scan: Timing,
}

#[derive(Serialize)]
struct PhraseReport {
    timing_eligible: bool,
    exclusion_reason: Option<&'static str>,
    target_df_ppm: u32,
    exact_candidates: u64,
    exact_phrase_matches: u64,
    bitmap_intersect_then_positions: Timing,
    skip_intersect_then_positions: Timing,
}

#[derive(Serialize)]
struct Timing {
    scope: &'static str,
    warmup_runs: usize,
    samples_ns: Vec<u128>,
    min_ns: u128,
    median_ns: u128,
    max_ns: u128,
}

struct Corpus {
    document_video: Vec<VideoId>,
    start_ms: Vec<u32>,
    left_position: Vec<u16>,
    right_position: Vec<u16>,
}

struct ExactSet {
    sparse: Vec<DocId>,
    bitmap: DenseBitmap,
}

struct DenseBitmap {
    words: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
struct DocId(u32);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
struct VideoId(u32);

fn main() -> ExitCode {
    match parse_config().and_then(run) {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed to encode measurement report: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            let failure = serde_json::json!({
                "format_version": 1,
                "outcome": "failed",
                "throwaway_schema9_prototype": true,
                "reason": error,
            });
            println!("{failure}");
            ExitCode::FAILURE
        }
    }
}

fn parse_config() -> Result<Config, String> {
    let mut documents = DEFAULT_DOCUMENTS;
    let mut rounds = DEFAULT_ROUNDS;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--documents" => documents = parse_value(&mut args, "--documents")?,
            "--rounds" => rounds = parse_value(&mut args, "--rounds")?,
            "--help" | "-h" => {
                return Err(
                    "usage: lazy_exact_measure [--documents <u32>] [--rounds <usize>]".to_string(),
                );
            }
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }
    if documents == 0 || rounds == 0 {
        return Err("documents and rounds must both be positive".to_string());
    }
    Ok(Config { documents, rounds })
}

fn parse_value<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<T, String> {
    args.next()
        .ok_or_else(|| format!("missing value for {name}"))?
        .parse()
        .map_err(|_| format!("invalid value for {name}"))
}

fn run(config: Config) -> Result<Report, String> {
    let corpus = Corpus::new(config.documents);
    let videos = config.documents.div_ceil(DOCUMENTS_PER_VIDEO);
    let mut exact_set_count = Vec::with_capacity(DF_BANDS_PPM.len());
    let mut exact_moment_aggregation = Vec::with_capacity(DF_BANDS_PPM.len());
    let mut phrase_candidates = Vec::with_capacity(DF_BANDS_PPM.len());

    for target_df_ppm in DF_BANDS_PPM {
        let left = ExactSet::new(config.documents, target_df_ppm, 0x93a7_6f1d_45bc_2191);
        let right = ExactSet::new(config.documents, target_df_ppm, 0x17e3_81ba_a62c_f405);

        exact_set_count.push(measure_set_count(
            target_df_ppm,
            &left,
            &right,
            config.rounds,
        )?);
        exact_moment_aggregation.push(measure_moments(
            target_df_ppm,
            &left,
            &corpus,
            config.rounds,
        )?);
        phrase_candidates.push(measure_phrase(
            target_df_ppm,
            &left,
            &right,
            &corpus,
            config.rounds,
        )?);
    }

    Ok(Report {
        format_version: 1,
        outcome: "ok",
        throwaway_schema9_prototype: true,
        prototype_base_commit: "1b1092e262d9fcd0586f6236c0e945da431095b4",
        prototype: "lazy_exact_structural_forks_v1",
        corpus: CorpusReport {
            documents: config.documents,
            videos,
            documents_per_video: DOCUMENTS_PER_VIDEO,
            moment_window_ms: MOMENT_WINDOW_MS,
            deterministic_seed: "splitmix64-v1",
        },
        rounds: config.rounds,
        exact_set_count,
        exact_moment_aggregation,
        phrase_candidates,
    })
}

fn measure_set_count(
    target_df_ppm: u32,
    left: &ExactSet,
    right: &ExactSet,
    rounds: usize,
) -> Result<SetCountReport, String> {
    let bitmap_answer = bitmap_intersection_count(&left.bitmap, &right.bitmap);
    let skip_answer = skip_intersection_count(&left.sparse, &right.sparse);
    require_equal("set intersection", bitmap_answer, skip_answer)?;
    require_equal(
        "set intersection exhaustive oracle",
        bitmap_answer,
        reference_intersection_count(&left.sparse, &right.sparse),
    )?;

    Ok(SetCountReport {
        timing_eligible: !left.sparse.is_empty() && !right.sparse.is_empty(),
        exclusion_reason: (left.sparse.is_empty() || right.sparse.is_empty())
            .then_some("empty_input_negative_control"),
        target_df_ppm,
        left_documents: left.sparse.len(),
        right_documents: right.sparse.len(),
        exact_intersection: bitmap_answer,
        bitmap_bytes: left.bitmap.byte_len() + right.bitmap.byte_len(),
        sparse_bytes: (left.sparse.len() + right.sparse.len()) * size_of::<DocId>(),
        bitmap_construction: measure(rounds, || {
            DenseBitmap::from_documents(black_box(&left.sparse), left.bitmap.words.len()).checksum()
        }),
        sparse_construction: measure(rounds, || black_box(left.sparse.clone()).len() as u64),
        bitmap_query_scratch_bytes: 0,
        sparse_query_scratch_bytes: 0,
        bitmap_and_popcount: measure(rounds, || {
            bitmap_intersection_count(black_box(&left.bitmap), black_box(&right.bitmap))
        }),
        sparse_skip_leapfrog: measure(rounds, || {
            skip_intersection_count(black_box(&left.sparse), black_box(&right.sparse))
        }),
    })
}

fn measure_moments(
    target_df_ppm: u32,
    set: &ExactSet,
    corpus: &Corpus,
    rounds: usize,
) -> Result<MomentReport, String> {
    let dense_answer = moments_from_bitmap(&set.bitmap, corpus);
    let sparse_answer = moments_from_sparse(&set.sparse, corpus);
    require_equal("20-second moment aggregation", dense_answer, sparse_answer)?;
    require_equal(
        "20-second moment exhaustive oracle",
        dense_answer,
        reference_moments(&set.sparse, corpus),
    )?;

    Ok(MomentReport {
        timing_eligible: !set.sparse.is_empty(),
        exclusion_reason: set
            .sparse
            .is_empty()
            .then_some("empty_input_negative_control"),
        target_df_ppm,
        matching_documents: set.sparse.len(),
        matching_videos: dense_answer >> 32,
        exact_moments: dense_answer & u64::from(u32::MAX),
        bitmap_bytes: set.bitmap.byte_len(),
        sparse_bytes: set.sparse.len() * size_of::<DocId>(),
        dense_set_bit_scan: measure(rounds, || {
            moments_from_bitmap(black_box(&set.bitmap), black_box(corpus))
        }),
        sparse_posting_scan: measure(rounds, || {
            moments_from_sparse(black_box(&set.sparse), black_box(corpus))
        }),
    })
}

fn measure_phrase(
    target_df_ppm: u32,
    left: &ExactSet,
    right: &ExactSet,
    corpus: &Corpus,
    rounds: usize,
) -> Result<PhraseReport, String> {
    let bitmap_answer = phrase_from_bitmap(&left.bitmap, &right.bitmap, corpus);
    let skip_answer = phrase_from_skip(&left.sparse, &right.sparse, corpus);
    require_equal("phrase candidate verification", bitmap_answer, skip_answer)?;
    require_equal(
        "phrase exhaustive oracle",
        bitmap_answer,
        reference_phrase(left, right, corpus),
    )?;

    Ok(PhraseReport {
        timing_eligible: bitmap_answer >> 32 != 0,
        exclusion_reason: (bitmap_answer >> 32 == 0).then_some("no_phrase_candidates"),
        target_df_ppm,
        exact_candidates: bitmap_answer >> 32,
        exact_phrase_matches: bitmap_answer & u64::from(u32::MAX),
        bitmap_intersect_then_positions: measure(rounds, || {
            phrase_from_bitmap(
                black_box(&left.bitmap),
                black_box(&right.bitmap),
                black_box(corpus),
            )
        }),
        skip_intersect_then_positions: measure(rounds, || {
            phrase_from_skip(
                black_box(&left.sparse),
                black_box(&right.sparse),
                black_box(corpus),
            )
        }),
    })
}

fn require_equal(label: &str, left: u64, right: u64) -> Result<(), String> {
    if left == right {
        Ok(())
    } else {
        Err(format!("{label} parity failed: {left} != {right}"))
    }
}

fn measure(mut rounds: usize, mut operation: impl FnMut() -> u64) -> Timing {
    black_box(operation());
    let mut elapsed = Vec::with_capacity(rounds);
    while rounds > 0 {
        let started = Instant::now();
        black_box(operation());
        elapsed.push(started.elapsed().as_nanos());
        rounds -= 1;
    }
    elapsed.sort_unstable();
    Timing {
        scope: "steady_state_in_process_os_cache_warm",
        warmup_runs: 1,
        samples_ns: elapsed.clone(),
        min_ns: elapsed[0],
        median_ns: elapsed[elapsed.len() / 2],
        max_ns: elapsed[elapsed.len() - 1],
    }
}

impl Corpus {
    fn new(documents: u32) -> Self {
        let mut document_video = Vec::with_capacity(documents as usize);
        let mut start_ms = Vec::with_capacity(documents as usize);
        let mut left_position = Vec::with_capacity(documents as usize);
        let mut right_position = Vec::with_capacity(documents as usize);
        for document in 0..documents {
            let within_video = document % DOCUMENTS_PER_VIDEO;
            let hash = splitmix64(u64::from(document) ^ 0x6c8e_9cf5_7093_2bd5);
            document_video.push(VideoId(document / DOCUMENTS_PER_VIDEO));
            start_ms.push(
                within_video
                    .saturating_mul(10_000)
                    .saturating_add(hash as u32 % 3_000),
            );
            let left = (hash % 180) as u16;
            let adjacent = hash.rotate_left(17).is_multiple_of(3);
            left_position.push(left);
            right_position.push(left + if adjacent { 1 } else { 2 + (hash % 5) as u16 });
        }
        Self {
            document_video,
            start_ms,
            left_position,
            right_position,
        }
    }
}

impl ExactSet {
    fn new(documents: u32, df_ppm: u32, seed: u64) -> Self {
        let mut sparse = Vec::with_capacity(
            (u64::from(documents) * u64::from(df_ppm) / PARTS_PER_MILLION) as usize,
        );
        let mut bitmap = DenseBitmap::new(documents);
        for document in 0..documents {
            let selected =
                splitmix64(u64::from(document) ^ seed) % PARTS_PER_MILLION < u64::from(df_ppm);
            if selected {
                let document = DocId(document);
                sparse.push(document);
                bitmap.insert(document);
            }
        }
        Self { sparse, bitmap }
    }
}

impl DenseBitmap {
    fn new(documents: u32) -> Self {
        Self {
            words: vec![0; (documents as usize).div_ceil(u64::BITS as usize)],
        }
    }

    fn insert(&mut self, document: DocId) {
        let document = document.0 as usize;
        self.words[document / u64::BITS as usize] |= 1 << (document % u64::BITS as usize);
    }

    fn from_documents(documents: &[DocId], word_count: usize) -> Self {
        let mut bitmap = Self {
            words: vec![0; word_count],
        };
        for &document in documents {
            bitmap.insert(document);
        }
        bitmap
    }

    fn checksum(&self) -> u64 {
        self.words.iter().fold(0, |checksum, word| checksum ^ word)
    }

    fn byte_len(&self) -> usize {
        self.words.len() * size_of::<u64>()
    }
}

fn bitmap_intersection_count(left: &DenseBitmap, right: &DenseBitmap) -> u64 {
    left.words
        .iter()
        .zip(&right.words)
        .map(|(left, right)| u64::from((left & right).count_ones()))
        .sum()
}

fn skip_intersection_count(left: &[DocId], right: &[DocId]) -> u64 {
    let mut count = 0_u64;
    visit_skip_intersection(left, right, |_| count += 1);
    count
}

fn reference_intersection_count(left: &[DocId], right: &[DocId]) -> u64 {
    left.iter()
        .filter(|document| right.binary_search(document).is_ok())
        .count() as u64
}

fn visit_skip_intersection(left: &[DocId], right: &[DocId], mut visit: impl FnMut(DocId)) {
    let (mut left_index, mut right_index) = (0, 0);
    while left_index < left.len() && right_index < right.len() {
        let left_document = left[left_index];
        let right_document = right[right_index];
        if left_document == right_document {
            visit(left_document);
            left_index += 1;
            right_index += 1;
        } else if left_document < right_document {
            left_index = gallop_to(left, left_index + 1, right_document);
        } else {
            right_index = gallop_to(right, right_index + 1, left_document);
        }
    }
}

fn gallop_to(values: &[DocId], start: usize, target: DocId) -> usize {
    if start >= values.len() || values[start] >= target {
        return start;
    }
    let mut width = 1_usize;
    while start + width < values.len() && values[start + width] < target {
        width = width.saturating_mul(2);
    }
    let low = start + width / 2 + 1;
    let high = (start + width + 1).min(values.len());
    low + values[low..high].partition_point(|value| *value < target)
}

fn moments_from_bitmap(set: &DenseBitmap, corpus: &Corpus) -> u64 {
    let mut counter = MomentCounter::default();
    for (word_index, word) in set.words.iter().copied().enumerate() {
        let mut remaining = word;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let document = word_index * u64::BITS as usize + bit;
            counter.record(corpus.document_video[document], corpus.start_ms[document]);
            remaining &= remaining - 1;
        }
    }
    counter.packed()
}

fn moments_from_sparse(set: &[DocId], corpus: &Corpus) -> u64 {
    let mut counter = MomentCounter::default();
    for &document in set {
        let index = document.0 as usize;
        counter.record(corpus.document_video[index], corpus.start_ms[index]);
    }
    counter.packed()
}

fn reference_moments(set: &[DocId], corpus: &Corpus) -> u64 {
    let mut by_video = std::collections::BTreeMap::<VideoId, Vec<u32>>::new();
    for &document in set {
        let index = document.0 as usize;
        by_video
            .entry(corpus.document_video[index])
            .or_default()
            .push(corpus.start_ms[index]);
    }
    let mut moments = 0_u32;
    for timestamps in by_video.values() {
        let mut cluster_start = None;
        for &timestamp in timestamps {
            if cluster_start.is_none_or(|start| timestamp.saturating_sub(start) > MOMENT_WINDOW_MS)
            {
                moments += 1;
                cluster_start = Some(timestamp);
            }
        }
    }
    (u64::try_from(by_video.len()).unwrap() << 32) | u64::from(moments)
}

#[derive(Default)]
struct MomentCounter {
    current_video: Option<VideoId>,
    cluster_start_ms: u32,
    videos: u32,
    moments: u32,
}

impl MomentCounter {
    fn record(&mut self, video: VideoId, start_ms: u32) {
        if self.current_video != Some(video) {
            self.current_video = Some(video);
            self.cluster_start_ms = start_ms;
            self.videos += 1;
            self.moments += 1;
            return;
        }
        if start_ms.saturating_sub(self.cluster_start_ms) > MOMENT_WINDOW_MS {
            self.cluster_start_ms = start_ms;
            self.moments += 1;
        }
    }

    fn packed(&self) -> u64 {
        (u64::from(self.videos) << 32) | u64::from(self.moments)
    }
}

fn phrase_from_bitmap(left: &DenseBitmap, right: &DenseBitmap, corpus: &Corpus) -> u64 {
    let mut candidates = 0_u32;
    let mut matches = 0_u32;
    for (word_index, (left, right)) in left.words.iter().zip(&right.words).enumerate() {
        let mut remaining = left & right;
        while remaining != 0 {
            let bit = remaining.trailing_zeros() as usize;
            let document = word_index * u64::BITS as usize + bit;
            candidates += 1;
            matches +=
                u32::from(corpus.right_position[document] == corpus.left_position[document] + 1);
            remaining &= remaining - 1;
        }
    }
    (u64::from(candidates) << 32) | u64::from(matches)
}

fn phrase_from_skip(left: &[DocId], right: &[DocId], corpus: &Corpus) -> u64 {
    let mut candidates = 0_u32;
    let mut matches = 0_u32;
    visit_skip_intersection(left, right, |document| {
        let document = document.0 as usize;
        candidates += 1;
        matches += u32::from(corpus.right_position[document] == corpus.left_position[document] + 1);
    });
    (u64::from(candidates) << 32) | u64::from(matches)
}

fn reference_phrase(left: &ExactSet, right: &ExactSet, corpus: &Corpus) -> u64 {
    let mut candidates = 0_u32;
    let mut matches = 0_u32;
    for &document in &left.sparse {
        if right.sparse.binary_search(&document).is_err() {
            continue;
        }
        candidates += 1;
        let index = document.0 as usize;
        matches += u32::from(corpus.right_position[index] == corpus.left_position[index] + 1);
    }
    (u64::from(candidates) << 32) | u64::from(matches)
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_set_counts_match_at_every_df_band() {
        for df_ppm in DF_BANDS_PPM {
            let left = ExactSet::new(20_003, df_ppm, 11);
            let right = ExactSet::new(20_003, df_ppm, 29);
            assert_eq!(
                bitmap_intersection_count(&left.bitmap, &right.bitmap),
                skip_intersection_count(&left.sparse, &right.sparse)
            );
        }
    }

    #[test]
    fn exact_moment_aggregations_match_at_every_df_band() {
        let corpus = Corpus::new(20_003);
        for df_ppm in DF_BANDS_PPM {
            let set = ExactSet::new(20_003, df_ppm, 41);
            assert_eq!(
                moments_from_bitmap(&set.bitmap, &corpus),
                moments_from_sparse(&set.sparse, &corpus)
            );
        }
    }

    #[test]
    fn moment_window_is_anchored_and_non_transitive() {
        let mut counter = MomentCounter::default();
        for start_ms in [0, 19_000, 38_000] {
            counter.record(VideoId(0), start_ms);
        }
        assert_eq!(counter.videos, 1);
        assert_eq!(counter.moments, 2);
    }

    #[test]
    fn moment_window_preserves_exact_boundaries_and_video_resets() {
        let cases = [
            (&[1_000, 21_000][..], 1),
            (&[1_000, 21_001][..], 2),
            (&[1_000, 21_000, 21_001, 41_001][..], 2),
        ];
        for (timestamps, expected) in cases {
            let mut counter = MomentCounter::default();
            for &timestamp in timestamps {
                counter.record(VideoId(0), timestamp);
            }
            assert_eq!(counter.moments, expected);
        }

        let mut counter = MomentCounter::default();
        counter.record(VideoId(0), 1_000);
        counter.record(VideoId(1), 1_000);
        assert_eq!((counter.videos, counter.moments), (2, 2));
    }

    #[test]
    fn generated_corpus_is_ordered_by_video_timestamp_and_document() {
        let corpus = Corpus::new(20_003);
        for document in 1..corpus.document_video.len() {
            let previous = (
                corpus.document_video[document - 1],
                corpus.start_ms[document - 1],
                document - 1,
            );
            let current = (
                corpus.document_video[document],
                corpus.start_ms[document],
                document,
            );
            assert!(previous < current);
        }
    }

    #[test]
    fn phrase_candidates_and_position_results_match() {
        let corpus = Corpus::new(20_003);
        for df_ppm in DF_BANDS_PPM {
            let left = ExactSet::new(20_003, df_ppm, 43);
            let right = ExactSet::new(20_003, df_ppm, 47);
            assert_eq!(
                phrase_from_bitmap(&left.bitmap, &right.bitmap, &corpus),
                phrase_from_skip(&left.sparse, &right.sparse, &corpus)
            );
        }
    }

    #[test]
    fn oracle_rejects_wrong_count_and_unverified_phrase_candidates() {
        assert!(require_equal("wrong count", 4, 5).is_err());

        let corpus = Corpus::new(20_003);
        let left = ExactSet::new(20_003, 500_000, 43);
        let right = ExactSet::new(20_003, 500_000, 47);
        let phrase = reference_phrase(&left, &right, &corpus);
        let candidates = phrase >> 32;
        let matches = phrase & u64::from(u32::MAX);
        assert_ne!(candidates, matches);
    }

    #[test]
    fn gallop_reaches_first_value_at_or_after_target() {
        let values = [1, 3, 8, 13, 21, 34, 55].map(DocId);
        for target in 0..60 {
            assert_eq!(
                gallop_to(&values, 0, DocId(target)),
                values.partition_point(|value| *value < DocId(target))
            );
        }
    }
}
