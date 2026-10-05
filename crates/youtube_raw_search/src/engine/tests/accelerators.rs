//! Schema-9 sidecars: their contents equal what the reader would compute at
//! run time, and a schema-9 generation answers exactly as its schema-7 source.

use super::skips::build_skip_entries;
use super::*;
use crate::engine::accelerators::{
    AcceleratorConfig, BlockBound, score_factor, text_streams, upgrade_to_schema_nine,
};

// Small thresholds so a few hundred documents exercise every sidecar part.
const TEST_CONFIG: AcceleratorConfig = AcceleratorConfig {
    min_postings: 8,
    block_postings: 4,
    text_bitmap_budget_bytes: 64,
    title_bitmap_min_per_mille: 100,
    plan_bounds: crate::engine::plan_bounds::PlanBoundConfig {
        window_documents: 8,
        min_documents: 1,
    },
};

const WORDS: [&str; 16] = [
    "the", "and", "is", "this", "there", "end", "they", "of", "then", "fire", "react", "problem",
    "that", "world", "code", "coder",
];

/// Builds the same skewed corpus twice: a schema-7 generation and a copy
/// upgraded to schema 9.
fn schema_pair() -> (TempDir, RawSearchEngine, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    let mut state = 0x2545_f491_u64;
    let mut next = |bound: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (state >> 33) as usize % bound
    };
    for index in 0..400 {
        let video = index / 7;
        // Low word indices are far more common, as in real transcripts.
        let length = 3 + next(12);
        let text = (0..length)
            .map(|_| WORDS[next(WORDS.len()).min(next(WORDS.len()))])
            .collect::<Vec<_>>()
            .join(" ");
        let title = format!("{} {}", WORDS[video % 5], WORDS[4 + video % 8]);
        let start = (index % 7) as u64 * 15_000;
        let document = json!({
            "id": format!("v{video}-{index}"), "video_id": format!("v{video}"),
            "title": title, "text": text, "youtuber_handle": format!("@c{}", video % 3),
            "t_start_ms": start, "t_end_ms": start + 1000,
            "published_at_timestamp": video, "view_count": video * 13 % 97,
        });
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    let seven = root.path().join("seven");
    let nine = root.path().join("nine");
    build_index(&chunks_path, &channels_path, &seven).expect("build schema 7");
    build_index(&chunks_path, &channels_path, &nine).expect("build schema 9 source");
    upgrade_to_schema_nine(&nine, TEST_CONFIG).expect("add accelerators");
    let seven = RawSearchEngine::open(&seven).expect("open schema 7");
    let nine = RawSearchEngine::open(&nine).expect("open schema 9");
    (root, seven, nine)
}

fn accelerated_terms(engine: &RawSearchEngine) -> Vec<TermEntry> {
    (0..engine.inner.lexicon.count)
        .map(|index| {
            engine
                .inner
                .lexicon
                .entry_at(index)
                .expect("lexicon entry")
                .1
        })
        .filter(|entry| entry.text_frequency >= TEST_CONFIG.min_postings)
        .collect()
}

#[test]
fn schema_nine_answers_exactly_as_schema_seven() {
    let (_root, seven, nine) = schema_pair();
    assert_eq!(nine.inner.manifest.schema_version, 9);
    assert!(nine.inner.accelerators.is_some() && nine.inner.bitmaps.is_some());
    let mut queries = vec![
        "the",
        "and the",
        "this is the end",
        "\"this is the\"",
        "\"the end\"",
        "fire code",
        "the problem is that",
        "react -the",
        "wor",
        "end of the world",
        "\"and the\" code",
    ]
    .into_iter()
    .map(grouped_query)
    .collect::<Vec<_>>();
    for query in ["the end", "\"is the\"", "code that"] {
        queries.push(SearchParams {
            group_by: None,
            ..grouped_query(query)
        });
        queries.push(SearchParams {
            sort_by: Some("published_at_timestamp:desc".to_string()),
            ..grouped_query(query)
        });
        queries.push(SearchParams {
            filter_by: Some("youtuber_handle:=@c1".to_string()),
            page: Some(2),
            per_page: Some(3),
            offset: None,
            limit: None,
            ..grouped_query(query)
        });
    }
    for params in &queries {
        assert_eq!(
            seven.search(params).expect("schema 7 search"),
            nine.search(params).expect("schema 9 search"),
            "schema 9 diverged for {:?}",
            params.q
        );
    }
}

