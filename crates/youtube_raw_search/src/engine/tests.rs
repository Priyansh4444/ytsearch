use super::*;
use std::{fs::OpenOptions, io::Write};
use tempfile::TempDir;

fn test_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"id":"a-1","video_id":"a","title":"Rust performance","text":"a compact inverted search index about relativity","youtuber_handle":"@one","youtuber_name":"One","t_start_ms":1000,"t_end_ms":2000,"published_at_timestamp":20}),
        json!({"id":"a-2","video_id":"a","title":"Rust performance","text":"memory mapped search is fast","youtuber_handle":"@one","youtuber_name":"One","t_start_ms":30000,"t_end_ms":31000,"published_at_timestamp":20}),
        json!({"id":"b-1","video_id":"b","title":"JavaScript runtimes","text":"Bun starts quickly","youtuber_handle":"@Two","youtuber_name":"Two","t_start_ms":1000,"t_end_ms":2000,"published_at_timestamp":10}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    let mut channels = File::create(&channels_path).expect("channels");
    writeln!(
        channels,
        "{}",
        json!({"id":"one","name":"One","handle":"@one"})
    )
    .expect("write channel");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    (root, engine)
}

fn blackhole_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"id":"literal-1","video_id":"literal","title":"Compiler blackhole identifier","text":"the blackhole optimization marker","youtuber_handle":"@other","youtuber_name":"Other","published_at":"1970-01-01T00:00:30Z"}),
        json!({"id":"veritasium-1","video_id":"veritasium","title":"A Black Hole Is Not Black","text":"a black hole bends spacetime","youtuber_handle":"@Veritasium","youtuber_name":"Veritasium","published_at":"1970-01-01T00:00:10Z"}),
        json!({"id":"vsauce-1","video_id":"vsauce","title":"Questions About Space","text":"what happens near black holes","youtuber_handle":"@Vsauce","youtuber_name":"Vsauce","published_at":"1970-01-01T00:00:20Z"}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    let mut channels = File::create(&channels_path).expect("channels");
    for channel in [
        json!({"id":"veritasium","name":"Veritasium","handle":"@veritasium"}),
        json!({"id":"vsauce","name":"Vsauce","handle":"@vsauce"}),
        json!({"id":"other","name":"Other","handle":"@other"}),
    ] {
        writeln!(channels, "{channel}").expect("write channel");
    }
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    (root, engine)
}

fn weighted_blackhole_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"id":"black-heavy-1","video_id":"black-heavy","title":"black black black black black hole","text":"space","youtuber_handle":"@one","youtuber_name":"One"}),
        json!({"id":"hole-heavy-1","video_id":"hole-heavy","title":"black hole hole hole hole","text":"space","youtuber_handle":"@two","youtuber_name":"Two"}),
        json!({"id":"blank-distractor-1","video_id":"blank-distractor","title":"blank blank blank blank tape with a hole","text":"space","youtuber_handle":"@three","youtuber_name":"Three"}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    (root, engine)
}

fn engagement_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"id":"small-1","video_id":"small","title":"Space systems","text":"orbital mechanics explained","youtuber_handle":"@one","youtuber_name":"One","view_count":100,"like_count":20,"comment_count":2}),
        json!({"id":"large-1","video_id":"large","title":"Space systems","text":"orbital mechanics explained","youtuber_handle":"@two","youtuber_name":"Two","view_count":1_000_000,"like_count":100_000,"comment_count":10_000}),
        json!({"id":"exact-1","video_id":"exact","title":"Orbital mechanics","text":"an introduction","youtuber_handle":"@three","youtuber_name":"Three","view_count":1,"like_count":1,"comment_count":1}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    (root, engine)
}

fn position_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"id":"early-1","video_id":"early","title":"Same","text":"target alpha beta gamma","youtuber_handle":"@one","youtuber_name":"One"}),
        json!({"id":"late-1","video_id":"late","title":"Same","text":"alpha beta gamma target","youtuber_handle":"@two","youtuber_name":"Two"}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    (root, engine)
}

fn token_drop_engine() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"id":"prefix-pair-1","video_id":"prefix-pair","title":"Alpha beta","text":"paired terms","youtuber_handle":"@one","youtuber_name":"One"}),
        json!({"id":"suffix-pair-1","video_id":"suffix-pair","title":"Beta gamma","text":"paired terms","youtuber_handle":"@two","youtuber_name":"Two"}),
        json!({"id":"single-1","video_id":"single","title":"Alpha alone","text":"one term only","youtuber_handle":"@three","youtuber_name":"Three"}),
        json!({"id":"exact-relativity-1","video_id":"exact-relativity","title":"Relativity explained","text":"a clear account","youtuber_handle":"@one","youtuber_name":"One"}),
        json!({"id":"fuzzy-relativity-1","video_id":"fuzzy-relativity","title":"Relatively explained explained explained explained","text":"a repeated account","youtuber_handle":"@two","youtuber_name":"Two"}),
        json!({"id":"early-exact-1","video_id":"early-exact","title":"Planet oribt","text":"an account","youtuber_handle":"@one","youtuber_name":"One"}),
        json!({"id":"late-exact-1","video_id":"late-exact","title":"Planat orbit orbit orbit","text":"a repeated account","youtuber_handle":"@two","youtuber_name":"Two"}),
        json!({"id":"battery-pair-1","video_id":"battery-pair","title":"Battery gate","text":"exact retained pair","youtuber_handle":"@one","youtuber_name":"One"}),
        json!({"id":"sparse-fuzzy-1","video_id":"sparse-fuzzy","title":"Battery gait incident","text":"one fuzzy full-query result","youtuber_handle":"@two","youtuber_name":"Two"}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    (root, engine)
}

fn grouped_query(query: &str) -> SearchParams {
    SearchParams {
        q: query.to_string(),
        query_by: Some("text,title".to_string()),
        filter_by: None,
        group_by: Some("video_id".to_string()),
        group_limit: Some(3),
        per_page: None,
        page: None,
        offset: Some(0),
        limit: Some(10),
        sort_by: None,
        use_cache: None,
        cache_ttl: None,
        ..SearchParams::default()
    }
}

#[test]
fn raw_index_groups_chunks_and_highlights_matches() {
    let (_root, engine) = test_engine();
    let response = engine
        .search(&grouped_query("memory mapped"))
        .expect("search");
    assert_eq!(response["found"], 1);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "a");
    assert!(
        response["grouped_hits"][0]["hits"][0]["highlights"][0]["snippet"]
            .as_str()
            .expect("snippet")
            .contains("<mark>")
    );
}

#[test]
fn highlights_every_whole_query_token_without_matching_token_substrings() {
    let snippet = highlighted_snippet(
        "includes things like Uno CSS Wy CSS which I think is dead now Panda CSS",
        &ordered_tokens("Css which i think is dead now"),
        360,
    )
    .expect("highlighted snippet");

    assert!(snippet.starts_with("includes"));
    assert!(!snippet.starts_with("<mark>i</mark>ncludes"));
    for token in ["CSS", "which", "I", "think", "is", "dead", "now"] {
        assert!(
            snippet.contains(&format!("<mark>{token}</mark>")),
            "missing token highlight: {token} in {snippet}"
        );
    }
}

