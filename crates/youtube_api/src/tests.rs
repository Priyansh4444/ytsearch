use super::*;
use serde_json::json;

#[test]
fn browser_origins_allow_only_configured_and_local_development() {
    let configured = vec!["https://search.pronsh.dev".to_string()];

    for allowed in [
        "https://search.pronsh.dev",
        "http://localhost:5173",
        "http://127.0.0.1:3000",
        "http://[::1]:8787",
    ] {
        assert!(browser_origin_allowed(
            &HeaderValue::from_str(allowed).unwrap(),
            &configured
        ));
    }
    for rejected in [
        "https://ytsearch-755.pages.dev",
        "https://search.pronsh.dev.evil.example",
        "https://evil.example",
        "null",
    ] {
        assert!(!browser_origin_allowed(
            &HeaderValue::from_str(rejected).unwrap(),
            &configured
        ));
    }
}

fn video_results_from_hits(
    hits: &[SearchHit],
    sort: SearchSort,
    query: &str,
) -> Vec<SearchVideoResult> {
    video_results_from_hits_with_policy(hits.to_vec(), sort, query, RelevanceOrdering::ApiHeuristic)
}

fn test_document(id: &str, video_id: &str, text: &str) -> Value {
    json!({
        "id": id,
        "video_id": video_id,
        "title": "Test video",
        "text": text,
        "youtuber_handle": "@test",
        "youtuber_name": "Test",
        "view_count": 1_000_000,
        "like_count": "100000",
        "comment_count": 10000.0,
        "t_start_ms": 1000,
        "t_end_ms": 2000
    })
}

fn test_hit(video_id: &str, title: &str, text: &str) -> SearchHit {
    let document = json!({
        "id": format!("{video_id}-1"),
        "video_id": video_id,
        "title": title,
        "text": text,
        "youtuber_handle": "@test",
        "youtuber_name": "Test",
        "t_start_ms": 1000,
        "t_end_ms": 2000,
        "published_at": "2026-01-01T00:00:00Z"
    });
    SearchHit::from_backend_hit(&document, None).expect("test hit")
}

#[test]
fn parse_hits_flattens_raw_grouped_hits() {
    let raw = json!({
        "grouped_hits": [
            {
                "group_key": ["video-a"],
                "found": 4,
                "hits": [
                    {
                        "document": test_document("a-1", "video-a", "first"),
                        "highlights": [{"field": "text", "snippet": "<mark>first</mark>"}]
                    }
                ]
            },
            {
                "group_key": ["video-b"],
                "hits": [
                    {
                        "document": test_document("b-1", "video-b", "second"),
                        "highlights": [{"field": "text", "snippet": "<mark>second</mark>"}]
                    }
                ]
            }
        ]
    });

    let hits = parse_hits(&raw).expect("grouped hits parse");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].video_id, "video-a");
    assert_eq!(hits[1].video_id, "video-b");
    assert_eq!(hits[0].view_count, 1_000_000);
    assert_eq!(hits[0].like_count, 100_000);
    assert_eq!(hits[0].comment_count, 10_000);
    let videos = video_results_from_hits_with_policy(
        hits,
        SearchSort::Relevance,
        "first",
        RelevanceOrdering::Backend,
    );
    assert_eq!(videos[0].match_count, 4);
}

#[test]
fn grouped_match_count_survives_nearby_moment_coalescing() {
    let raw = json!({
        "grouped_hits": [{
            "group_key": ["video-a"],
            "found": 3,
            "hits": [
                {"document": test_document("a-1", "video-a", "first")},
                {"document": test_document("a-2", "video-a", "second")},
                {"document": test_document("a-3", "video-a", "third")}
            ]
        }]
    });
    let mut hits = parse_hits(&raw).expect("grouped hits parse");
    hits[0].t_start_ms = 0;
    hits[1].t_start_ms = 10_000;
    hits[2].t_start_ms = 20_000;

    let merged = merge_nearby_transcript_hits(hits);
    let videos = video_results_from_hits_with_policy(
        merged,
        SearchSort::Relevance,
        "first",
        RelevanceOrdering::Backend,
    );

    assert_eq!(videos[0].hits.len(), 1);
    assert_eq!(videos[0].match_count, 3);
}

