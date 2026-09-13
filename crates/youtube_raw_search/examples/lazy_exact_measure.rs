//! Throwaway, corpus-shaped measurements for lazy-exact schema decisions.
//!
//! This example deliberately does not read or write a Raw index. It compares
//! structural representations over one deterministic synthetic corpus, emits
//! JSON, and can be deleted once the schema-9 choices are made.

use serde::Serialize;
use std::{env, fs, hint::black_box, process::ExitCode, time::Instant};

const DEFAULT_DOCUMENTS: u32 = 1_000_000;
const DEFAULT_ROUNDS: usize = 9;
const DOCUMENTS_PER_VIDEO: u32 = 240;
const MOMENT_WINDOW_MS: u32 = 20_000;
const PARTS_PER_MILLION: u64 = 1_000_000;
const REPORT_SCHEMA_VERSION: u8 = 3;
const HARNESS_VERSION: &str = "lazy-exact-structural-forks-v3";
const GENERATOR_VERSION: &str = "splitmix64-corpus-v2";
const DEFAULT_SEED: u64 = 0x6c8e_9cf5_7093_2bd5;
const DF_BANDS_PPM: [u32; 7] = [0, 1_000, 10_000, 100_000, 500_000, 900_000, 1_000_000];

#[derive(Clone, Copy)]
struct Config {
    documents: u32,
    rounds: usize,
    repetition: u32,
    inject_failure: Option<FailureInjection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailureInjection {
    Oracle,
    Measurement,
}

#[derive(Serialize)]
struct Report {
    schema_version: u8,
    outcome: RunOutcome,
    throwaway_schema9_prototype: bool,
    run_identity: RunIdentity,
    counters: OutcomeCounters,
    resources: ResourceReport,
    corpus: CorpusReport,
    rounds: usize,
    exact_set_count: Vec<SetCountReport>,
    exact_moment_aggregation: Vec<MomentReport>,
    phrase_candidates: Vec<PhraseReport>,
    semantic_contracts: Vec<SemanticContractReport>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RunOutcome {
    Complete,
    CompleteWithExclusions,
    CompleteWithFailures,
}

#[derive(Serialize)]
struct RunIdentity {
    report_schema_version: u8,
    harness_version: &'static str,
    prototype_base_commit: &'static str,
    generator_version: &'static str,
    seed: u64,
    documents: u32,
    rounds: usize,
    repetition: u32,
    df_bands_ppm: [u32; DF_BANDS_PPM.len()],
    source_commit: String,
    source_dirty: bool,
    rustc_version: String,
    rustc_target: String,
    build_profile: String,
    environment: String,
    identity_digest: String,
}

#[derive(Serialize)]
struct OutcomeCounters {
    attempted: usize,
    eligible: usize,
    excluded: usize,
    failed: usize,
}

#[derive(Serialize)]
struct ResourceReport {
    baseline_peak_rss_bytes: Option<u64>,
    final_peak_rss_bytes: Option<u64>,
    peak_rss_increment_bytes: Option<u64>,
    rss_unavailable_reason: Option<&'static str>,
    proc_io_read_bytes_delta: Option<u64>,
    proc_io_write_bytes_delta: Option<u64>,
    proc_io_unavailable_reason: Option<&'static str>,
}

#[derive(Clone, Copy)]
struct ResourceSnapshot {
    peak_rss_bytes: Option<u64>,
    io: Option<(u64, u64)>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CaseOutcome {
    Eligible,
    Ineligible { reason: CaseExclusion },
    Failed { reason: CaseFailure },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CaseExclusion {
    EmptyInputNegativeControl,
    NoPhraseCandidates,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CaseFailure {
    OracleMismatch,
    MeasurementFailure,
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
    case_id: String,
    outcome: CaseOutcome,
    target_df_ppm: u32,
    left_documents: usize,
    right_documents: usize,
    exact_intersection: u64,
    bitmap_bytes: usize,
    sparse_bytes: usize,
    construction_source_bytes: usize,
    construction_measurement_order: &'static str,
    query_measurement_order: &'static str,
    bitmap_construction: Option<Timing>,
    sparse_construction: Option<Timing>,
    bitmap_query_scratch_bytes: usize,
    sparse_query_scratch_bytes: usize,
    bitmap_and_popcount: Option<Timing>,
    sparse_skip_leapfrog: Option<Timing>,
}

#[derive(Serialize)]
struct MomentReport {
    case_id: String,
    outcome: CaseOutcome,
    target_df_ppm: u32,
    matching_documents: usize,
    matching_videos: u64,
    exact_moments: u64,
    bitmap_bytes: usize,
    sparse_bytes: usize,
    title_matching_videos: usize,
    measurement_order: &'static str,
    dense_set_bit_scan: Option<Timing>,
    sparse_posting_scan: Option<Timing>,
}

#[derive(Serialize)]
struct PhraseReport {
    case_id: String,
    outcome: CaseOutcome,
    target_df_ppm: u32,
    exact_candidates: u64,
    exact_phrase_matches: u64,
    measurement_order: &'static str,
    bitmap_intersect_then_positions: Option<Timing>,
    skip_intersect_then_positions: Option<Timing>,
}

#[derive(Serialize)]
struct SemanticContractReport {
    case_id: &'static str,
    outcome: CaseOutcome,
    expected: u64,
    actual: u64,
    measurement: Option<Timing>,
}

#[derive(Serialize)]
struct Timing {
    scope: &'static str,
    warmup_runs: usize,
    samples_ns: Vec<u128>,
    min_ns: u128,
    median_ns: u128,
    p95_ns: u128,
    max_ns: u128,
    process_cpu_samples_ns: Option<Vec<u128>>,
    process_cpu_unavailable_reason: Option<&'static str>,
}

struct Corpus {
    document_video: Vec<VideoId>,
    start_ms: Vec<u32>,
    left_position: Vec<u16>,
    right_position: Vec<u16>,
}

struct DocSet {
    sparse: Vec<DocId>,
    bitmap: DenseBitmap,
}

struct VideoSet {
    bitmap: DenseBitmap,
}

struct MatchSet<'a> {
    transcript_documents: &'a DocSet,
    title_videos: &'a VideoSet,
}

struct FieldPositions {
    text: Vec<u16>,
    title: Vec<u16>,
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
                "schema_version": REPORT_SCHEMA_VERSION,
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
    let mut repetition = 0;
    let mut inject_failure = None;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--documents" => documents = parse_value(&mut args, "--documents")?,
            "--rounds" => rounds = parse_value(&mut args, "--rounds")?,
            "--repetition" => repetition = parse_value(&mut args, "--repetition")?,
            "--inject-failure" => {
                inject_failure = Some(match args.next().as_deref() {
                    Some("oracle") => FailureInjection::Oracle,
                    Some("measurement") => FailureInjection::Measurement,
                    _ => return Err("--inject-failure requires oracle or measurement".to_string()),
                });
            }
            "--help" | "-h" => {
                return Err(
                    "usage: lazy_exact_measure [--documents <u32>] [--rounds <usize>] [--repetition <u32>] [--inject-failure oracle|measurement]".to_string(),
                );
            }
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }
    if documents == 0 || rounds == 0 {
        return Err("documents and rounds must both be positive".to_string());
    }
    Ok(Config {
        documents,
        rounds,
        repetition,
        inject_failure,
    })
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
    let resources_before = resource_snapshot();
    let corpus = Corpus::new(config.documents);
    let videos = config.documents.div_ceil(DOCUMENTS_PER_VIDEO);
    let mut exact_set_count = Vec::with_capacity(DF_BANDS_PPM.len());
    let mut exact_moment_aggregation = Vec::with_capacity(DF_BANDS_PPM.len());
    let mut phrase_candidates = Vec::with_capacity(DF_BANDS_PPM.len());

    for target_df_ppm in DF_BANDS_PPM {
        let left = DocSet::new(config.documents, target_df_ppm, 0x93a7_6f1d_45bc_2191);
        let right = DocSet::new(config.documents, target_df_ppm, 0x17e3_81ba_a62c_f405);

        exact_set_count.push(measure_set_count(
            target_df_ppm,
            &left,
            &right,
            config.rounds,
            (target_df_ppm == 1_000)
                .then_some(config.inject_failure)
                .flatten(),
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

    let semantic_contracts = semantic_contracts(config.rounds);
    let outcomes = exact_set_count
        .iter()
        .map(|case| &case.outcome)
        .chain(exact_moment_aggregation.iter().map(|case| &case.outcome))
        .chain(phrase_candidates.iter().map(|case| &case.outcome))
        .chain(semantic_contracts.iter().map(|case| &case.outcome));
    let (mut has_exclusion, mut has_failure) = (false, false);
    for outcome in outcomes {
        has_exclusion |= matches!(outcome, CaseOutcome::Ineligible { .. });
        has_failure |= matches!(outcome, CaseOutcome::Failed { .. });
    }
    let outcome = if has_failure {
        RunOutcome::CompleteWithFailures
    } else if has_exclusion {
        RunOutcome::CompleteWithExclusions
    } else {
        RunOutcome::Complete
    };
    let counters = outcome_counters(
        exact_set_count
            .iter()
            .map(|case| &case.outcome)
            .chain(exact_moment_aggregation.iter().map(|case| &case.outcome))
            .chain(phrase_candidates.iter().map(|case| &case.outcome))
            .chain(semantic_contracts.iter().map(|case| &case.outcome)),
    );
    let resources = resource_report(resources_before, resource_snapshot());
    Ok(Report {
        schema_version: REPORT_SCHEMA_VERSION,
        outcome,
        throwaway_schema9_prototype: true,
        run_identity: run_identity(config),
        counters,
        resources,
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
        semantic_contracts,
    })
}

fn outcome_counters<'a>(outcomes: impl Iterator<Item = &'a CaseOutcome>) -> OutcomeCounters {
    let mut counters = OutcomeCounters {
        attempted: 0,
        eligible: 0,
        excluded: 0,
        failed: 0,
    };
    for outcome in outcomes {
        counters.attempted += 1;
        match outcome {
            CaseOutcome::Eligible => counters.eligible += 1,
            CaseOutcome::Ineligible { .. } => counters.excluded += 1,
            CaseOutcome::Failed { .. } => counters.failed += 1,
        }
    }
    counters
}

fn semantic_contracts(rounds: usize) -> Vec<SemanticContractReport> {
    let corpus = Corpus::new(DOCUMENTS_PER_VIDEO * 2);
    let mut title_videos = VideoSet::new(2);
    title_videos.insert(VideoId(0));
    title_videos.insert(VideoId(1));
    let mut documents = DocSet::new(DOCUMENTS_PER_VIDEO * 2, 0, 1);
    documents.sparse.push(DocId(0));
    documents.bitmap.insert(DocId(0));
    let matches = MatchSet {
        transcript_documents: &documents,
        title_videos: &title_videos,
    };
    let title_actual = moments_with_title_matches(matches, &corpus);
    let title_expected = reference_moments_with_titles(
        MatchSet {
            transcript_documents: &documents,
            title_videos: &title_videos,
        },
        &corpus,
    );
    let mut reports = vec![SemanticContractReport {
        case_id: "title-only-and-mixed-moments",
        outcome: parity_outcome(title_actual, title_actual, title_expected, || {
            CaseOutcome::Eligible
        }),
        expected: title_expected,
        actual: title_actual,
        measurement: Some(measure(rounds, || {
            moments_with_title_matches(
                MatchSet {
                    transcript_documents: black_box(&documents),
                    title_videos: black_box(&title_videos),
                },
                black_box(&corpus),
            )
        })),
    }];
    reports.push(measured_phrase_contract(
        "multiple-position-phrase",
        FieldPositions {
            text: vec![2, 8],
            title: vec![],
        },
        FieldPositions {
            text: vec![3, 12],
            title: vec![],
        },
        rounds,
    ));
    reports.push(measured_phrase_contract(
        "repeated-term-phrase",
        FieldPositions {
            text: vec![2, 3],
            title: vec![],
        },
        FieldPositions {
            text: vec![2, 3],
            title: vec![],
        },
        rounds,
    ));
    reports
}

fn measured_phrase_contract(
    case_id: &'static str,
    left: FieldPositions,
    right: FieldPositions,
    rounds: usize,
) -> SemanticContractReport {
    let actual = u64::from(phrase_matches_document(&left, &right));
    let expected = u64::from(reference_phrase_matches_document(&left, &right));
    SemanticContractReport {
        case_id,
        outcome: parity_outcome(actual, actual, expected, || CaseOutcome::Eligible),
        expected,
        actual,
        measurement: Some(measure(rounds, || {
            u64::from(phrase_matches_document(black_box(&left), black_box(&right)))
        })),
    }
}

fn run_identity(config: Config) -> RunIdentity {
    let source_commit = provenance("YTSEARCH_PROTOTYPE_GIT_COMMIT");
    let source_dirty = provenance("YTSEARCH_PROTOTYPE_GIT_DIRTY") == "true";
    let rustc_version = provenance("YTSEARCH_PROTOTYPE_RUSTC_VERSION");
    let rustc_target = provenance("YTSEARCH_PROTOTYPE_RUSTC_TARGET");
    let build_profile = provenance("YTSEARCH_PROTOTYPE_BUILD_PROFILE");
    let environment = provenance("YTSEARCH_PROTOTYPE_ENVIRONMENT");
    let identity = format!(
        "{REPORT_SCHEMA_VERSION}:{HARNESS_VERSION}:{GENERATOR_VERSION}:{DEFAULT_SEED}:{}:{}:{}:{DF_BANDS_PPM:?}:{source_commit}:{source_dirty}:{rustc_version}:{rustc_target}:{build_profile}:{environment}",
        config.documents, config.rounds, config.repetition
    );
    RunIdentity {
        report_schema_version: REPORT_SCHEMA_VERSION,
        harness_version: HARNESS_VERSION,
        prototype_base_commit: "1b1092e262d9fcd0586f6236c0e945da431095b4",
        generator_version: GENERATOR_VERSION,
        seed: DEFAULT_SEED,
        documents: config.documents,
        rounds: config.rounds,
        repetition: config.repetition,
        df_bands_ppm: DF_BANDS_PPM,
        source_commit,
        source_dirty,
        rustc_version,
        rustc_target,
        build_profile,
        environment,
        identity_digest: format!("blake3:{}", blake3::hash(identity.as_bytes()).to_hex()),
    }
}

fn provenance(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| "unwrapped-test-process".to_string())
}

fn measure_set_count(
    target_df_ppm: u32,
    left: &DocSet,
    right: &DocSet,
    rounds: usize,
    inject_failure: Option<FailureInjection>,
) -> Result<SetCountReport, String> {
    let bitmap_answer = bitmap_intersection_count(&left.bitmap, &right.bitmap);
    let skip_answer = skip_intersection_count(&left.sparse, &right.sparse)
        + u64::from(inject_failure == Some(FailureInjection::Oracle));
    let oracle_answer = reference_intersection_count(&left.sparse, &right.sparse);
    let outcome = parity_outcome(bitmap_answer, skip_answer, oracle_answer, || {
        if left.sparse.is_empty() || right.sparse.is_empty() {
            CaseOutcome::Ineligible {
                reason: CaseExclusion::EmptyInputNegativeControl,
            }
        } else {
            CaseOutcome::Eligible
        }
    });
    let left_first = counterbalance_left_first(target_df_ppm);
    let (outcome, bitmap_construction, sparse_construction) = measure_pair(
        outcome,
        if inject_failure == Some(FailureInjection::Measurement) {
            0
        } else {
            rounds
        },
        left_first,
        || {
            let left =
                DenseBitmap::from_documents(black_box(&left.sparse), left.bitmap.words.len());
            let right =
                DenseBitmap::from_documents(black_box(&right.sparse), right.bitmap.words.len());
            left.checksum() ^ right.checksum()
        },
        || {
            let left = black_box(left.sparse.clone());
            let right = black_box(right.sparse.clone());
            (left.len() + right.len()) as u64
        },
    );
    let (outcome, bitmap_and_popcount, sparse_skip_leapfrog) = measure_pair(
        outcome,
        rounds,
        !left_first,
        || bitmap_intersection_count(black_box(&left.bitmap), black_box(&right.bitmap)),
        || skip_intersection_count(black_box(&left.sparse), black_box(&right.sparse)),
    );

    Ok(SetCountReport {
        case_id: format!("set-count-df-{target_df_ppm}"),
        outcome,
        target_df_ppm,
        left_documents: left.sparse.len(),
        right_documents: right.sparse.len(),
        exact_intersection: bitmap_answer,
        bitmap_bytes: left.bitmap.byte_len() + right.bitmap.byte_len(),
        sparse_bytes: (left.sparse.len() + right.sparse.len()) * size_of::<DocId>(),
        construction_source_bytes: (left.sparse.len() + right.sparse.len()) * size_of::<DocId>(),
        construction_measurement_order: if left_first {
            "bitmap_then_sparse"
        } else {
            "sparse_then_bitmap"
        },
        query_measurement_order: if left_first {
            "sparse_then_bitmap"
        } else {
            "bitmap_then_sparse"
        },
        bitmap_construction,
        sparse_construction,
        bitmap_query_scratch_bytes: 0,
        sparse_query_scratch_bytes: 0,
        bitmap_and_popcount,
        sparse_skip_leapfrog,
    })
}

fn parity_outcome(
    left: u64,
    right: u64,
    oracle: u64,
    otherwise: impl FnOnce() -> CaseOutcome,
) -> CaseOutcome {
    if left != right || left != oracle {
        CaseOutcome::Failed {
            reason: CaseFailure::OracleMismatch,
        }
    } else {
        otherwise()
    }
}

fn measure_pair<L, R>(
    outcome: CaseOutcome,
    rounds: usize,
    left_first: bool,
    mut left: L,
    mut right: R,
) -> (CaseOutcome, Option<Timing>, Option<Timing>)
where
    L: FnMut() -> u64,
    R: FnMut() -> u64,
{
    if outcome != CaseOutcome::Eligible {
        return (outcome, None, None);
    }
    let (left, right) = if left_first {
        (
            measure_checked(rounds, &mut left),
            measure_checked(rounds, &mut right),
        )
    } else {
        let right_timing = measure_checked(rounds, &mut right);
        let left_timing = measure_checked(rounds, &mut left);
        (left_timing, right_timing)
    };
    match (left, right) {
        (Ok(left), Ok(right)) => (outcome, Some(left), Some(right)),
        _ => (
            CaseOutcome::Failed {
                reason: CaseFailure::MeasurementFailure,
            },
            None,
            None,
        ),
    }
}

fn counterbalance_left_first(target_df_ppm: u32) -> bool {
    DF_BANDS_PPM
        .iter()
        .position(|value| *value == target_df_ppm)
        .is_none_or(|index| index.is_multiple_of(2))
}

fn measure_checked(rounds: usize, operation: impl FnMut() -> u64) -> Result<Timing, CaseFailure> {
    if rounds == 0 {
        Err(CaseFailure::MeasurementFailure)
    } else {
        Ok(measure(rounds, operation))
    }
}

fn measure_moments(
    target_df_ppm: u32,
    set: &DocSet,
    corpus: &Corpus,
    rounds: usize,
) -> Result<MomentReport, String> {
    let dense_answer = moments_from_bitmap(&set.bitmap, corpus);
    let sparse_answer = moments_from_sparse(&set.sparse, corpus);
    let oracle_answer = reference_moments(&set.sparse, corpus);
    let outcome = parity_outcome(dense_answer, sparse_answer, oracle_answer, || {
        if set.sparse.is_empty() {
            CaseOutcome::Ineligible {
                reason: CaseExclusion::EmptyInputNegativeControl,
            }
        } else {
            CaseOutcome::Eligible
        }
    });
    let (outcome, dense_set_bit_scan, sparse_posting_scan) = measure_pair(
        outcome,
        rounds,
        counterbalance_left_first(target_df_ppm),
        || moments_from_bitmap(black_box(&set.bitmap), black_box(corpus)),
        || moments_from_sparse(black_box(&set.sparse), black_box(corpus)),
    );

    Ok(MomentReport {
        case_id: format!("moment-df-{target_df_ppm}"),
        outcome,
        target_df_ppm,
        matching_documents: set.sparse.len(),
        matching_videos: dense_answer >> 32,
        exact_moments: dense_answer & u64::from(u32::MAX),
        bitmap_bytes: set.bitmap.byte_len(),
        sparse_bytes: set.sparse.len() * size_of::<DocId>(),
        title_matching_videos: 0,
        measurement_order: if counterbalance_left_first(target_df_ppm) {
            "bitmap_then_sparse"
        } else {
            "sparse_then_bitmap"
        },
        dense_set_bit_scan,
        sparse_posting_scan,
    })
}

fn measure_phrase(
    target_df_ppm: u32,
    left: &DocSet,
    right: &DocSet,
    corpus: &Corpus,
    rounds: usize,
) -> Result<PhraseReport, String> {
    let bitmap_answer = phrase_from_bitmap(&left.bitmap, &right.bitmap, corpus);
    let skip_answer = phrase_from_skip(&left.sparse, &right.sparse, corpus);
    let oracle_answer = reference_phrase(left, right, corpus);
    let outcome = parity_outcome(bitmap_answer, skip_answer, oracle_answer, || {
        if bitmap_answer >> 32 == 0 {
            CaseOutcome::Ineligible {
                reason: CaseExclusion::NoPhraseCandidates,
            }
        } else {
            CaseOutcome::Eligible
        }
    });
    let (outcome, bitmap_intersect_then_positions, skip_intersect_then_positions) = measure_pair(
        outcome,
        rounds,
        counterbalance_left_first(target_df_ppm),
        || {
            phrase_from_bitmap(
                black_box(&left.bitmap),
                black_box(&right.bitmap),
                black_box(corpus),
            )
        },
        || {
            phrase_from_skip(
                black_box(&left.sparse),
                black_box(&right.sparse),
                black_box(corpus),
            )
        },
    );

    Ok(PhraseReport {
        case_id: format!("phrase-df-{target_df_ppm}"),
        outcome,
        target_df_ppm,
        exact_candidates: bitmap_answer >> 32,
        exact_phrase_matches: bitmap_answer & u64::from(u32::MAX),
        measurement_order: if counterbalance_left_first(target_df_ppm) {
            "bitmap_then_skip"
        } else {
            "skip_then_bitmap"
        },
        bitmap_intersect_then_positions,
        skip_intersect_then_positions,
    })
}

fn measure(mut rounds: usize, mut operation: impl FnMut() -> u64) -> Timing {
    black_box(operation());
    let mut elapsed = Vec::with_capacity(rounds);
    let mut cpu_elapsed = Vec::with_capacity(rounds);
    while rounds > 0 {
        let cpu_started = process_cpu_ns();
        let started = Instant::now();
        black_box(operation());
        elapsed.push(started.elapsed().as_nanos());
        if let (Some(started), Some(ended)) = (cpu_started, process_cpu_ns()) {
            cpu_elapsed.push(ended.saturating_sub(started));
        }
        rounds -= 1;
    }
    elapsed.sort_unstable();
    cpu_elapsed.sort_unstable();
    let p95_index = (elapsed.len() * 95).div_ceil(100).saturating_sub(1);
    let cpu_available = cpu_elapsed.len() == elapsed.len();
    Timing {
        scope: "steady_state_in_process_os_cache_warm",
        warmup_runs: 1,
        samples_ns: elapsed.clone(),
        min_ns: elapsed[0],
        median_ns: elapsed[elapsed.len() / 2],
        p95_ns: elapsed[p95_index],
        max_ns: elapsed[elapsed.len() - 1],
        process_cpu_samples_ns: cpu_available.then_some(cpu_elapsed),
        process_cpu_unavailable_reason: (!cpu_available).then_some("process_cpu_clock_unavailable"),
    }
}

fn process_cpu_ns() -> Option<u128> {
    let mut value = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: clock_gettime initializes the provided timespec when it returns zero.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, value.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: a zero return from clock_gettime guarantees initialization.
    let value = unsafe { value.assume_init() };
    Some((value.tv_sec as u128) * 1_000_000_000 + value.tv_nsec as u128)
}

fn resource_snapshot() -> ResourceSnapshot {
    ResourceSnapshot {
        peak_rss_bytes: peak_rss_bytes(),
        io: proc_io_bytes(),
    }
}

fn peak_rss_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the provided structure when it returns zero.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: a zero return from getrusage guarantees initialization.
    let usage = unsafe { usage.assume_init() };
    #[cfg(target_os = "macos")]
    return u64::try_from(usage.ru_maxrss).ok();
    #[cfg(not(target_os = "macos"))]
    u64::try_from(usage.ru_maxrss).ok()?.checked_mul(1024)
}

fn proc_io_bytes() -> Option<(u64, u64)> {
    let contents = fs::read_to_string("/proc/self/io").ok()?;
    let value = |name: &str| {
        contents.lines().find_map(|line| {
            line.strip_prefix(name)
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
    };
    Some((value("read_bytes:")?, value("write_bytes:")?))
}

fn resource_report(before: ResourceSnapshot, after: ResourceSnapshot) -> ResourceReport {
    let io_delta = before.io.zip(after.io).map(|(before, after)| {
        (
            after.0.saturating_sub(before.0),
            after.1.saturating_sub(before.1),
        )
    });
    ResourceReport {
        baseline_peak_rss_bytes: before.peak_rss_bytes,
        final_peak_rss_bytes: after.peak_rss_bytes,
        peak_rss_increment_bytes: before
            .peak_rss_bytes
            .zip(after.peak_rss_bytes)
            .map(|(before, after)| after.saturating_sub(before)),
        rss_unavailable_reason: (before.peak_rss_bytes.is_none() || after.peak_rss_bytes.is_none())
            .then_some("getrusage_unavailable"),
        proc_io_read_bytes_delta: io_delta.map(|value| value.0),
        proc_io_write_bytes_delta: io_delta.map(|value| value.1),
        proc_io_unavailable_reason: io_delta.is_none().then_some("proc_self_io_unavailable"),
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

impl DocSet {
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

impl VideoSet {
    fn new(videos: u32) -> Self {
        Self {
            bitmap: DenseBitmap::new(videos),
        }
    }

    fn insert(&mut self, video: VideoId) {
        self.bitmap.insert(DocId(video.0));
    }

    fn contains(&self, video: VideoId) -> bool {
        let index = video.0 as usize;
        self.bitmap
            .words
            .get(index / u64::BITS as usize)
            .is_some_and(|word| word & (1 << (index % u64::BITS as usize)) != 0)
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

fn moments_with_title_matches(matches: MatchSet<'_>, corpus: &Corpus) -> u64 {
    let transcript = moments_from_sparse(&matches.transcript_documents.sparse, corpus);
    let mut videos = (transcript >> 32) as u32;
    let mut moments = transcript as u32;
    let mut transcript_videos = VideoSet::new(
        corpus
            .document_video
            .len()
            .div_ceil(DOCUMENTS_PER_VIDEO as usize) as u32,
    );
    for &document in &matches.transcript_documents.sparse {
        transcript_videos.insert(corpus.document_video[document.0 as usize]);
    }
    for video in 0..matches.title_videos.bitmap.words.len() * u64::BITS as usize {
        let video = VideoId(video as u32);
        if matches.title_videos.contains(video) && !transcript_videos.contains(video) {
            videos += 1;
            moments += 1;
        }
    }
    (u64::from(videos) << 32) | u64::from(moments)
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

fn reference_moments_with_titles(matches: MatchSet<'_>, corpus: &Corpus) -> u64 {
    let mut timestamps = std::collections::BTreeMap::<VideoId, Vec<u32>>::new();
    for &document in &matches.transcript_documents.sparse {
        let index = document.0 as usize;
        timestamps
            .entry(corpus.document_video[index])
            .or_default()
            .push(corpus.start_ms[index]);
    }
    let mut result_videos = std::collections::BTreeSet::new();
    let mut moments = 0_u32;
    for (video, starts) in &timestamps {
        result_videos.insert(*video);
        let mut anchor = None;
        for &start in starts {
            if anchor.is_none_or(|value| start.saturating_sub(value) > MOMENT_WINDOW_MS) {
                moments += 1;
                anchor = Some(start);
            }
        }
    }
    for video in 0..matches.title_videos.bitmap.words.len() * u64::BITS as usize {
        let video = VideoId(video as u32);
        if matches.title_videos.contains(video) && result_videos.insert(video) {
            moments += 1;
        }
    }
    (u64::try_from(result_videos.len()).unwrap() << 32) | u64::from(moments)
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
            matches += u32::from(phrase_matches_positions(
                &[corpus.left_position[document]],
                &[corpus.right_position[document]],
            ));
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
        matches += u32::from(phrase_matches_positions(
            &[corpus.left_position[document]],
            &[corpus.right_position[document]],
        ));
    });
    (u64::from(candidates) << 32) | u64::from(matches)
}

fn reference_phrase(left: &DocSet, right: &DocSet, corpus: &Corpus) -> u64 {
    let mut candidates = 0_u32;
    let mut matches = 0_u32;
    for &document in &left.sparse {
        if right.sparse.binary_search(&document).is_err() {
            continue;
        }
        candidates += 1;
        let index = document.0 as usize;
        matches += u32::from(
            corpus.left_position[index]
                .checked_add(1)
                .is_some_and(|position| position == corpus.right_position[index]),
        );
    }
    (u64::from(candidates) << 32) | u64::from(matches)
}

fn phrase_matches_positions(left: &[u16], right: &[u16]) -> bool {
    left.iter().any(|position| {
        position
            .checked_add(1)
            .is_some_and(|adjacent| right.binary_search(&adjacent).is_ok())
    })
}

fn phrase_matches_document(left: &FieldPositions, right: &FieldPositions) -> bool {
    phrase_matches_positions(&left.text, &right.text)
        || phrase_matches_positions(&left.title, &right.title)
}

fn reference_phrase_matches_document(left: &FieldPositions, right: &FieldPositions) -> bool {
    fn adjacent(left: &[u16], right: &[u16]) -> bool {
        for &left_position in left {
            for &right_position in right {
                if left_position.checked_add(1) == Some(right_position) {
                    return true;
                }
            }
        }
        false
    }
    adjacent(&left.text, &right.text) || adjacent(&left.title, &right.title)
}

#[cfg(test)]
fn bad_phrase_without_positions(left: &DocSet, right: &DocSet) -> u64 {
    skip_intersection_count(&left.sparse, &right.sparse)
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
            let left = DocSet::new(20_003, df_ppm, 11);
            let right = DocSet::new(20_003, df_ppm, 29);
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
            let set = DocSet::new(20_003, df_ppm, 41);
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
            let left = DocSet::new(20_003, df_ppm, 43);
            let right = DocSet::new(20_003, df_ppm, 47);
            assert_eq!(
                phrase_from_bitmap(&left.bitmap, &right.bitmap, &corpus),
                phrase_from_skip(&left.sparse, &right.sparse, &corpus)
            );
        }
    }

    #[test]
    fn oracle_rejects_wrong_count_and_unverified_phrase_candidates() {
        assert_eq!(
            parity_outcome(4, 5, 4, || CaseOutcome::Eligible),
            CaseOutcome::Failed {
                reason: CaseFailure::OracleMismatch,
            }
        );

        let corpus = Corpus::new(20_003);
        let left = DocSet::new(20_003, 500_000, 43);
        let right = DocSet::new(20_003, 500_000, 47);
        let phrase = reference_phrase(&left, &right, &corpus);
        let candidates = phrase >> 32;
        let matches = phrase & u64::from(u32::MAX);
        assert_ne!(candidates, matches);
        assert_ne!(bad_phrase_without_positions(&left, &right), matches);
        assert_eq!(
            parity_outcome(
                bad_phrase_without_positions(&left, &right),
                matches,
                matches,
                || CaseOutcome::Eligible,
            ),
            CaseOutcome::Failed {
                reason: CaseFailure::OracleMismatch,
            }
        );
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

    #[test]
    fn parity_failures_are_typed_and_do_not_remove_the_case() {
        assert_eq!(
            parity_outcome(4, 5, 4, || CaseOutcome::Eligible),
            CaseOutcome::Failed {
                reason: CaseFailure::OracleMismatch,
            }
        );
        let report = run(Config {
            documents: 101,
            rounds: 1,
            repetition: 0,
            inject_failure: None,
        })
        .unwrap();
        assert_eq!(report.exact_set_count.len(), DF_BANDS_PPM.len());
        assert!(matches!(
            report.exact_set_count[0].outcome,
            CaseOutcome::Ineligible {
                reason: CaseExclusion::EmptyInputNegativeControl
            }
        ));
        let measurement_failure = CaseOutcome::Failed {
            reason: CaseFailure::MeasurementFailure,
        };
        assert_eq!(
            serde_json::to_value(&measurement_failure).unwrap()["reason"],
            "measurement_failure"
        );
        let (outcome, left, right) = measure_pair(CaseOutcome::Eligible, 0, true, || 1, || 1);
        assert_eq!(outcome, measurement_failure);
        assert!(left.is_none() && right.is_none());
    }

    #[test]
    fn run_identity_is_stable_and_changes_with_effective_config() {
        let config = Config {
            documents: 101,
            rounds: 2,
            repetition: 0,
            inject_failure: None,
        };
        assert_eq!(
            run_identity(config).identity_digest,
            run_identity(config).identity_digest
        );
        assert_ne!(
            run_identity(config).identity_digest,
            run_identity(Config {
                rounds: 3,
                ..config
            })
            .identity_digest
        );
        let json = serde_json::to_value(run(config).unwrap()).unwrap();
        assert_eq!(json["schema_version"], REPORT_SCHEMA_VERSION);
        assert_eq!(
            json["run_identity"]["report_schema_version"],
            REPORT_SCHEMA_VERSION
        );
        assert_eq!(json["exact_set_count"].as_array().unwrap().len(), 7);
    }

    #[test]
    fn phrase_semantics_cover_repetition_occurrences_order_and_fields() {
        assert!(phrase_matches_positions(&[2, 8], &[3, 12]));
        assert!(!phrase_matches_positions(&[2, 8], &[4, 12]));
        assert!(!phrase_matches_positions(&[3], &[2]));
        assert!(phrase_matches_positions(&[2, 3], &[2, 3]));
        assert!(!phrase_matches_positions(&[2], &[2]));

        let left = FieldPositions {
            text: vec![2],
            title: vec![],
        };
        let cross_field_right = FieldPositions {
            text: vec![],
            title: vec![3],
        };
        assert!(!phrase_matches_document(&left, &cross_field_right));
        let multiple = FieldPositions {
            text: vec![1, 3, 10],
            title: vec![],
        };
        let right = FieldPositions {
            text: vec![4, 20],
            title: vec![],
        };
        assert!(phrase_matches_document(&multiple, &right));
    }

    #[test]
    fn title_only_adds_one_moment_but_mixed_does_not_add_a_phantom() {
        let corpus = Corpus::new(DOCUMENTS_PER_VIDEO * 2);
        let mut title_videos = VideoSet::new(2);
        title_videos.insert(VideoId(0));
        title_videos.insert(VideoId(1));

        let mut documents = DocSet::new(DOCUMENTS_PER_VIDEO * 2, 0, 1);
        documents.sparse.push(DocId(0));
        documents.bitmap.insert(DocId(0));
        let answer = moments_with_title_matches(
            MatchSet {
                transcript_documents: &documents,
                title_videos: &title_videos,
            },
            &corpus,
        );
        let reference = reference_moments_with_titles(
            MatchSet {
                transcript_documents: &documents,
                title_videos: &title_videos,
            },
            &corpus,
        );
        assert_eq!((answer >> 32, answer & u64::from(u32::MAX)), (2, 2));
        assert_eq!(answer, reference);
    }
}