#[test]
fn mixed_exact_and_typo_highlights_reconstruct_each_query_token() {
    let groups = vec![
        vec!["memroy".to_string(), "memory".to_string()],
        vec!["mapped".to_string(), "mapping".to_string()],
    ];
    let selected = matched_highlight_terms("memory mapped systems", &groups);

    assert_eq!(selected, ["memory", "mapped"]);
    let snippet = highlighted_snippet("memory mapped systems", &selected, 180).expect("snippet");
    assert_eq!(snippet, "<mark>memory</mark> <mark>mapped</mark> systems");
}

#[test]
fn compact_split_highlights_both_matched_components() {
    let groups = vec![vec![
        "blackhole".to_string(),
        "black".to_string(),
        "hole".to_string(),
    ]];
    assert_eq!(
        matched_highlight_terms("A Black Hole Is Not Black", &groups),
        ["black", "hole"]
    );
}

#[test]
fn schema_six_reconstructs_complete_positions_from_aligned_sidecars() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    writeln!(
        File::create(&chunks_path).expect("chunks"),
        "{}",
        json!({
            "video_id":"a",
            "title":"Positions",
            "text":"one echo two echo echo",
            "youtuber_handle":"@one"
        })
    )
    .expect("write chunk");
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build");
    let engine = RawSearchEngine::open(&index_path).expect("open");

    let entry = engine
        .inner
        .lexicon
        .lookup("echo")
        .expect("lookup")
        .expect("echo term");
    let posting_bytes = engine
        .postings_range(entry.text_offset, entry.text_length)
        .expect("posting range");
    let mut posting_cursor = 0;
    assert_eq!(
        read_varint(posting_bytes, &mut posting_cursor, posting_bytes.len()).expect("document"),
        0
    );
    let frequency =
        read_varint(posting_bytes, &mut posting_cursor, posting_bytes.len()).expect("frequency");
    assert_eq!(posting_cursor, posting_bytes.len());

    let first_bytes = engine
        .inner
        .positions
        .range(entry.term, false)
        .expect("first positions");
    let mut first_cursor = 0;
    let first =
        read_varint(first_bytes, &mut first_cursor, first_bytes.len()).expect("first position");
    assert_eq!(first_cursor, first_bytes.len());
    let remaining = engine
        .inner
        .full_positions
        .as_ref()
        .expect("schema 6 positions")
        .range(entry.term, false)
        .expect("remaining positions");
    let mut positions = Vec::new();
    let mut cursor = crate::positions::PositionCursor::new(remaining);
    cursor
        .read_into(first, frequency, &mut positions)
        .expect("decode positions");
    cursor.finish().expect("consume position range");
    assert_eq!(positions, [1, 3, 4]);
}

