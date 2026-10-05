use super::*;

fn common_word_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for (index, (video, title, text, start)) in [
        (
            "adjacent",
            "Lecture",
            "this is the compiler and the runtime",
            0,
        ),
        ("adjacent", "Lecture", "and the compiler", 19_999),
        ("adjacent", "Lecture", "and the compiler", 20_000),
        ("adjacent", "Lecture", "and the compiler", 20_001),
        ("gapped", "Lecture", "and compiler the", 0),
        ("reversed", "Lecture", "the and compiler", 0),
        ("duplicate", "Lecture", "the the and compiler", 0),
        ("cross", "the", "and compiler", 0),
        ("prefix", "Lecture", "and theory compiler", 0),
        ("without", "Lecture", "compiler and runtime", 0),
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

fn group_ids(response: &Value) -> HashSet<String> {
    response["grouped_hits"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|group| group["group_key"][0].as_str().expect("id").to_owned())
        .collect()
}

#[test]
fn common_words_in_phrases_keep_order_adjacency_duplicates_and_exclusions() {
    let (_root, engine) = common_word_engine();
    for (query, expected) in [
        (r#""and the""#, vec!["adjacent"]),
        (r#""the and""#, vec!["reversed", "duplicate"]),
        (r#""the the""#, vec!["duplicate"]),
        (r#""this is the""#, vec!["adjacent"]),
        (r#""and the" compiler"#, vec!["adjacent"]),
        ("compiler -the", vec!["prefix", "without"]),
    ] {
        let response = engine.search(&grouped_query(query)).expect("search");
        assert_eq!(
            group_ids(&response),
            expected.into_iter().map(str::to_owned).collect(),
            "{query}"
        );
    }
}

#[test]
fn common_word_conjunction_keeps_separated_cross_field_and_prefix_matches() {
    let (_root, engine) = common_word_engine();
    for (query, prefix, expected) in [
        (
            "and the",
            false,
            vec!["adjacent", "gapped", "reversed", "duplicate", "cross"],
        ),
        (
            "and the",
            true,
            vec![
                "adjacent",
                "gapped",
                "reversed",
                "duplicate",
                "cross",
                "prefix",
            ],
        ),
        (
            "the compiler",
            false,
            vec!["adjacent", "gapped", "reversed", "duplicate", "cross"],
        ),
        ("this is the", false, vec!["adjacent"]),
    ] {
        let terms = ordered_tokens(query);
        let fields = query_fields(Some("text,title"), true).expect("fields");
        let (matches, highlights) = engine
            .exact_prefix_conjunction(&terms, fields, prefix, None, &mut BlockCache::default())
            .expect("conjunction");
        let response = engine
            .format_matches(
                &grouped_query(query),
                &highlights,
                matches,
                &mut BlockCache::default(),
            )
            .expect("response");
        assert_eq!(
            group_ids(&response),
            expected.into_iter().map(str::to_owned).collect(),
            "{query}, prefix={prefix}"
        );
    }
}

#[test]
fn common_word_streaming_reducer_matches_independent_materialized_json() {
    let (_root, engine) = common_word_engine();
    for query in ["and the", "this is the", "the compiler", "and the the"] {
        for position in [false, true] {
            for query_by in ["text", "title", "text,title"] {
                let fields = query_fields(Some(query_by), position).expect("fields");
                let (mut plans, highlights) = engine
                    .conjunction_plans(&ordered_tokens(query), fields, true, 0)
                    .expect("plans");
                plans.sort_unstable_by_key(|plan| plan.estimated_documents);
                let dominant_exact_mask =
                    plans.iter().fold(0, |mask, plan| mask | plan.exact_mask());
                let materialized = intersect_all_borrowed(
                    &plans
                        .iter()
                        .map(|plan| {
                            engine
                                .matches_for_plan(plan, None, &mut BlockCache::default())
                                .expect("plan matches")
                        })
                        .collect::<Vec<_>>(),
                );
                for sort in [
                    None,
                    Some("_text_match:desc"),
                    Some("published_at_timestamp:asc"),
                    Some("published_at_timestamp:desc"),
                    Some("view_count:desc"),
                    Some("like_count:desc"),
                    Some("comment_count:desc"),
                    Some("engagement_score:desc"),
                ] {
                    for group_limit in [1, 3, 5, 249, 250] {
                        let mut params = grouped_query(query);
                        params.query_by = Some(query_by.to_owned());
                        params.prioritize_token_position = Some(position);
                        params.sort_by = sort.map(str::to_owned);
                        params.group_limit = Some(group_limit);
                        let expected = engine
                            .format_matches(
                                &params,
                                &highlights,
                                materialized.clone(),
                                &mut BlockCache::default(),
                            )
                            .expect("materialized response");
                        let mut groups = engine.empty_group_slots();
                        engine
                            .visit_plan_conjunction(&plans, |candidate, record| {
                                engine.record_group_match_values(
                                    &mut groups,
                                    candidate,
                                    record.video,
                                    record.t_start_ms,
                                    usize::from(group_limit),
                                    Some(dominant_exact_mask),
                                )
                            })
                            .expect("stream groups");
                        let actual = engine
                            .format_group_slots(
                                &params,
                                &highlights,
                                groups,
                                &mut BlockCache::default(),
                            )
                            .expect("stream response");
                        assert_eq!(
                            actual, expected,
                            "{query}; fields={query_by}; position={position}; sort={sort:?}; limit={group_limit}"
                        );
                    }
                }
            }
        }
    }
}
