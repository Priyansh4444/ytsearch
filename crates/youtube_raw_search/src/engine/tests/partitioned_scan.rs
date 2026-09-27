use super::*;

// Source order interleaves "alpha" around "beta", as it does for a few hundred
// production videos, so only some document IDs are safe partition boundaries.
fn interleaved_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for (index, (video, title, text, start)) in [
        ("alpha", "The compiler", "and the parser", 0),
        ("alpha", "The compiler", "the and runtime", 10_000),
        ("beta", "Runtime notes", "and the runtime", 0),
        ("beta", "Runtime notes", "the parser and", 30_000),
        ("alpha", "The compiler", "and the linker", 50_000),
        ("gamma", "Parser and lexer", "the lexer and the parser", 0),
        ("gamma", "Parser and lexer", "and then the end", 25_000),
        ("delta", "The end", "and the finale", 0),
        ("epsilon", "Other", "unrelated words only", 0),
        ("zeta", "And the rest", "the rest and more", 0),
    ]
    .into_iter()
    .enumerate()
    {
        let document = json!({
            "id": format!("{video}-{index}"), "video_id": video,
            "title": title, "text": text, "youtuber_handle": "@test",
            "t_start_ms": start, "t_end_ms": start + 1000,
            "published_at_timestamp": index + 1,
            "view_count": index * 100, "like_count": index * 10,
            "comment_count": index
        });
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build");
    let engine = RawSearchEngine::open(&index_path).expect("open");
    (root, engine)
}

#[test]
fn boundaries_exclude_positions_inside_interleaved_videos() {
    let (_root, engine) = interleaved_engine();
    // alpha owns 0, 1, and 4; beta owns 2 and 3. Nothing may split before 5.
    assert_eq!(
        partition::video_disjoint_boundaries(&engine.inner.videos),
        vec![5, 7, 8, 9]
    );
}

#[test]
fn merge_rejects_a_video_present_in_two_partitions() {
    let (_root, engine) = interleaved_engine();
    let group = || VideoMatches {
        video: 0,
        score: 1.0,
        exact_tokens: 0,
        exact_mask: 0,
        ordered_phrase: false,
        moments: MomentCounter::default(),
        documents: SmallVec::new(),
    };
    let mut groups = engine.empty_group_slots();
    let mut first = engine.empty_group_slots();
    first[0] = Some(group());
    let mut second = engine.empty_group_slots();
    second[0] = Some(group());
    partition::merge_disjoint_groups(&mut groups, first).expect("first partition");
    assert!(partition::merge_disjoint_groups(&mut groups, second).is_err());
}

fn checkpoints_at(
    engine: &RawSearchEngine,
    split: u32,
) -> HashMap<u32, partition::TextSeekCheckpoint> {
    partition::text_seek_checkpoints(
        &engine.inner.lexicon,
        engine.inner.postings.bytes(),
        &engine.inner.positions,
        engine.inner.manifest.documents,
        &[split],
        0,
    )
    .expect("checkpoints")
    .remove(0)
}

#[test]
fn checkpoints_resume_exactly_where_decoding_stops() {
    let (_root, engine) = interleaved_engine();
    let checkpoints = checkpoints_at(&engine, 5);
    let (_, entry) = (0..engine.inner.lexicon.count)
        .map(|index| engine.inner.lexicon.entry_at(index).expect("entry"))
        .find(|(term, _)| *term == "the")
        .expect("the");
    // "the" occurs in text documents 0-4 before the split.
    let checkpoint = checkpoints[&entry.term];
    assert_eq!(checkpoint.decoded, 5);
    assert_eq!(checkpoint.previous_document, 4);
    assert!(checkpoint.position_cursor.is_some());
    assert!(checkpoint.posting_cursor < entry.text_length);
}

fn grouped_response(
    engine: &RawSearchEngine,
    params: &SearchParams,
    highlights: &[Vec<String>],
    ranges: &[std::ops::Range<u32>],
    mut visit: impl FnMut(
        std::ops::Range<u32>,
        &mut dyn FnMut(DocMatch, SearchDocumentRecord) -> Result<()>,
    ) -> Result<()>,
) -> Value {
    let hits_per_group = usize::from(params.group_limit.expect("group limit"));
    let mut groups = engine.empty_group_slots();
    for range in ranges {
        let mut partition = engine.empty_group_slots();
        visit(range.clone(), &mut |candidate, record| {
            engine.record_group_match_values(
                &mut partition,
                candidate,
                record.video,
                record.t_start_ms,
                hits_per_group,
                None,
            )
        })
        .expect("partition visit");
        partition::merge_disjoint_groups(&mut groups, partition).expect("disjoint partitions");
    }
    engine
        .format_group_slots(params, highlights, groups, &mut BlockCache::default())
        .expect("grouped response")
}