#[test]
fn absent_optional_search_fields_are_omitted_from_json() {
    let mut hit = test_hit("video-a", "Result", "matching moment");
    hit.prev_text = None;
    hit.next_text = None;
    hit.text_highlight = None;
    hit.title_highlight = None;
    hit.thumbnail = None;
    hit.published_at = None;
    hit.timestamp = None;
    let hit_json = serde_json::to_value(&hit).expect("serialize hit");
    for field in [
        "prev_text",
        "next_text",
        "text_highlight",
        "title_highlight",
        "thumbnail",
        "published_at",
        "timestamp",
    ] {
        assert!(
            hit_json.get(field).is_none(),
            "unexpected null field {field}"
        );
    }

    let mut video = SearchVideoResult::from_hit(&hit);
    video.best_timestamp = None;
    let video_json = serde_json::to_value(video).expect("serialize video");
    for field in [
        "title_highlight",
        "thumbnail",
        "published_at",
        "best_timestamp",
    ] {
        assert!(
            video_json.get(field).is_none(),
            "unexpected null field {field}"
        );
    }
}

#[test]
fn engagement_sorts_are_gated_to_the_experimental_backend() {
    for sort in [
        SearchSort::RelevanceEngagement,
        SearchSort::ViewsDesc,
        SearchSort::LikesDesc,
        SearchSort::CommentsDesc,
    ] {
        assert!(validate_search_sort(sort, true).is_ok());
        assert!(matches!(
            validate_search_sort(sort, false),
            Err(ApiError::BadRequest(_))
        ));
    }
    for sort in [
        SearchSort::Relevance,
        SearchSort::PublishedDesc,
        SearchSort::PublishedAsc,
    ] {
        assert!(validate_search_sort(sort, false).is_ok());
    }
}

#[test]
fn engagement_results_preserve_backend_video_order_and_counts() {
    let mut popular = test_hit("popular", "Popular", "matching moment");
    popular.view_count = 9_000_000;
    popular.like_count = 700_000;
    popular.comment_count = 40_000;
    let mut niche = test_hit("niche", "Niche", "matching moment");
    niche.view_count = 1_000;
    niche.like_count = 100;
    niche.comment_count = 10;

    let videos = video_results_from_hits(
        &[popular, niche],
        SearchSort::RelevanceEngagement,
        "matching moment",
    );

    assert_eq!(videos[0].video_id, "popular");
    assert_eq!(videos[0].view_count, 9_000_000);
    assert_eq!(videos[0].like_count, 700_000);
    assert_eq!(videos[0].comment_count, 40_000);
}

#[test]
fn one_character_queries_are_low_signal() {
    assert!(is_low_signal_query("I"));
    assert!(is_low_signal_query("?"));
    assert!(!is_low_signal_query("fable"));
}

#[tokio::test]
async fn health_response_is_successful_and_not_cached() {
    let response = health().await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static(CACHE_NO_STORE))
    );
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("application/json"))
    );
}

#[test]
fn api_errors_use_stable_http_statuses_and_json_content_type() {
    let cases = [
        (ApiError::EmptyQuery, StatusCode::BAD_REQUEST),
        (ApiError::InvalidHandle, StatusCode::BAD_REQUEST),
        (ApiError::Unauthorized, StatusCode::UNAUTHORIZED),
        (ApiError::RateLimited(10), StatusCode::TOO_MANY_REQUESTS),
        (
            ApiError::BadRequest("bad input".to_string()),
            StatusCode::BAD_REQUEST,
        ),
    ];

    for (error, expected_status) in cases {
        let response = error.into_response();
        assert_eq!(response.status(), expected_status);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/json"))
        );
    }
}