#[test]
fn query_parser_preserves_phrase_order_and_duplicate_tokens() {
    let parsed = parse_query(r#"outside "black black hole" tail"#).expect("parse query");
    assert_eq!(parsed.terms, ["outside", "black", "hole", "tail"]);
    assert_eq!(parsed.phrases, [["black", "black", "hole"]]);
    assert!(parsed.excluded_terms.is_empty());
    assert!(parsed.excluded_phrases.is_empty());

    let escaped = parse_query(r#"say \"hello\""#).expect("escaped quotes");
    assert_eq!(escaped.terms, ["say", "hello"]);
    assert!(escaped.phrases.is_empty());
    assert!(escaped.excluded_terms.is_empty());
    assert!(escaped.excluded_phrases.is_empty());

    let excluded = parse_query("alpha -beta x-ray -beta").expect("negative term");
    assert_eq!(excluded.terms, ["alpha", "x", "ray"]);
    assert_eq!(excluded.excluded_terms, ["beta"]);
    assert!(excluded.excluded_phrases.is_empty());

    let excluded_phrase =
        parse_query(r#"outside -"black black hole" -"tail""#).expect("negative phrases");
    assert_eq!(excluded_phrase.terms, ["outside"]);
    assert_eq!(excluded_phrase.excluded_terms, ["tail"]);
    assert_eq!(
        excluded_phrase.excluded_phrases,
        [["black", "black", "hole"]]
    );

    let adjacent = parse_query(r#""alpha"-beta"#).expect("adjacent hyphen");
    assert_eq!(adjacent.terms, ["alpha", "beta"]);
    assert!(adjacent.excluded_terms.is_empty());

    assert!(parse_query(r#""unterminated"#).is_err());
    assert!(parse_query(r#"empty "" phrase"#).is_err());
    for invalid in ["alpha - beta", "alpha -", "--alpha", r#"alpha - "beta""#] {
        assert!(parse_query(invalid).is_err(), "accepted {invalid:?}");
    }
}

#[test]
fn quoted_phrases_require_order_adjacency_and_one_field() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"video_id":"adjacent","title":"A","text":"black hole forms in deep space","youtuber_handle":"@one"}),
        json!({"video_id":"gapped","title":"B","text":"black deep hole","youtuber_handle":"@one"}),
        json!({"video_id":"reversed","title":"C","text":"hole black","youtuber_handle":"@one"}),
        json!({"video_id":"duplicate","title":"D","text":"black black hole","youtuber_handle":"@one"}),
        json!({"video_id":"title","title":"Black Hole Lecture","text":"astronomy","youtuber_handle":"@one"}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build");
    let engine = RawSearchEngine::open(&index_path).expect("open");

    let ids = |query: &str, query_by: &str, slop: Option<u8>| {
        let mut params = grouped_query(query);
        params.query_by = Some(query_by.to_string());
        params.phrase_slop = slop;
        let response = engine.search(&params).expect("phrase search");
        response["grouped_hits"]
            .as_array()
            .expect("groups")
            .iter()
            .map(|group| {
                group["group_key"][0]
                    .as_str()
                    .expect("group key")
                    .to_string()
            })
            .collect::<HashSet<_>>()
    };

    assert_eq!(
        ids(r#""black hole""#, "text", None),
        HashSet::from(["adjacent".to_string(), "duplicate".to_string()])
    );
    assert_eq!(
        ids(r#""black black""#, "text", None),
        HashSet::from(["duplicate".to_string()])
    );
    assert_eq!(
        ids(r#""hole black""#, "text", None),
        HashSet::from(["reversed".to_string()])
    );
    assert_eq!(
        ids(r#""black hole" deep"#, "text", None),
        HashSet::from(["adjacent".to_string()])
    );
    assert_eq!(
        ids(r#""black hole""#, "title", None),
        HashSet::from(["title".to_string()])
    );
    assert_eq!(
        ids(r#""black hole""#, "text", Some(1)),
        HashSet::from([
            "adjacent".to_string(),
            "gapped".to_string(),
            "duplicate".to_string(),
        ])
    );
    assert_eq!(
        ids(r#""black hole" -deep"#, "text", None),
        HashSet::from(["duplicate".to_string()])
    );
    assert_eq!(
        ids(r#"black -"deep space""#, "text", None),
        HashSet::from([
            "duplicate".to_string(),
            "gapped".to_string(),
            "reversed".to_string(),
        ])
    );
    assert_eq!(
        ids(r#""black hole" -"deep space""#, "text", None),
        HashSet::from(["duplicate".to_string()])
    );
    assert_eq!(
        ids(r#""black hole" -"deep hole""#, "text", Some(1)),
        HashSet::from(["adjacent".to_string(), "duplicate".to_string()])
    );
    assert_eq!(
        ids(r#"black -"black black""#, "text", None),
        HashSet::from([
            "adjacent".to_string(),
            "gapped".to_string(),
            "reversed".to_string(),
        ])
    );
    assert!(ids(r#"black -"black hole""#, "title", None).is_empty());
    let without_deep = HashSet::from(["duplicate".to_string(), "reversed".to_string()]);
    assert_eq!(ids("bla -deep", "text", None), without_deep);
    assert_eq!(ids("blak -deep", "text", None), without_deep);
    assert_eq!(ids("black missing -deep", "text", None), without_deep);
    assert!(ids("black -deap", "text", None).contains("adjacent"));
    assert!(ids("black -lecture", "title", None).is_empty());
    assert!(ids("black hole", "text", None).contains("gapped"));
    for query in ["black hole", "*", r#"-"black hole""#] {
        assert!(
            engine
                .search(&SearchParams {
                    q: query.to_string(),
                    phrase_slop: Some(1),
                    ..SearchParams::default()
                })
                .is_err()
        );
    }
    for query in ["-deep", "* -deep"] {
        assert_eq!(
            ids(query, "text", None),
            HashSet::from([
                "duplicate".to_string(),
                "reversed".to_string(),
                "title".to_string(),
            ])
        );
    }
    for query in [r#"-"black hole""#, r#"* -"black hole""#] {
        assert_eq!(
            ids(query, "text", None),
            HashSet::from([
                "gapped".to_string(),
                "reversed".to_string(),
                "title".to_string(),
            ])
        );
    }
}

#[test]
fn complete_positions_prefer_the_tightest_bounded_token_window() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"video_id":"wide","title":"Wide","text":"alpha x beta x x x x","youtuber_handle":"@one"}),
        json!({"video_id":"tight","title":"Tight","text":"alpha x x beta x alpha beta","youtuber_handle":"@one"}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build");
    let engine = RawSearchEngine::open(&index_path).expect("open");
    let mut query = grouped_query("alpha beta");
    query.query_by = Some("text".to_string());
    let response = engine.search(&query).expect("proximity search");
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "tight");

    let exact = proximity_multiplier(
        PositionalWindow {
            displacement: 1,
            max_offset: 1,
        },
        2,
        true,
    );
    let gapped = proximity_multiplier(
        PositionalWindow {
            displacement: 2,
            max_offset: 2,
        },
        2,
        true,
    );
    assert!(exact > gapped && exact <= 1.25);
    assert_eq!(
        proximity_multiplier(
            PositionalWindow {
                displacement: 11,
                max_offset: 11,
            },
            2,
            true,
        ),
        1.0
    );
}

#[test]
fn long_exact_phrase_ranks_before_the_same_tokens_out_of_order() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({"video_id":"shuffled","title":"Shuffled","text":"now dead is think i which css","youtuber_handle":"@t3dotgg"}),
        json!({"video_id":"ordered","title":"Ordered","text":"includes things like CSS which I think is dead now","youtuber_handle":"@t3dotgg"}),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build");
    let engine = RawSearchEngine::open(&index_path).expect("open");
    let mut query = grouped_query("Css which i think is dead now");
    query.query_by = Some("text".to_string());

    let response = engine.search(&query).expect("search");

    assert_eq!(response["grouped_hits"][0]["group_key"][0], "ordered");
    let snippet = response["grouped_hits"][0]["hits"][0]["highlights"][0]["snippet"]
        .as_str()
        .expect("snippet");
    assert_eq!(snippet.matches("<mark>").count(), 7);
    assert!(!snippet.contains("<mark>things</mark>"));
}

#[test]
fn grouped_totals_include_all_moments_beyond_the_return_window() {
    let (_root, engine) = test_engine();
    let mut query = grouped_query("search");
    query.query_by = Some("text".to_string());
    query.group_limit = Some(1);
    let response = engine.search(&query).expect("search");

    assert_eq!(response["found"], 1);
    assert_eq!(response["total_moments"], 2);
    assert_eq!(response["grouped_hits"][0]["found"], 2);
    assert_eq!(
        response["grouped_hits"][0]["hits"]
            .as_array()
            .expect("hits")
            .len(),
        1
    );
}

#[test]
fn match_provenance_preserves_the_full_u32_document_space() {
    assert_eq!(std::mem::size_of::<DocMatch>(), 12);
    // The extra four bytes are build-only spool coordinates. Query candidates
    // stay fixed-width while rank flags no longer steal bits from document IDs.
    assert_eq!(std::mem::size_of::<BuildPosting>(), 12);
    let mut transcript = DocMatch::new(u32::MAX, 1.0, true);
    assert_eq!(transcript.document(), u32::MAX);
    assert!(transcript.matches_text());
    assert_eq!(transcript.exact_tokens(), 1);
    transcript.set_exact_mask(0b1000);
    let mut fuzzy = DocMatch::new(u32::MAX, 2.0, true);
    fuzzy.set_exact_mask(0);
    let union = union_sorted(vec![transcript], vec![fuzzy]);
    assert_eq!(union[0].exact_tokens(), 1);
    assert_eq!(union[0].exact_mask(), 0b1000);
    assert!(union[0].all_retained_exact());

    let mut later_exact = transcript;
    later_exact.set_exact_mask(0b0100);
    let two_exact = intersect_sorted(vec![transcript], vec![later_exact]);
    assert_eq!(two_exact[0].exact_tokens(), 2);
    assert_eq!(two_exact[0].exact_mask(), 0b1100);
    assert!(two_exact[0].all_retained_exact());
    let one_exact = intersect_sorted(vec![transcript], vec![fuzzy]);
    assert_eq!(one_exact[0].exact_tokens(), 1);
    assert!(!one_exact[0].all_retained_exact());
}

#[test]
fn sorted_exclusion_union_deduplicates_without_reordering() {
    assert_eq!(
        union_sorted_ids(vec![1, 3, 7], vec![2, 3, 8]),
        [1, 2, 3, 7, 8]
    );
    assert_eq!(union_sorted_ids(Vec::new(), vec![4, 9]), [4, 9]);
}

#[test]
fn first_token_position_breaks_equal_relevance_ties() {
    let (_root, engine) = position_engine();
    let mut query = grouped_query("target");
    query.query_by = Some("text".to_string());
    let response = engine.search(&query).expect("position search");
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "early");
    assert!(token_position_multiplier(0, true) > token_position_multiplier(3, true));
    assert_eq!(token_position_multiplier(0, false), 1.0);
}

#[test]
fn moment_totals_use_the_same_twenty_second_coalescing_window_as_the_api() {
    let mut moments = MomentCounter::default();
    for start in [1_000, 21_000, 21_001, 41_001] {
        moments.record(start);
    }
    assert_eq!(moments.visible(), 2);
}

#[test]
fn title_only_matches_count_once_per_video() {
    let (_root, engine) = test_engine();
    let mut query = grouped_query("performance");
    query.group_limit = Some(1);
    let response = engine.search(&query).expect("search");

    assert_eq!(response["found"], 1);
    assert_eq!(response["total_moments"], 1);
    assert_eq!(response["grouped_hits"][0]["found"], 1);
}

#[test]
fn filters_and_date_sort_match_the_shared_contract() {
    let (_root, engine) = test_engine();
    let mut query = grouped_query("*");
    query.filter_by = Some("youtuber_handle:=@TWO".to_string());
    query.sort_by = Some("published_at_timestamp:desc".to_string());
    let response = engine.search(&query).expect("search");
    assert_eq!(response["found"], 1);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "b");
}

#[test]
fn unsupported_search_contracts_fail_closed() {
    let (_root, engine) = test_engine();
    for (field, value, expected) in [
        ("query_by", "text,description", "unsupported query_by field"),
        ("query_by", "text,text", "duplicate field"),
        (
            "filter_by",
            "published_at:=2026",
            "unsupported filter_by field",
        ),
        ("group_by", "youtuber_handle", "unsupported group_by field"),
        (
            "sort_by",
            "published_at:desc",
            "unsupported sort_by expression",
        ),
    ] {
        let mut query = grouped_query("search");
        match field {
            "query_by" => query.query_by = Some(value.to_string()),
            "filter_by" => query.filter_by = Some(value.to_string()),
            "group_by" => query.group_by = Some(value.to_string()),
            "sort_by" => query.sort_by = Some(value.to_string()),
            _ => unreachable!(),
        }
        let error = engine.search(&query).expect_err("invalid request");
        assert!(error.to_string().contains(expected), "error={error}");
    }

    let mut malformed_filter = grouped_query("search");
    malformed_filter.filter_by = Some("video_id:abc".to_string());
    assert!(engine.search(&malformed_filter).is_err());
    for filter in [
        "video_id:=abc && youtuber_handle:=@one",
        "video_id:=[abc,def]",
        "video_id:=abc:=def",
        "video_id:=`abc",
        "video_id:=abc`",
    ] {
        malformed_filter.filter_by = Some(filter.to_string());
        assert!(
            engine.search(&malformed_filter).is_err(),
            "accepted {filter}"
        );
    }

    let mut quoted_filter = grouped_query("*");
    quoted_filter.filter_by = Some("video_id:=`a`".to_string());
    assert_eq!(
        engine.search(&quoted_filter).expect("quoted filter")["found"],
        1
    );

    let mut conflicting_window = grouped_query("search");
    conflicting_window.per_page = Some(10);
    assert!(engine.search(&conflicting_window).is_err());

    let mut empty = grouped_query("  ");
    empty.limit = Some(10);
    assert!(engine.search(&empty).is_err());
}

#[test]
fn active_typesense_parameters_are_validated_and_supported() {
    let (_root, engine) = test_engine();
    let mut query = grouped_query("search");
    query.query_by_weights = Some("4,6".to_string());
    query.text_match_type = Some("sum_score".to_string());
    query.prioritize_exact_match = Some(true);
    query.prioritize_token_position = Some(true);
    query.typo_tokens_threshold = Some(5);
    query.drop_tokens_threshold = Some(1);
    query.drop_tokens_mode = Some("right_to_left".to_string());
    query.split_join_tokens = Some("always".to_string());
    query.highlight_fields = Some("text,title".to_string());
    query.include_fields = Some(ACTIVE_INCLUDE_FIELDS.to_string());
    assert_eq!(engine.search(&query).expect("active contract")["found"], 1);

    query.typo_tokens_threshold = Some(1);
    assert!(engine.search(&query).is_err());

    query.typo_tokens_threshold = Some(5);
    query.drop_tokens_threshold = Some(0);
    assert!(engine.search(&query).is_err());

    query.drop_tokens_threshold = Some(1);
    query.drop_tokens_mode = Some("left_to_right".to_string());
    assert!(engine.search(&query).is_err());
}

#[test]
fn typo_search_is_a_guarded_fallback() {
    let (_root, engine) = test_engine();
    let response = engine.search(&grouped_query("invertd")).expect("search");
    assert_eq!(response["found"], 1);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "a");
}

#[test]
fn prefix_transposition_and_two_edit_search_share_the_end_to_end_contract() {
    let (_root, engine) = test_engine();
    for query in ["relat", "relaitvity", "relativtyy"] {
        let response = engine.search(&grouped_query(query)).expect("search");
        assert_eq!(response["found"], 1, "query={query}");
        assert_eq!(
            response["grouped_hits"][0]["group_key"][0], "a",
            "query={query}"
        );
        assert!(
            response["grouped_hits"][0]["hits"][0]["highlights"]
                .as_array()
                .expect("highlights")
                .iter()
                .any(|highlight| highlight["snippet"]
                    .as_str()
                    .is_some_and(|snippet| snippet.contains("<mark>"))),
            "query={query}"
        );
    }
}

#[test]
fn multi_token_typo_and_last_token_prefix_preserve_strict_intersection() {
    let (_root, engine) = test_engine();
    for query in ["memory map", "memory maped"] {
        let response = engine.search(&grouped_query(query)).expect("search");
        assert_eq!(response["found"], 1, "query={query}");
        assert_eq!(
            response["grouped_hits"][0]["group_key"][0], "a",
            "query={query}"
        );
    }
}

#[test]
fn prefix_candidates_are_not_hidden_by_five_exact_results() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for index in 0..5 {
        writeln!(
            chunks,
            "{}",
            json!({
                "id": format!("exact-{index}"),
                "video_id": format!("exact-{index}"),
                "title": "Linux exploit",
                "text": "linux exploit",
                "youtuber_handle": "@security",
                "youtuber_name": "Security"
            })
        )
        .expect("write exact chunk");
    }
    writeln!(
        chunks,
        "{}",
        json!({
            "id": "prefix-only",
            "video_id": "prefix-only",
            "title": "Linux exploitation",
            "text": "linux exploitation",
            "youtuber_handle": "@security",
            "youtuber_name": "Security"
        })
    )
    .expect("write prefix chunk");
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");

    for query in ["exploit", "linux exploit"] {
        let response = engine.search(&grouped_query(query)).expect("search");
        assert_eq!(response["found"], 6, "query={query}");
        assert!(
            response["grouped_hits"]
                .as_array()
                .expect("groups")
                .iter()
                .any(|group| group["group_key"][0] == "prefix-only"),
            "query={query}"
        );
    }
}

#[test]
fn token_drop_prefers_the_longest_right_to_left_conjunction() {
    let (_root, engine) = token_drop_engine();
    let response = engine
        .search(&grouped_query("alpha beta missing"))
        .expect("token-drop search");

    assert_eq!(response["found"], 1);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "prefix-pair");
}

#[test]
fn sparse_fuzzy_conjunction_does_not_block_a_stronger_exact_token_drop() {
    let (_root, engine) = token_drop_engine();
    let response = engine
        .search(&grouped_query("battery gate incident"))
        .expect("quality-aware token-drop search");

    assert_eq!(response["grouped_hits"][0]["group_key"][0], "battery-pair");
}

#[test]
fn token_drop_switches_direction_only_after_prefixes_are_exhausted() {
    let (_root, engine) = token_drop_engine();
    let response = engine
        .search(&grouped_query("missing beta gamma"))
        .expect("token-drop search");

    assert_eq!(response["found"], 1);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "suffix-pair");
}

#[test]
fn token_drop_threshold_applies_the_active_filter() {
    let (_root, engine) = token_drop_engine();
    let mut query = grouped_query("alpha beta gamma");
    query.filter_by = Some("youtuber_handle:=@two".to_string());
    let response = engine.search(&query).expect("filtered token-drop search");

    assert_eq!(response["found"], 1);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "suffix-pair");
}

#[test]
fn an_exact_phrase_outside_the_filter_cannot_suppress_scoped_prefix_recall() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for document in [
        json!({
            "id": "global-phrase-1",
            "video_id": "global-phrase",
            "title": "Global phrase",
            "text": "alpha beta gamma delta",
            "youtuber_handle": "@one",
            "youtuber_name": "One"
        }),
        json!({
            "id": "scoped-prefix-1",
            "video_id": "scoped-prefix",
            "title": "Scoped prefix",
            "text": "alpha beta gamma deltas",
            "youtuber_handle": "@two",
            "youtuber_name": "Two"
        }),
    ] {
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    let mut query = grouped_query("alpha beta gamma delta");
    query.filter_by = Some("youtuber_handle:=@two".to_string());

    let response = engine.search(&query).expect("filtered prefix search");

    assert_eq!(response["found"], 1);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "scoped-prefix");
}

#[test]
fn typo_expansion_stays_conjunctive_inside_a_retained_subset() {
    let (_root, engine) = token_drop_engine();
    let response = engine
        .search(&grouped_query("alpha betta missing"))
        .expect("fuzzy token-drop search");

    assert!(response["found"].as_u64().is_some_and(|found| found >= 1));
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "prefix-pair");
}

#[test]
fn fully_exact_matches_rank_before_higher_frequency_typo_matches() {
    let (_root, engine) = token_drop_engine();
    let response = engine
        .search(&grouped_query("relativity explained"))
        .expect("exact-priority search");

    assert_eq!(response["found"], 2);
    assert_eq!(
        response["grouped_hits"][0]["group_key"][0],
        "exact-relativity"
    );
}

#[test]
fn rarest_first_exact_prefix_conjunction_matches_the_materialized_contract() {
    let (_root, engine) = token_drop_engine();
    let terms = vec!["relativity".to_string(), "explained".to_string()];
    let fields = query_fields(None, true).expect("query fields");
    let (fast, _) = engine
        .exact_prefix_conjunction(&terms, fields, true, None, &mut BlockCache::default())
        .expect("rarest-first conjunction");

    let mut materialized = engine
        .exact_term_matches(&terms, fields)
        .expect("materialized term matches");
    let (prefix, _) = engine
        .fuzzy_alternatives(&terms[1], fields, 0, None, true)
        .expect("last-token prefixes");
    let last = materialized.last_mut().expect("last term matches");
    *last = union_sorted(std::mem::take(last), prefix);
    let materialized = intersect_all_borrowed(&materialized);

    assert_eq!(fast.len(), materialized.len());
    for (fast, materialized) in fast.iter().zip(materialized) {
        assert_eq!(fast.document(), materialized.document());
        assert_eq!(fast.exact_mask(), materialized.exact_mask());
        assert_eq!(fast.matches_text(), materialized.matches_text());
        let tolerance = materialized.score.abs().max(1.0) * 1e-6;
        assert!(
            (fast.score - materialized.score).abs() <= tolerance,
            "fast={} materialized={} tolerance={tolerance}",
            fast.score,
            materialized.score
        );
    }
}

#[test]
fn document_local_proximity_matches_postings_at_the_direct_path_boundary() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for index in 0..129 {
        let document = json!({
            "id": format!("video-{index}-1"),
            "video_id": format!("video-{index}"),
            "title": if index % 2 == 0 { "alpha beta" } else { "beta alpha" },
            "text": if index % 3 == 0 { "alpha beta alpha" } else { "beta then alpha" },
            "youtuber_handle": "@boundary",
            "youtuber_name": "Boundary"
        });
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    let terms = ordered_tokens("alpha beta");
    let fields = query_fields(Some("text,title"), true).expect("query fields");
    let (candidates, _) = engine
        .exact_prefix_conjunction(&terms, fields, false, None, &mut BlockCache::default())
        .expect("candidates");
    assert_eq!(candidates.len(), 129);

    for target_count in [127, 128, 129] {
        let mut direct = candidates[..target_count].to_vec();
        let mut postings = direct.clone();
        engine
            .apply_exact_proximity_from_documents(
                &terms,
                fields,
                &mut direct,
                &mut BlockCache::default(),
            )
            .expect("document-local proximity");
        engine
            .apply_exact_proximity_from_postings(&terms, fields, &mut postings)
            .expect("posting proximity");

        for (direct, postings) in direct.iter().zip(postings) {
            assert_eq!(direct.document(), postings.document());
            assert_eq!(direct.exact_mask(), postings.exact_mask());
            assert_eq!(direct.matches_text(), postings.matches_text());
            assert_eq!(direct.ordered_phrase(), postings.ordered_phrase());
            let tolerance = postings.score.abs().max(1.0) * 1e-6;
            assert!(
                (direct.score - postings.score).abs() <= tolerance,
                "count={target_count} direct={} postings={} tolerance={tolerance}",
                direct.score,
                postings.score
            );
        }
    }
}

#[test]
fn targeted_scoring_matches_materialized_scoring_above_the_direct_boundary() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for index in 0..130 {
        writeln!(
            chunks,
            "{}",
            json!({
                "id": format!("candidate-{index}-1"),
                "video_id": format!("candidate-{index}"),
                "title": if index % 2 == 0 { "rare common" } else { "rare" },
                "text": if index % 3 == 0 { "rare common common" } else { "rare common" },
                "youtuber_handle": "@boundary",
                "youtuber_name": "Boundary"
            })
        )
        .expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    let terms = ordered_tokens("rare common");
    let fields = query_fields(Some("text,title"), true).expect("query fields");
    let (fast, _) = engine
        .exact_prefix_conjunction(&terms, fields, false, None, &mut BlockCache::default())
        .expect("targeted conjunction");
    let materialized = intersect_all_borrowed(
        &engine
            .exact_term_matches(&terms, fields)
            .expect("materialized matches"),
    );

    assert_eq!(fast.len(), 130);
    assert_eq!(fast.len(), materialized.len());
    for (fast, materialized) in fast.iter().zip(materialized) {
        assert_eq!(fast.document(), materialized.document());
        let tolerance = materialized.score.abs().max(1.0) * 1e-6;
        assert!(
            (fast.score - materialized.score).abs() <= tolerance,
            "fast={} materialized={} tolerance={tolerance}",
            fast.score,
            materialized.score
        );
    }
}

#[test]
fn earlier_exact_query_tokens_break_equal_exact_count_ties() {
    let (_root, engine) = token_drop_engine();
    let response = engine
        .search(&grouped_query("planet orbit missing"))
        .expect("query-order exact-priority search");

    assert_eq!(response["found"], 2);
    assert_eq!(response["grouped_hits"][0]["group_key"][0], "early-exact");
}

#[test]
fn batched_document_validation_matches_materialized_multi_term_scoring() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for index in 0..120 {
        writeln!(
            chunks,
            "{}",
            json!({
                "id": format!("batched-{index}-1"),
                "video_id": format!("batched-{index}"),
                "title": "rare alpha beta gamma",
                "text": if index % 2 == 0 {
                    "rare alpha beta gamma alpha"
                } else {
                    "gamma rare beta alpha"
                },
                "youtuber_handle": "@batched",
                "youtuber_name": "Batched"
            })
        )
        .expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    let terms = ordered_tokens("rare alpha beta gamma");
    let fields = query_fields(Some("text,title"), true).expect("query fields");

    let (batched, _) = engine
        .exact_prefix_conjunction(&terms, fields, false, None, &mut BlockCache::default())
        .expect("batched conjunction");
    let materialized = intersect_all_borrowed(
        &engine
            .exact_term_matches(&terms, fields)
            .expect("materialized matches"),
    );

    assert_eq!(batched.len(), materialized.len());
    for (batched, materialized) in batched.iter().zip(materialized) {
        assert_eq!(batched.document(), materialized.document());
        assert_eq!(batched.exact_mask(), materialized.exact_mask());
        assert_eq!(
            batched.all_retained_exact(),
            materialized.all_retained_exact()
        );
        assert_eq!(batched.matches_text(), materialized.matches_text());
        let tolerance = materialized.score.abs().max(1.0) * 1e-6;
        assert!(
            (batched.score - materialized.score).abs() <= tolerance,
            "batched={} materialized={} tolerance={tolerance}",
            batched.score,
            materialized.score
        );
    }
}

#[test]
fn batched_document_validation_matches_materialized_typo_alternatives() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for index in 0..127 {
        writeln!(
            chunks,
            "{}",
            json!({
                "id": format!("fuzzy-{index}-1"),
                "video_id": format!("fuzzy-{index}"),
                "title": "rare alpha beta gamma",
                "text": "rare alpha beta gamma",
                "youtuber_handle": "@fuzzy",
                "youtuber_name": "Fuzzy"
            })
        )
        .expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    let terms = ordered_tokens("raer alpha beta gamma");
    let fields = query_fields(Some("text,title"), true).expect("query fields");

    let (batched, _) = engine
        .fuzzy_prefix_conjunction(&terms, fields, false, 1, None, &mut BlockCache::default())
        .expect("batched fuzzy conjunction");
    let mut materialized = Vec::with_capacity(terms.len());
    for (index, term) in terms.iter().enumerate() {
        let exact_mask = 1 << (3 - index.min(3));
        let exact_fields = QueryFields {
            exact_mask,
            ..fields
        };
        let exact = engine.matches_for_term(term, exact_fields).expect("exact");
        let (fuzzy, _) = engine
            .fuzzy_alternatives(term, fields, 1, Some(1), false)
            .expect("fuzzy alternatives");
        materialized.push(union_sorted(exact, fuzzy));
    }
    let materialized = intersect_all_borrowed(&materialized);

    assert_eq!(batched.len(), 127);
    assert_eq!(batched.len(), materialized.len());
    for (batched, materialized) in batched.iter().zip(materialized) {
        assert_eq!(batched.document(), materialized.document());
        assert_eq!(batched.exact_mask(), materialized.exact_mask());
        assert_eq!(
            batched.all_retained_exact(),
            materialized.all_retained_exact()
        );
        assert!(!batched.all_retained_exact());
        let tolerance = materialized.score.abs().max(1.0) * 1e-6;
        assert!((batched.score - materialized.score).abs() <= tolerance);
    }
}

#[test]
fn fuzzy_later_token_does_not_stop_five_token_drop_progression() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for index in 0..3 {
        writeln!(
            chunks,
            "{}",
            json!({
                "id": format!("drop-{index}-1"),
                "video_id": format!("drop-{index}"),
                "title": "Drop progression",
                "text": if index == 0 {
                    "alpha beta gamma memory delta"
                } else {
                    "alpha beta gamma"
                },
                "youtuber_handle": "@drop",
                "youtuber_name": "Drop"
            })
        )
        .expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    let mut query = grouped_query("alpha beta gamma memroy delta");
    query.query_by = Some("text".to_string());

    let response = engine.search(&query).expect("search");

    assert_eq!(response["found"], 3);
}

#[test]
fn blackhole_split_is_consistent_globally_and_per_creator() {
    let (_root, engine) = blackhole_engine();
    let global = engine.search(&grouped_query("blackhole")).expect("global");
    assert_eq!(global["found"], 3);
    let global_ids = global["grouped_hits"]
        .as_array()
        .expect("groups")
        .iter()
        .map(|group| group["group_key"][0].as_str().expect("video id"))
        .collect::<HashSet<_>>();
    assert_eq!(
        global_ids,
        HashSet::from(["literal", "veritasium", "vsauce"])
    );
    for (handle, expected) in [("@VERITASIUM", "veritasium"), ("@vsauce", "vsauce")] {
        let mut query = grouped_query("blackhole");
        query.filter_by = Some(format!("youtuber_handle:={handle}"));
        let response = engine.search(&query).expect("creator search");
        assert_eq!(response["found"], 1);
        assert_eq!(response["grouped_hits"][0]["group_key"][0], expected);
        let snippet = response["grouped_hits"][0]["hits"][0]["highlights"][0]["snippet"]
            .as_str()
            .unwrap_or_else(|| panic!("split highlight response={response}"));
        assert!(snippet.contains("<mark>"), "response={response}");
    }
}

#[test]
fn typo_corrected_compact_split_recovers_black_hole_results() {
    let (_root, engine) = blackhole_engine();
    for query in ["blackhloe", "blakhole", "blackhle"] {
        let response = engine.search(&grouped_query(query)).expect("search");
        let ids = response["grouped_hits"]
            .as_array()
            .expect("groups")
            .iter()
            .map(|group| group["group_key"][0].as_str().expect("video id"))
            .collect::<HashSet<_>>();
        assert!(ids.contains("veritasium"), "query={query}");
        assert!(ids.contains("vsauce"), "query={query}");
    }
}

#[test]
fn typo_corrected_split_preserves_corrected_query_ranking() {
    let (_root, engine) = weighted_blackhole_engine();
    let exact = engine.search(&grouped_query("blackhole")).expect("exact");
    let expected = exact["grouped_hits"][0]["group_key"][0]
        .as_str()
        .expect("exact top result");
    for query in ["blackhloe", "blakhole", "blackhle"] {
        let response = engine.search(&grouped_query(query)).expect("search");
        assert_eq!(
            response["grouped_hits"][0]["group_key"][0], expected,
            "query={query}"
        );
    }
}

#[test]
fn relevance_and_explicit_date_sorts_are_distinct_and_deterministic() {
    let (_root, engine) = blackhole_engine();
    let relevant = engine
        .search(&grouped_query("blackhole"))
        .expect("relevance");
    assert_eq!(relevant["grouped_hits"][0]["group_key"][0], "veritasium");

    let mut newest = grouped_query("blackhole");
    newest.sort_by = Some("published_at_timestamp:desc".to_string());
    let newest = engine.search(&newest).expect("newest");
    assert_eq!(newest["grouped_hits"][0]["group_key"][0], "literal");

    let mut oldest = grouped_query("blackhole");
    oldest.sort_by = Some("published_at_timestamp:asc".to_string());
    let oldest = engine.search(&oldest).expect("oldest");
    assert_eq!(oldest["grouped_hits"][0]["group_key"][0], "veritasium");
}

#[test]
fn engagement_sorts_are_explicit_bounded_and_return_the_source_counts() {
    let (_root, engine) = engagement_engine();

    for sort in [
        "engagement_score:desc",
        "view_count:desc",
        "like_count:desc",
        "comment_count:desc",
    ] {
        let mut query = grouped_query("systems explained");
        query.sort_by = Some(sort.to_string());
        let response = engine.search(&query).expect("engagement search");
        assert_eq!(response["grouped_hits"][0]["group_key"][0], "large");
        assert_eq!(
            response["grouped_hits"][0]["hits"][0]["document"]["view_count"],
            1_000_000
        );
        assert_eq!(
            response["grouped_hits"][0]["hits"][0]["document"]["like_count"],
            100_000
        );
        assert_eq!(
            response["grouped_hits"][0]["hits"][0]["document"]["comment_count"],
            10_000
        );
    }

    let relevant = engine
        .search(&grouped_query("orbital mechanics"))
        .expect("relevance search");
    assert_eq!(relevant["grouped_hits"][0]["group_key"][0], "exact");
}

#[test]
fn engagement_normalization_is_bounded_and_handles_missing_metrics() {
    let manifest = Manifest {
        schema_version: 5,
        documents: 0,
        videos: 0,
        channels: 0,
        terms: 0,
        blocks: 0,
        average_document_tokens: 0.0,
        max_view_count: 1_000,
        max_like_count: 100,
        max_comment_count: 10,
        files: BTreeMap::new(),
    };
    let video = VideoMeta {
        id: "video".to_string(),
        youtuber_id: "creator".to_string(),
        handle: "@creator".to_string(),
        title: "Video".to_string(),
        published_at_timestamp: 0,
        view_count: 1_000,
        like_count: 100,
        comment_count: 10,
        documents: Vec::new(),
    };
    assert!((normalized_engagement(&video, &manifest) - 1.0).abs() < f32::EPSILON);
    assert_eq!(normalized_log(0, 0), 0.0);
    assert_eq!(blended_relevance(10.0, &video, &manifest), 12.0);
}

#[test]
fn pagination_and_query_fields_preserve_the_search_contract() {
    let (_root, engine) = blackhole_engine();
    let mut first = grouped_query("blackhole");
    first.limit = None;
    first.offset = None;
    first.per_page = Some(1);
    first.page = Some(1);
    let mut second = first.clone();
    second.page = Some(2);
    let first = engine.search(&first).expect("first page");
    let second = engine.search(&second).expect("second page");
    assert_eq!(first["found"], 3);
    assert_eq!(second["found"], 3);
    assert_eq!(first["total_moments"], 3);
    assert_eq!(second["total_moments"], 3);
    assert_ne!(
        first["grouped_hits"][0]["group_key"][0],
        second["grouped_hits"][0]["group_key"][0]
    );

    let mut title_only = grouped_query("blackhole");
    title_only.query_by = Some("title".to_string());
    let title_only = engine.search(&title_only).expect("title only");
    assert_eq!(title_only["found"], 2);

    let mut ungrouped = grouped_query("blackhole");
    ungrouped.group_by = None;
    let ungrouped = engine.search(&ungrouped).expect("ungrouped");
    assert_eq!(ungrouped["found"], 3);
    assert_eq!(ungrouped["hits"].as_array().expect("hits").len(), 3);
}

#[test]
fn wildcard_video_filter_group_limit_and_channel_search_are_supported() {
    let (_root, engine) = blackhole_engine();
    let mut wildcard = grouped_query("*");
    wildcard.filter_by = Some("video_id:=vsauce".to_string());
    wildcard.group_limit = Some(1);
    let wildcard = engine.search(&wildcard).expect("wildcard");
    assert_eq!(wildcard["found"], 1);
    assert_eq!(wildcard["grouped_hits"][0]["group_key"][0], "vsauce");
    assert_eq!(
        wildcard["grouped_hits"][0]["hits"]
            .as_array()
            .expect("hits")
            .len(),
        1
    );

    let channels = engine
        .search_channels(&SearchParams {
            q: "sauce".to_string(),
            query_by: Some("name,handle".to_string()),
            filter_by: None,
            group_by: None,
            group_limit: None,
            per_page: Some(1),
            page: Some(1),
            offset: None,
            limit: None,
            sort_by: None,
            use_cache: None,
            cache_ttl: None,
            ..SearchParams::default()
        })
        .expect("channel search");
    assert_eq!(channels["found"], 1);
    assert_eq!(channels["hits"][0]["document"]["handle"], "@vsauce");

    let mut invalid_channel = SearchParams {
        q: "sauce".to_string(),
        query_by: Some("text,title".to_string()),
        ..SearchParams::default()
    };
    assert!(engine.search_channels(&invalid_channel).is_err());
    invalid_channel.query_by = Some("name,handle".to_string());
    invalid_channel.sort_by = Some("name:asc".to_string());
    assert!(engine.search_channels(&invalid_channel).is_err());
}

#[test]
fn empty_corpus_builds_and_searches_without_special_cases() {
    let root = TempDir::new().expect("temp dir");
    let chunks = root.path().join("chunks.jsonl");
    let channels = root.path().join("channels.jsonl");
    File::create(&chunks).expect("chunks");
    File::create(&channels).expect("channels");
    let index = root.path().join("index");
    let report = build_index(&chunks, &channels, &index).expect("build empty index");
    assert_eq!(report.documents, 0);
    assert_eq!(report.videos, 0);
    let engine = RawSearchEngine::open(&index).expect("open empty index");
    assert_eq!(
        engine
            .search(&grouped_query("blackhole"))
            .expect("empty search")["found"],
        0
    );
    assert_eq!(
        engine.search(&grouped_query("*")).expect("empty wildcard")["found"],
        0
    );
}

#[test]
fn cloned_engines_return_stable_results_under_concurrency() {
    let (_root, engine) = blackhole_engine();
    let expected = engine
        .search(&grouped_query("blackhole"))
        .expect("expected");
    let workers = (0..8)
        .map(|_| {
            let engine = engine.clone();
            let expected = expected.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    assert_eq!(
                        engine.search(&grouped_query("blackhole")).expect("search"),
                        expected
                    );
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("worker");
    }
}

#[test]
fn non_contiguous_video_input_preserves_group_membership() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for video in ["a", "b", "a"] {
        writeln!(
            chunks,
            "{}",
            json!({"video_id":video,"title":"t","text":"x","youtuber_handle":"@one"})
        )
        .expect("write");
    }
    File::create(&channels_path).expect("channels");
    let index_path = root.path().join("index");
    build_index(&chunks_path, &channels_path, &index_path).expect("build");
    let engine = RawSearchEngine::open(&index_path).expect("open");
    let response = engine.search(&grouped_query("x")).expect("search");
    assert_eq!(response["found"], 2);
    let a_group = response["grouped_hits"]
        .as_array()
        .expect("groups")
        .iter()
        .find(|group| group["group_key"][0] == "a")
        .expect("a group");
    assert_eq!(a_group["hits"].as_array().expect("hits").len(), 2);
}

#[test]
fn typo_distance_uses_typesense_length_gates() {
    assert_eq!(bounded_typo_distance("abc"), 0);
    assert_eq!(bounded_typo_distance("abcd"), 1);
    assert_eq!(bounded_typo_distance("abcdef"), 1);
    assert_eq!(bounded_typo_distance("abcdefg"), 2);
}

#[test]
fn corrupt_file_magic_is_rejected_before_search() {
    let (root, engine) = test_engine();
    drop(engine);
    let mut lexicon = OpenOptions::new()
        .write(true)
        .open(root.path().join("index").join(LEXICON_FILE))
        .expect("lexicon");
    lexicon.write_all(b"BROKEN!!").expect("corrupt magic");
    let error = match RawSearchEngine::open(&root.path().join("index")) {
        Ok(_) => panic!("corrupt index must fail"),
        Err(error) => error,
    };
    assert!(matches!(error, Error::Corrupt(_)));
}

#[test]
fn truncated_fuzzy_index_is_rejected_at_open() {
    let (root, engine) = test_engine();
    drop(engine);
    let fuzzy_path = root.path().join("index").join(crate::fuzzy::FUZZY_FILE);
    OpenOptions::new()
        .write(true)
        .open(&fuzzy_path)
        .expect("fuzzy index")
        .set_len((HEADER_BYTES + 1) as u64)
        .expect("truncate fuzzy index");
    assert!(matches!(
        RawSearchEngine::open(&root.path().join("index")),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn truncated_postings_are_rejected_at_open() {
    let (root, engine) = test_engine();
    drop(engine);
    let postings_path = root.path().join("index").join(POSTINGS_FILE);
    OpenOptions::new()
        .write(true)
        .open(&postings_path)
        .expect("postings")
        .set_len(HEADER_BYTES as u64)
        .expect("truncate postings");
    assert!(matches!(
        RawSearchEngine::open(&root.path().join("index")),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn truncated_positions_are_rejected_at_open() {
    let (root, engine) = test_engine();
    drop(engine);
    let positions_path = root.path().join("index").join(POSITIONS_FILE);
    OpenOptions::new()
        .write(true)
        .open(&positions_path)
        .expect("positions")
        .set_len(HEADER_BYTES as u64)
        .expect("truncate positions");
    assert!(matches!(
        RawSearchEngine::open(&root.path().join("index")),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn truncated_full_positions_are_rejected_at_open() {
    let (root, engine) = test_engine();
    drop(engine);
    let positions_path = root.path().join("index").join(FULL_POSITIONS_FILE);
    OpenOptions::new()
        .write(true)
        .open(&positions_path)
        .expect("full positions")
        .set_len(HEADER_BYTES as u64)
        .expect("truncate full positions");
    assert!(matches!(
        RawSearchEngine::open(&root.path().join("index")),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn same_length_generation_corruption_is_rejected_at_open() {
    let (root, engine) = test_engine();
    drop(engine);
    let postings_path = root.path().join("index").join(POSTINGS_FILE);
    let mut postings = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&postings_path)
        .expect("postings");
    postings
        .seek(SeekFrom::Start(HEADER_BYTES as u64))
        .expect("seek posting");
    let mut byte = [0_u8; 1];
    postings.read_exact(&mut byte).expect("read posting");
    byte[0] ^= 0x01;
    postings
        .seek(SeekFrom::Start(HEADER_BYTES as u64))
        .expect("seek posting again");
    postings.write_all(&byte).expect("corrupt posting");
    postings.flush().expect("flush corruption");
    assert!(matches!(
        RawSearchEngine::open(&root.path().join("index")),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn truncated_document_blocks_are_rejected_at_open() {
    let (root, engine) = test_engine();
    drop(engine);
    let documents_path = root.path().join("index").join(DOCUMENTS_FILE);
    OpenOptions::new()
        .write(true)
        .open(&documents_path)
        .expect("documents")
        .set_len(HEADER_BYTES as u64)
        .expect("truncate documents");
    assert!(matches!(
        RawSearchEngine::open(&root.path().join("index")),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn schema_five_reader_is_a_response_compatible_rollback() {
    let (root, engine) = test_engine();
    let expected = ["memory", "bun", "*"]
        .into_iter()
        .map(|query| {
            (
                query,
                engine
                    .search(&grouped_query(query))
                    .expect("schema 6 search"),
            )
        })
        .collect::<Vec<_>>();
    drop(engine);

    let index_path = root.path().join("index");
    let manifest_path = index_path.join(MANIFEST_FILE);
    let mut manifest: Value =
        serde_json::from_reader(File::open(&manifest_path).expect("manifest"))
            .expect("manifest JSON");
    manifest["schema_version"] = json!(5);
    manifest
        .as_object_mut()
        .expect("manifest object")
        .remove("files");
    serde_json::to_writer_pretty(
        File::create(&manifest_path).expect("rewrite manifest"),
        &manifest,
    )
    .expect("write schema 5 manifest");
    fs::remove_file(index_path.join(FULL_POSITIONS_FILE)).expect("remove v6 sidecar");

    let rollback = RawSearchEngine::open(&index_path).expect("open schema 5 rollback");
    for (query, expected) in expected {
        assert_eq!(
            rollback
                .search(&grouped_query(query))
                .expect("schema 5 search"),
            expected,
            "schema 5 response changed for {query}"
        );
    }
    assert!(matches!(
        rollback.search(&grouped_query(r#""memory mapped""#)),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        rollback.search(&grouped_query(r#"bun -"memory mapped""#)),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(
        rollback
            .search(&grouped_query(r#"bun -"memory""#))
            .expect("single-token negative quote"),
        rollback
            .search(&grouped_query("bun -memory"))
            .expect("negative term")
    );
}

#[test]
fn manifest_channel_count_mismatch_is_rejected_at_open() {
    let (root, engine) = test_engine();
    drop(engine);
    let manifest_path = root.path().join("index").join(MANIFEST_FILE);
    let mut manifest: Value =
        serde_json::from_reader(File::open(&manifest_path).expect("manifest"))
            .expect("manifest JSON");
    manifest["channels"] = json!(99);
    serde_json::to_writer(
        File::create(&manifest_path).expect("rewrite manifest"),
        &manifest,
    )
    .expect("write manifest");
    assert!(matches!(
        RawSearchEngine::open(&root.path().join("index")),
        Err(Error::Corrupt(_))
    ));
}

#[test]
fn completed_index_is_never_overwritten() {
    let (root, engine) = test_engine();
    drop(engine);
    let error = build_index(
        &root.path().join("chunks.jsonl"),
        &root.path().join("channels.jsonl"),
        &root.path().join("index"),
    )
    .expect_err("completed index must not be overwritten");
    assert!(matches!(error, Error::ExistingIndex(_)));
}

#[test]
fn stable_slice_is_nested_and_input_order_independent() {
    let root = TempDir::new().expect("temp dir");
    let input = root.path().join("input.jsonl");
    let small = root.path().join("small.jsonl");
    let large = root.path().join("large.jsonl");
    let mut source = File::create(&input).expect("input");
    for video in ["c", "a", "d", "b", "a"] {
        writeln!(
            source,
            "{}",
            json!({"video_id":video,"title":"t","text":"x","youtuber_handle":"@one"})
        )
        .expect("write");
    }
    let small_report = slice_corpus(&input, &small, 2).expect("small slice");
    let large_report = slice_corpus(&input, &large, 3).expect("large slice");
    assert_eq!(small_report.videos, 2);
    assert_eq!(large_report.videos, 3);
    let small_ids = read_video_ids(&small);
    let large_ids = read_video_ids(&large);
    assert!(small_ids.is_subset(&large_ids));
}

fn read_video_ids(path: &Path) -> HashSet<String> {
    BufReader::new(File::open(path).expect("slice"))
        .lines()
        .map(|line| {
            serde_json::from_str::<IndexDocument>(&line.expect("line"))
                .expect("document")
                .video_id
        })
        .collect()
}