#[test]
fn sidecar_checkpoints_equal_runtime_checkpoints() {
    let (_root, _seven, nine) = schema_pair();
    let sidecar = nine.inner.accelerators.as_ref().expect("accelerators");
    let terms = accelerated_terms(&nine);
    assert!(terms.len() >= 5, "fixture must accelerate several terms");
    for entry in terms {
        let streams = text_streams(&nine, entry).expect("streams");
        let mut expected = build_skip_entries(streams, TEST_CONFIG.block_postings).expect("build");
        expected.retain(|checkpoint| checkpoint.decoded < entry.text_frequency);
        let stored = sidecar
            .checkpoints(entry.term, streams)
            .expect("valid checkpoints")
            .expect("accelerated term");
        assert_eq!(&*stored, expected.as_slice(), "term {}", entry.term);
    }
}

#[test]
fn block_bounds_are_the_maximum_factor_of_each_block() {
    let (_root, _seven, nine) = schema_pair();
    let sidecar = nine.inner.accelerators.as_ref().expect("accelerators");
    let average = nine.inner.manifest.average_document_tokens;
    for entry in accelerated_terms(&nine) {
        // Independent decode of (document, frequency, first position).
        let streams = text_streams(&nine, entry).expect("streams");
        let (mut cursor, mut first_cursor, mut document) = (0, 0, 0_u32);
        let mut postings = Vec::new();
        while cursor < streams.postings.len() {
            document += read_varint(streams.postings, &mut cursor, streams.postings.len()).unwrap();
            let frequency =
                read_varint(streams.postings, &mut cursor, streams.postings.len()).unwrap();
            let first = read_varint(
                streams.first_positions,
                &mut first_cursor,
                streams.first_positions.len(),
            )
            .unwrap();
            let tokens = nine
                .inner
                .metadata
                .search_document(document)
                .unwrap()
                .tokens;
            postings.push((document, score_factor(frequency, tokens, average, first)));
        }
        let expected = postings
            .chunks(TEST_CONFIG.block_postings as usize)
            .map(|block| BlockBound {
                last_document: block.last().unwrap().0,
                max_factor: block
                    .iter()
                    .map(|(_, factor)| *factor)
                    .fold(f32::MIN, f32::max),
            })
            .collect::<Vec<_>>();
        assert_eq!(sidecar.blocks(entry.term).unwrap().unwrap(), expected);
    }
}