#[test]
fn partitioned_scans_match_whole_scans_at_every_boundary() {
    let (_root, engine) = interleaved_engine();
    let documents = engine.inner.manifest.documents;
    let fields = query_fields(None, true).expect("query fields");
    let boundaries = partition::video_disjoint_boundaries(&engine.inner.videos);
    assert!(!boundaries.is_empty());

    let terms = vec!["and".to_string(), "the".to_string()];
    let (plans, highlights) = engine
        .conjunction_plans(&terms, fields, true, 0)
        .expect("conjunction plans");
    let params = grouped_query("and the");
    let whole = grouped_response(
        &engine,
        &params,
        &highlights,
        std::slice::from_ref(&(0..documents)),
        |range, visit| engine.visit_plan_conjunction_range(&plans, range, visit),
    );
    assert!(whole["found"].as_u64().is_some_and(|found| found >= 4));
    for &boundary in &boundaries {
        let checkpoints = checkpoints_at(&engine, boundary);
        for checkpoints in [None, Some(&checkpoints)] {
            let split = grouped_response(
                &engine,
                &params,
                &highlights,
                &[0..boundary, boundary..documents],
                |range, visit| {
                    let checkpoints = checkpoints.filter(|_| range.start == boundary);
                    engine.visit_plan_conjunction_from(&plans, range, checkpoints, visit)
                },
            );
            assert_eq!(split, whole, "conjunction split at {boundary}");
        }
    }

    let terms = vec!["the".to_string()];
    let (plans, highlights) = engine
        .conjunction_plans(&terms, fields, true, 0)
        .expect("term plan");
    let [plan] = plans.as_slice() else {
        panic!("single token must produce one plan");
    };
    let params = grouped_query("the");
    let whole = grouped_response(
        &engine,
        &params,
        &highlights,
        std::slice::from_ref(&(0..documents)),
        |range, visit| engine.visit_term_plan_range(plan, range, visit),
    );
    for &boundary in &boundaries {
        let checkpoints = checkpoints_at(&engine, boundary);
        for checkpoints in [None, Some(&checkpoints)] {
            let split = grouped_response(
                &engine,
                &params,
                &highlights,
                &[0..boundary, boundary..documents],
                |range, visit| {
                    let checkpoints = checkpoints.filter(|_| range.start == boundary);
                    engine.visit_term_plan_from(plan, range, checkpoints, visit)
                },
            );
            assert_eq!(split, whole, "term split at {boundary}");
        }
    }
}

// One decoding pass records every split's resume point; each must equal the
// point a separate single-split pass records.
#[test]
fn multi_split_checkpoints_match_single_split_passes() {
    let (_root, engine) = interleaved_engine();
    let boundaries = partition::video_disjoint_boundaries(&engine.inner.videos);
    assert!(boundaries.len() >= 3);
    let combined = partition::text_seek_checkpoints(
        &engine.inner.lexicon,
        engine.inner.postings.bytes(),
        &engine.inner.positions,
        engine.inner.manifest.documents,
        &boundaries,
        0,
    )
    .expect("combined checkpoints");
    for (boundary, checkpoints) in boundaries.iter().zip(&combined) {
        assert_eq!(
            *checkpoints,
            checkpoints_at(&engine, *boundary),
            "split {boundary}"
        );
    }
}

#[test]
fn many_way_partitioned_scans_match_the_whole_scan() {
    let (_root, engine) = interleaved_engine();
    let documents = engine.inner.manifest.documents;
    let fields = query_fields(None, true).expect("query fields");
    let boundaries = partition::video_disjoint_boundaries(&engine.inner.videos);
    let checkpoints = partition::text_seek_checkpoints(
        &engine.inner.lexicon,
        engine.inner.postings.bytes(),
        &engine.inner.positions,
        documents,
        &boundaries,
        0,
    )
    .expect("checkpoints");
    let starts = std::iter::once(0).chain(boundaries.iter().copied());
    let ends = boundaries.iter().copied().chain(std::iter::once(documents));
    let ranges = starts
        .zip(ends)
        .map(|(start, end)| start..end)
        .collect::<Vec<_>>();
    assert!(ranges.len() >= 4);

    let terms = vec!["and".to_string(), "the".to_string()];
    let (plans, highlights) = engine
        .conjunction_plans(&terms, fields, true, 0)
        .expect("conjunction plans");
    let params = grouped_query("and the");
    let whole = grouped_response(
        &engine,
        &params,
        &highlights,
        std::slice::from_ref(&(0..documents)),
        |range, visit| engine.visit_plan_conjunction_range(&plans, range, visit),
    );
    let split = grouped_response(&engine, &params, &highlights, &ranges, |range, visit| {
        let resume = boundaries
            .binary_search(&range.start)
            .ok()
            .map(|index| &checkpoints[index]);
        engine.visit_plan_conjunction_from(&plans, range, resume, visit)
    });
    assert_eq!(split, whole);
}