#[test]
fn search_page_slicing_reports_next_page_without_leaking_extra_result() {
    let videos: Vec<SearchVideoResult> = (0..4)
        .map(|index| {
            SearchVideoResult::from_hit(&test_hit(
                &format!("video-{index}"),
                "Result",
                "matching moment",
            ))
        })
        .collect();

    let (first, has_next_page) = page_of_videos(videos.clone(), 0, 3);
    assert_eq!(first.len(), 3);
    assert!(has_next_page);

    let (second, has_next_page) = page_of_videos(videos, 3, 3);
    assert_eq!(second.len(), 1);
    assert!(!has_next_page);
}

#[test]
fn grouped_pagination_uses_and_removes_a_lookahead_group() {
    let mut result = json!({
        "grouped_hits": [{"group_key": ["a"]}, {"group_key": ["b"]}]
    });
    assert!(truncate_grouped_lookahead(&mut result, 1));
    assert_eq!(array_len(&result, "grouped_hits"), 1);

    assert!(!truncate_grouped_lookahead(&mut result, 1));
    assert_eq!(array_len(&result, "grouped_hits"), 1);
}

#[test]
fn ungrouped_totals_preserve_the_matching_moment_count() {
    let totals = json!({ "found": 35_383, "out_of": 7_129_139 });
    assert_eq!(raw_u64(&totals, "found"), Some(35_383));
    assert_eq!(raw_u64(&totals, "out_of"), Some(7_129_139));
}

#[test]
fn raw_grouped_totals_preserve_all_matching_moments() {
    let totals = json!({
        "found": 15,
        "total_moments": 41,
        "grouped_hits": [{ "found": 5, "hits": [{}, {}, {}] }]
    });

    assert_eq!(raw_u64(&totals, "found"), Some(15));
    assert_eq!(raw_u64(&totals, "total_moments"), Some(41));
    assert_eq!(array_len(&totals, "grouped_hits"), 1);
}

#[test]
fn index_request_statuses_include_review_states() {
    assert!(is_valid_index_request_status("pending"));
    assert!(is_valid_index_request_status("queued"));
    assert!(is_valid_index_request_status("running"));
    assert!(is_valid_index_request_status("failed"));
    assert!(is_valid_index_request_status("rejected"));
    assert!(!is_valid_index_request_status("complete"));
}

#[test]
fn index_request_text_fields_are_bounded() {
    assert!(normalize_handle_checked("@valid-handle").is_ok());
    assert!(normalize_handle_checked(&format!("@{}", "a".repeat(MAX_HANDLE_BYTES))).is_err());
    assert_eq!(
        normalize_index_request_note(Some("  useful context  ".to_string())).unwrap(),
        Some("useful context".to_string())
    );
    assert!(normalize_index_request_note(Some("x".repeat(MAX_NOTE_CHARS + 1))).is_err());
}

#[test]
fn searchable_terms_ignore_short_terms_and_stopwords() {
    assert_eq!(searchable_terms("fable on high"), vec!["fable", "high"]);
    assert!(searchable_terms("I").is_empty());
}

#[test]
fn relevance_score_prefers_specific_query_terms_over_stopword_noise() {
    let focused = SearchHit {
        text: "Use Fable on high reasoning".to_string(),
        text_highlight: Some("Use <mark>Fable</mark> on <mark>high</mark> reasoning".to_string()),
        ..SearchHit::from_backend_hit(
            &test_document("focused", "video-a", "Use Fable on high reasoning"),
            None,
        )
        .expect("focused hit")
    };
    let noisy = SearchHit {
        text: "on and on and on".to_string(),
        text_highlight: Some("<mark>on</mark> and <mark>on</mark>".to_string()),
        ..SearchHit::from_backend_hit(&test_document("noisy", "video-a", "on and on and on"), None)
            .expect("noisy hit")
    };
    let terms = searchable_terms("fable on high");

    assert!(hit_relevance_score(&focused, &terms) > hit_relevance_score(&noisy, &terms));
}