#[test]
fn bitmaps_equal_posting_sets() {
    let (_root, _seven, nine) = schema_pair();
    let bitmaps = nine.inner.bitmaps.as_ref().expect("bitmaps");
    let documents = nine.inner.manifest.documents as usize;
    let mut text_bitmaps = 0;
    let mut title_bitmaps = 0;
    for index in 0..nine.inner.lexicon.count {
        let (_, entry) = nine.inner.lexicon.entry_at(index).unwrap();
        for field in [IndexedField::Text, IndexedField::Title] {
            let Some(words) = bitmaps.words(entry.term, field) else {
                continue;
            };
            let (offset, length) = match field {
                IndexedField::Text => (entry.text_offset, entry.text_length),
                IndexedField::Title => (entry.title_offset, entry.title_length),
            };
            let bytes = nine.postings_range(offset, length).unwrap();
            let (mut cursor, mut id, mut expected) = (0, 0_u32, Vec::new());
            while cursor < bytes.len() {
                id += read_varint(bytes, &mut cursor, bytes.len()).unwrap();
                read_varint(bytes, &mut cursor, bytes.len()).unwrap();
                expected.push(id);
            }
            let actual = words
                .enumerate()
                .flat_map(|(word, bits)| {
                    (0..64)
                        .filter(move |bit| bits & (1 << bit) != 0)
                        .map(move |bit| (word * 64 + bit) as u32)
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "term {} {field:?}", entry.term);
            match field {
                IndexedField::Text => text_bitmaps += 1,
                IndexedField::Title => title_bitmaps += 1,
            }
        }
    }
    // A 64-byte budget holds exactly one 400-document bitmap (56 bytes).
    assert_eq!(documents.div_ceil(64) * 8, 56);
    assert_eq!(text_bitmaps, 1);
    assert!(title_bitmaps > 0);
}

#[test]
fn corrupt_sidecars_are_rejected() {
    let (root, _seven, nine) = schema_pair();
    drop(nine);
    let index = root.path().join("nine");
    let path = index.join(crate::engine::accelerators::ACCELERATORS_FILE);
    let mut bytes = fs::read(&path).unwrap();
    // The first directory entry's block count.
    bytes[HEADER_BYTES + 8 + 4] ^= 1;
    fs::write(&path, &bytes).unwrap();
    assert!(RawSearchEngine::open(&index).is_err(), "checksum must fail");
    assert!(
        crate::engine::accelerators::Accelerators::open(&path, usize::MAX).is_err(),
        "structural validation must fail without the checksum"
    );
}

#[test]
fn only_schema_seven_generations_are_upgraded() {
    let (root, _seven, nine) = schema_pair();
    drop(nine);
    let error = upgrade_to_schema_nine(&root.path().join("nine"), TEST_CONFIG)
        .expect_err("schema 9 must not be upgraded twice");
    assert!(error.to_string().contains("found schema 9"), "{error}");
}

/// The complete scan's grouped response for `plans`, as the broad paths
/// build it before schema 9.
fn scanned_group_response(
    engine: &RawSearchEngine,
    params: &SearchParams,
    plans: &[conjunction::TermPlan],
    highlights: &[Vec<String>],
    dominant_exact_mask: u8,
) -> (Value, usize) {
    let hits_per_group = usize::from(params.group_limit.unwrap_or(5).max(1));
    let mut groups = engine.empty_group_slots();
    let mut documents = 0;
    let mut record = |candidate: DocMatch, record: SearchDocumentRecord| {
        documents += 1;
        engine.record_group_match_values(
            &mut groups,
            candidate,
            record.video,
            record.t_start_ms,
            hits_per_group,
            Some(dominant_exact_mask),
        )
    };
    if let [plan] = plans {
        engine
            .visit_term_plan(plan, &mut record)
            .expect("plan scan");
    } else {
        engine
            .visit_plan_conjunction(plans, &mut record)
            .expect("conjunction scan");
    }
    let response = engine
        .format_group_slots(params, highlights, groups, &mut BlockCache::default())
        .expect("format scan");
    (response, documents)
}

#[test]
fn accelerated_broad_pages_equal_the_complete_scan() {
    broad_fast::TEST_WINDOWS.set(Some((4, 2)));
    let (root, _seven, nine) = schema_pair();
    compare_accelerated_pages(&nine);

    // The same generation scanned as four video-disjoint partitions.
    let partitioned =
        RawSearchEngine::open_newly_built(&root.path().join("nine")).expect("open schema 9");
    let boundaries = partition::video_disjoint_boundaries(&partitioned.inner.videos);
    let splits = [1, 2, 3]
        .map(|quarter| boundaries[boundaries.len() * quarter / 4])
        .to_vec();
    assert!(splits.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        partitioned
            .inner
            .partition
            .set(Some(partition::DocumentPartition {
                text_checkpoints: vec![HashMap::new(); splits.len()],
                splits,
            }))
            .is_ok()
    );
    compare_accelerated_pages(&partitioned);
}

fn compare_accelerated_pages(nine: &RawSearchEngine) {
    let fields = query_fields(Some("text,title"), true).expect("fields");
    let sorts = [
        None,
        Some("published_at_timestamp:desc"),
        Some("published_at_timestamp:asc"),
        Some("view_count:desc"),
        Some("like_count:desc"),
        Some("comment_count:desc"),
        Some("_text_match:desc"),
    ];
    let windows = [(Some(0), Some(10)), (Some(3), Some(4)), (None, None)];
    let mut pages = 0;
    let mut pruned_rankings = 0;
    let mut declined = 0;
    for query in [
        "the",
        "and",
        "is",
        "the and",
        "and the",
        "this is the",
        "the end",
        "code th",
        "is the world",
        "fire code",
        "problem that",
        "end of the",
    ] {
        let terms = parse_query(query).expect("parse").terms;
        let (mut plans, highlights) = nine
            .conjunction_plans(&terms, fields, true, 0)
            .expect("plans");
        if plans.is_empty() {
            continue;
        }
        plans.sort_unstable_by_key(|plan| plan.estimated_documents);
        let dominant = plans.iter().fold(0, |mask, plan| mask | plan.exact_mask());
        for sort in sorts {
            let order = broad_fast::accelerated_group_order(sort).expect("supported");
            let Some(analysis) = nine.analyze_broad_plans(&plans, order).expect("analysis") else {
                // Only relevance declines, and only for a multi-alternative
                // plan without a covering stored bound.
                assert_eq!(order, broad_fast::GroupOrder::Relevance, "{query:?}");
                declined += 1;
                continue;
            };
            for (offset, limit) in windows {
                for group_limit in [1, 3] {
                    let params = SearchParams {
                        sort_by: sort.map(str::to_string),
                        offset,
                        limit,
                        page: offset.is_none().then_some(2),
                        per_page: offset.is_none().then_some(5),
                        group_limit: Some(group_limit),
                        ..grouped_query(query)
                    };
                    let (expected, documents) =
                        scanned_group_response(nine, &params, &plans, &highlights, dominant);
                    assert_eq!(analysis.totals.documents, documents, "{query:?}");
                    let actual = nine
                        .accelerated_broad_page(
                            &params,
                            &plans,
                            &analysis,
                            order,
                            &highlights,
                            usize::from(group_limit),
                            dominant,
                            &mut BlockCache::default(),
                        )
                        .expect("fast path")
                        .expect("fixture ties stay under the field-sort cap");
                    assert_eq!(
                        actual, expected,
                        "{query:?} sort {sort:?} window {offset:?}/{limit:?}"
                    );
                    pages += 1;
                    if order == broad_fast::GroupOrder::Relevance {
                        let (scored, candidates) = broad_fast::LAST_RELEVANCE_WINDOWS.get();
                        pruned_rankings += usize::from(scored < candidates);
                    }
                }
            }
        }
    }
    assert!(pages > 300, "{pages} pages compared");
    assert!(
        declined < 10,
        "{declined} relevance queries lacked plan bounds"
    );
    assert!(pruned_rankings > 0, "no relevance ranking skipped a window");
}

#[test]
fn engagement_sort_keeps_the_scan() {
    assert!(broad_fast::accelerated_group_order(Some("engagement_score:desc")).is_none());
}

#[test]
#[ignore = "diagnostic: needs RAW_TOTALS_INDEX pointing at a schema-9 generation"]
fn measure_accelerated_broad_relevance() {
    let index = std::env::var("RAW_TOTALS_INDEX").expect("set RAW_TOTALS_INDEX");
    let engine = RawSearchEngine::open(Path::new(&index)).expect("open generation");
    engine.warm();
    let fields = query_fields(Some("text,title"), true).expect("fields");
    for query in [
        "the",
        "and the",
        "in the",
        "this is the",
        "you know",
        "i think",
    ] {
        let terms = parse_query(query).expect("parse").terms;
        let (mut plans, highlights) = engine.conjunction_plans(&terms, fields, true, 0).unwrap();
        plans.sort_unstable_by_key(|plan| plan.estimated_documents);
        let alternatives = plans
            .iter()
            .map(|plan| plan.alternatives.len())
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        let mut analysis = engine
            .analyze_broad_plans(&plans, broad_fast::GroupOrder::Relevance)
            .unwrap()
            .unwrap();
        let analysis_ms = started.elapsed().as_secs_f64() * 1e3;
        if std::env::var_os("RAW_EXACT_PLAN_BOUNDS").is_some() {
            // Replace each window's bound with the sum over plans of the
            // plan's true maximum score in the window.
            let window = 512;
            let mut bounds = vec![0.0_f64; analysis.window_bounds.len()];
            for plan in &plans {
                let mut maxima = vec![0.0_f32; bounds.len()];
                engine
                    .visit_term_plan(plan, |candidate, _| {
                        let slot = &mut maxima[(candidate.document() / window) as usize];
                        *slot = slot.max(candidate.score);
                        Ok(())
                    })
                    .unwrap();
                for (bound, maximum) in bounds.iter_mut().zip(maxima) {
                    *bound += f64::from(maximum);
                }
            }
            analysis.window_bounds = bounds;
        }
        let dominant = plans.iter().fold(0, |mask, plan| mask | plan.exact_mask());
        let started = std::time::Instant::now();
        engine
            .accelerated_broad_page(
                &grouped_query(query),
                &plans,
                &analysis,
                broad_fast::GroupOrder::Relevance,
                &highlights,
                3,
                dominant,
                &mut BlockCache::default(),
            )
            .unwrap();
        let page_ms = started.elapsed().as_secs_f64() * 1e3;
        let (scored, candidates) = broad_fast::LAST_RELEVANCE_WINDOWS.get();
        println!(
            "{{\"query\":{query:?},\"alternatives\":{alternatives:?},\"documents\":{},\"analysis_ms\":{analysis_ms:.1},\"page_ms\":{page_ms:.1},\"scored_windows\":{scored},\"candidate_windows\":{candidates}}}",
            analysis.totals.documents
        );
    }
}

#[test]
fn plan_bounds_cover_each_window_within_one_step() {
    let (_root, _seven, nine) = schema_pair();
    let bounds = nine.inner.plan_bounds.as_ref().expect("plan bounds");
    let window = bounds.window_documents();
    assert_eq!(window, TEST_CONFIG.plan_bounds.window_documents);
    let fields = query_fields(Some("text,title"), true).expect("fields");
    let mut checked = 0;
    for index in 0..nine.inner.lexicon.count {
        let (term, entry) = nine.inner.lexicon.entry_at(index).unwrap();
        let Some(bound) = bounds.get(entry.term) else {
            continue;
        };
        let (plans, _) = nine
            .conjunction_plans(&[term.to_string()], fields, true, 0)
            .unwrap();
        assert!(bound.covers(&plans[0]), "{term}");
        let windows = nine.inner.manifest.documents.div_ceil(window) as usize;
        let mut maxima = vec![0.0_f32; windows];
        nine.visit_term_plan(&plans[0], |candidate, _| {
            let slot = &mut maxima[(candidate.document() / window) as usize];
            *slot = slot.max(candidate.score);
            Ok(())
        })
        .unwrap();
        let top = maxima.iter().copied().fold(0.0_f32, f32::max);
        let step = f64::from(top) / 255.0 * 1.001;
        for (stored, actual) in bound.maxima().zip(&maxima) {
            assert!(stored >= f64::from(*actual), "{term}: {stored} < {actual}");
            assert!(
                stored <= f64::from(*actual) + step,
                "{term}: {stored} is loose"
            );
        }
        // Fewer alternatives or disabled position boosts only lower scores.
        let (unboosted, _) = nine
            .conjunction_plans(
                &[term.to_string()],
                query_fields(Some("text"), false).unwrap(),
                true,
                0,
            )
            .unwrap();
        if let Some(plan) = unboosted.first()
            && plan.alternatives.len() > 1
            && bound.covers(plan)
        {
            nine.visit_term_plan(plan, |candidate, _| {
                let stored = bound
                    .maxima()
                    .nth((candidate.document() / window) as usize)
                    .unwrap();
                assert!(stored >= f64::from(candidate.score) * (1.0 - 1e-6));
                Ok(())
            })
            .unwrap();
        }
        checked += 1;
    }
    assert!(checked > 0, "fixture must store plan bounds");
}

#[test]
fn corrupt_plan_bounds_are_rejected() {
    let (root, _seven, nine) = schema_pair();
    let terms = nine.inner.lexicon.count;
    let documents = nine.inner.manifest.documents;
    drop(nine);
    let path = root
        .path()
        .join("nine")
        .join(crate::engine::plan_bounds::PLAN_BOUNDS_FILE);
    let mut bytes = fs::read(&path).unwrap();
    // The first entry's alternative count.
    bytes[HEADER_BYTES + 8 + 4] = 1;
    fs::write(&path, &bytes).unwrap();
    assert!(crate::engine::plan_bounds::PlanBounds::open(&path, terms, documents).is_err());
    // A different window count must not be accepted either.
    assert!(crate::engine::plan_bounds::PlanBounds::open(&path, terms, documents + 64).is_err());
}
