//! Independent QV-01 reference behavior; production parity remains a separate gate.

use super::*;

fn corpus() -> FixtureCorpus {
    let rows = [
        ("a0", "a", "inside", "fixture", "react", 0),
        ("a1", "a", "inside", "fixture", "react", 21_000),
        ("b0", "b", "inside", "react", "unrelated", 0),
        ("c0", "c", "outside", "fixture", "react", 0),
    ];
    let jsonl = rows
        .into_iter()
        .map(|(id, video_id, creator, title, text, timestamp_ms)| {
            serde_json::json!({
                "id": id,
                "video_id": video_id,
                "creator": creator,
                "title": title,
                "text": text,
                "timestamp_ms": timestamp_ms,
                "content_kind": "video"
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    FixtureCorpus::from_jsonl(&jsonl).unwrap()
}

#[test]
fn matrix_zero_page_size_preserves_exact_totals_and_disables_next_page() {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/schema9/query-matrix.json"
    )))
    .unwrap();
    let case = matrix["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "empty-010-invalid-page-size-zero")
        .unwrap();
    assert_eq!(case["expected"]["gate_id"], "QV-01");
    assert_eq!(case["request"]["page_sizes"][0], 0);
    let query = case["request"]["q"].as_str().unwrap();
    let corpus = corpus();

    let full = evaluate_exhaustively(&corpus, &request(query, 10)).unwrap();
    assert_eq!(full.total_videos, 3);
    assert_eq!(full.total_moments, 4);
    assert_eq!(full.hits.len(), 3);
    assert!(!full.has_next_page);

    let mut first_page = request(query, 1);
    let first = evaluate_exhaustively(&corpus, &first_page).unwrap();
    assert_eq!(first.hits.len(), 1);
    assert!(first.has_next_page);

    first_page.per_page = 0;
    for page in [0, 1, 3, u32::MAX as usize] {
        first_page.page = page;
        let count_only = evaluate_exhaustively(&corpus, &first_page).unwrap();
        assert_eq!(count_only.total_videos, full.total_videos);
        assert_eq!(count_only.total_moments, full.total_moments);
        assert_eq!(count_only.by_creator, full.by_creator);
        assert_eq!(count_only.by_content_kind, full.by_content_kind);
        assert!(count_only.hits.is_empty());
        assert!(!count_only.has_next_page);
    }

    first_page.creator = Some("@INSIDE".to_owned());
    let scoped = evaluate_exhaustively(&corpus, &first_page).unwrap();
    assert_eq!(scoped.total_videos, 2);
    assert_eq!(scoped.total_moments, 3);
    assert!(scoped.hits.is_empty());
    assert!(!scoped.has_next_page);
}

#[test]
fn zero_window_on_empty_match_stays_an_exact_empty_result() {
    let result = evaluate_exhaustively(&corpus(), &request("absent", 0)).unwrap();
    assert_eq!(result.total_videos, 0);
    assert_eq!(result.total_moments, 0);
    assert!(result.hits.is_empty());
    assert!(!result.has_next_page);
}

#[test]
fn differential_rejects_wrong_next_page_even_when_hits_and_totals_match() {
    struct WrongNext(OracleResult);
    impl CandidateImplementation for WrongNext {
        fn search(&self, _request: &OracleRequest) -> Result<OracleResult, OracleError> {
            let mut result = self.0.clone();
            result.has_next_page = !result.has_next_page;
            Ok(result)
        }
    }
    let corpus = corpus();
    let request = request("react", 0);
    let expected = evaluate_exhaustively(&corpus, &request).unwrap();
    assert_eq!(
        first_mismatch(&expected, &WrongNext(expected.clone()), &request),
        Some("has_next_page")
    );
}