#[test]
fn video_results_keep_every_collected_moment() {
    let hits = (0..12)
        .map(|index| {
            let mut hit = test_hit("same-video", "Complete result", "matching moment");
            hit.id = format!("moment-{index}");
            hit.t_start_ms = index * 30_000;
            hit
        })
        .collect::<Vec<_>>();

    let videos = video_results_from_hits(&hits, SearchSort::Relevance, "matching moment");

    assert_eq!(videos[0].match_count, 12);
    assert_eq!(videos[0].hits.len(), 12);
}

#[test]
fn distinct_video_probe_matches_grouped_result_count() {
    let hits = vec![
        test_hit("video-a", "A", "first"),
        test_hit("video-a", "A", "second"),
        test_hit("video-b", "B", "third"),
    ];
    assert!(has_at_least_distinct_videos(&hits, 2));
    assert!(!has_at_least_distinct_videos(&hits, 3));
    assert_eq!(
        video_results_from_hits(&hits, SearchSort::Relevance, "first").len(),
        2
    );
}

#[test]
fn relevance_reranker_promotes_batterygate_over_generic_battery_mentions() {
    let videos = video_results_from_hits(
        &[
            test_hit(
                "generic",
                "Why phones have batteries",
                "The battery can age over time.",
            ),
            test_hit(
                "batterygate",
                "The BatteryGate incident explained",
                "Apple slowed phones during the BatteryGate incident.",
            ),
        ],
        SearchSort::Relevance,
        "battery gate incident",
    );

    assert_eq!(
        videos.first().map(|video| video.video_id.as_str()),
        Some("batterygate")
    );
}

#[test]
fn relevance_reranker_promotes_a_direct_linux_exploit_match() {
    let videos = video_results_from_hits(
        &[
            test_hit(
                "generic-linux",
                "A tour of Linux desktops",
                "Linux has a broad software ecosystem.",
            ),
            test_hit(
                "exploit",
                "How the Linux exploit worked",
                "This Linux exploit gained local privileges.",
            ),
        ],
        SearchSort::Relevance,
        "linux exploit",
    );

    assert_eq!(
        videos.first().map(|video| video.video_id.as_str()),
        Some("exploit")
    );
}

#[test]
fn raw_relevance_policy_preserves_backend_video_and_moment_order() {
    let mut first_moment = test_hit(
        "backend-first",
        "A backend-ranked result",
        "A lower-scoring local substring match.",
    );
    first_moment.id = "backend-first-moment-a".to_string();
    first_moment.t_start_ms = 20_000;
    let mut second_moment = test_hit(
        "backend-first",
        "A backend-ranked result",
        "linux exploit appears here but was ranked second by the backend.",
    );
    second_moment.id = "backend-first-moment-b".to_string();
    second_moment.t_start_ms = 1_000;
    let locally_favored = test_hit(
        "local-first",
        "How the Linux exploit worked",
        "This Linux exploit gained local privileges.",
    );
    let hits = [first_moment, second_moment, locally_favored];

    let preserved = video_results_from_hits_with_policy(
        hits.to_vec(),
        SearchSort::Relevance,
        "linux exploit",
        RelevanceOrdering::Backend,
    );
    let legacy = video_results_from_hits_with_policy(
        hits.to_vec(),
        SearchSort::Relevance,
        "linux exploit",
        RelevanceOrdering::ApiHeuristic,
    );

    assert_eq!(preserved[0].video_id, "backend-first");
    assert_eq!(preserved[0].hits[0].id, "backend-first-moment-a");
    assert_eq!(legacy[0].video_id, "local-first");
    assert_eq!(legacy[1].hits[0].id, "backend-first-moment-b");
}

#[test]
fn grouping_preserves_title_hit_video_rank() {
    let mut ranked_title = test_hit("ranked", "C Programming Language", "title match");
    ranked_title.match_kind = MatchKind::Title;
    let videos = video_results_from_hits(
        &dedupe_title_only_hits(vec![
            ranked_title,
            test_hit("other", "Generic result", "other candidate"),
            test_hit("ranked", "C Programming Language", "arrays and pointers"),
        ]),
        SearchSort::Relevance,
        "c programming arrays",
    );

    assert_eq!(videos[0].video_id, "ranked");
    assert!(
        videos[0]
            .hits
            .iter()
            .all(|hit| hit.match_kind == MatchKind::Transcript)
    );
}

#[test]
fn nearby_moment_coalescing_preserves_the_best_backend_rank() {
    let hits = [20_000, 40_000, 0]
        .into_iter()
        .map(|start| {
            let mut hit = test_hit("video", "Video", "matching moment");
            hit.id = format!("moment-{start}");
            hit.t_start_ms = start;
            hit
        })
        .collect::<Vec<_>>();

    let merged = merge_nearby_transcript_hits(hits);
    let mut starts = merged.iter().map(|hit| hit.t_start_ms).collect::<Vec<_>>();
    starts.sort_unstable();
    assert_eq!(starts, vec![20_000, 40_000]);
    assert_eq!(merged[0].id, "moment-20000");
}

#[test]
#[ignore = "manual representative benchmark; run with --release --ignored --nocapture"]
fn benchmark_relevance_grouping_workload() {
    const VIDEOS: usize = 128;
    const HITS_PER_VIDEO: usize = 4;
    const ROUNDS: usize = 80;
    let hits = (0..VIDEOS)
        .flat_map(|video_index| {
            (0..HITS_PER_VIDEO).map(move |hit_index| {
                let mut hit = test_hit(
                    &format!("video-{video_index}"),
                    if video_index % 3 == 0 {
                        "Rust runtime memory safety and compiler performance"
                    } else {
                        "A practical software engineering discussion"
                    },
                    if hit_index % 2 == 0 {
                        "Rust compiler performance uses memory safety without garbage collection"
                    } else {
                        "Runtime design and practical systems engineering tradeoffs"
                    },
                );
                hit.id = format!("video-{video_index}-hit-{hit_index}");
                hit.t_start_ms = u32::try_from(hit_index * 30_000).expect("timestamp");
                hit
            })
        })
        .collect::<Vec<_>>();

    let started = Instant::now();
    let mut checksum = 0;
    for _ in 0..ROUNDS {
        let videos = std::hint::black_box(video_results_from_hits(
            std::hint::black_box(&hits),
            SearchSort::Relevance,
            std::hint::black_box("rust runtime memory safety compiler performance"),
        ));
        checksum ^= videos
            .iter()
            .map(|video| video.match_count + video.hits.len())
            .sum::<usize>();
    }
    let elapsed = started.elapsed();

    std::hint::black_box(checksum);
    assert_eq!(checksum, 0, "even rounds produce a stable checksum");
    println!(
        "videos={VIDEOS} hits={} rounds={ROUNDS} elapsed_ms={:.2} per_round_ms={:.3}",
        hits.len(),
        elapsed.as_secs_f64() * 1_000.0,
        elapsed.as_secs_f64() * 1_000.0 / ROUNDS as f64
    );
}

#[test]
fn search_response_keeps_an_empty_top_level_hits_array_for_older_frontends() {
    let videos = video_results_from_hits(
        &[test_hit("video1", "Title", "text")],
        SearchSort::Relevance,
        "q",
    );
    let response = SearchResponse {
        query: "q",
        total_moments: 1,
        total_videos: 1,
        totals_approximate: false,
        total_found: 1,
        out_of: 1,
        time_to_first_result_ms: 0,
        page: 1,
        per_page: 12,
        sort: SearchSort::Relevance,
        mode: "lexical",
        has_next_page: false,
        hits: [],
        videos: &videos,
    };
    let value = serde_json::to_value(&response).unwrap();

    assert_eq!(value["hits"], json!([]));
    assert_eq!(value["videos"][0]["hits"].as_array().unwrap().len(), 1);
}
