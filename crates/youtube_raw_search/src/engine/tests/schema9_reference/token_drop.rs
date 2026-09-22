//! QG-06's exact-only ordered selector, not the automatic expansion controller.
//! Scores and evidence describe retained slots; corpus statistics remain global.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedRange {
    Full,
    Prefix { end: usize },
    Suffix { start: usize },
}

#[derive(Debug, Eq, PartialEq)]
enum Selection {
    Selected {
        retained: RetainedRange,
        result: OracleResult,
    },
    Exhausted {
        result: OracleResult,
    },
}

fn select_exact_retained_query(
    corpus: &FixtureCorpus,
    request: &OracleRequest,
) -> Result<Selection, OracleError> {
    if request.expansion != ExpansionPolicy::ExactOnly {
        return Err(OracleError::UnsupportedReferenceSemantics(
            "retained_query_requires_exact_only",
        ));
    }
    let query = parse_query(&request.query)?;
    if query_contains_positive_phrase(&query) {
        return Err(OracleError::UnsupportedReferenceSemantics(
            "retained_query_with_positive_phrase",
        ));
    }
    let terms = positive_unquoted_terms(&query);
    let full = evaluate_query_exhaustively(corpus, request, query.clone())?;
    if full.total_videos > 0 {
        return Ok(Selection::Selected {
            retained: RetainedRange::Full,
            result: full,
        });
    }
    let ranges = (1..terms.len())
        .rev()
        .map(|end| RetainedRange::Prefix { end })
        .chain((1..terms.len()).map(|start| RetainedRange::Suffix { start }));
    for retained in ranges {
        let retained_terms = match retained {
            RetainedRange::Prefix { end } => &terms[..end],
            RetainedRange::Suffix { start } => &terms[start..],
            RetainedRange::Full => unreachable!("full evaluated before dropping"),
        };
        let predicate = retain_terms(&query, retained_terms);
        let result = evaluate_query_exhaustively(corpus, request, predicate)?;
        // Threshold one is equivalent for grouped videos and matching documents.
        // The returned page can be empty even when this predicate wins.
        if result.total_videos > 0 {
            return Ok(Selection::Selected { retained, result });
        }
    }
    Ok(Selection::Exhausted { result: full })
}

fn retain_terms(query: &Query, retained: &[&str]) -> Query {
    match query {
        Query::Term(term) if !retained.contains(&term.as_str()) => Query::All,
        Query::And(children) => Query::And(
            children
                .iter()
                .map(|child| retain_terms(child, retained))
                .collect(),
        ),
        // Negations stay opaque: their exact predicates never drop or expand.
        Query::All | Query::Term(_) | Query::Phrase(_) | Query::Not(_) => query.clone(),
    }
}

fn corpus(texts: &[&str]) -> FixtureCorpus {
    let source = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            serde_json::json!({
                "id": format!("doc-{index}"),
                "video_id": format!("video-{index}"),
                "creator": "inside",
                "title": "fixture",
                "text": text,
                "timestamp_ms": 0,
                "content_kind": "video"
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    FixtureCorpus::from_jsonl(&source).unwrap()
}

fn selected(selection: Selection, expected: RetainedRange) -> OracleResult {
    let Selection::Selected { retained, result } = selection else {
        panic!("expected selected predicate");
    };
    assert_eq!(retained, expected);
    result
}

#[test]
fn every_prefix_and_suffix_is_reachable_including_beyond_legacy_caps() {
    for count in [21, 22, 32] {
        let terms = (0..count).map(|i| format!("term{i}")).collect::<Vec<_>>();
        let request = request(&terms.join(" "), 10);
        for end in 1..count {
            let corpus = corpus(&[&terms[..end].join(" ")]);
            let result = selected(
                select_exact_retained_query(&corpus, &request).unwrap(),
                RetainedRange::Prefix { end },
            );
            assert_eq!(result.total_videos, 1);
            assert_eq!(result.hits[0].exact_evidence.len(), end);
        }
        for start in 1..count {
            let corpus = corpus(&[&terms[start..].join(" ")]);
            let result = selected(
                select_exact_retained_query(&corpus, &request).unwrap(),
                RetainedRange::Suffix { start },
            );
            assert_eq!(result.total_videos, 1);
            assert_eq!(result.hits[0].exact_evidence.len(), count - start);
        }
    }
}

#[test]
fn actual_threshold_004_query_reaches_twenty_first_prefix_and_suffix() {
    let matrix: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/schema9/query-matrix.json"
    )))
    .unwrap();
    let case = matrix["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "threshold-004-fifteen-terms")
        .unwrap();
    let query = case["request"]["q"].as_str().unwrap();
    let parsed = parse_query(query).unwrap();
    let terms = positive_unquoted_terms(&parsed);
    assert_eq!(terms.len(), 22);
    for (text, range) in [
        (terms[0], RetainedRange::Prefix { end: 1 }),
        (terms[21], RetainedRange::Suffix { start: 21 }),
    ] {
        let result = selected(
            select_exact_retained_query(&corpus(&[text]), &request(query, 10)).unwrap(),
            range,
        );
        assert_eq!(result.total_videos, 1);
    }
}

#[test]
fn selection_uses_first_distinct_slots_and_prefixes_before_suffixes() {
    let corpus = corpus(&["zulu alpha", "alpha beta gamma"]);
    let result = selected(
        select_exact_retained_query(&corpus, &request("zulu alpha zulu beta gamma alpha", 10))
            .unwrap(),
        RetainedRange::Prefix { end: 2 },
    );
    assert_eq!(result.hits[0].video_id, "video-0");
    assert_eq!(
        result,
        evaluate_exhaustively(&corpus, &request("zulu alpha", 10)).unwrap()
    );
}

#[test]
fn exclusions_and_creator_scope_apply_before_selection_and_scoring_stays_global() {
    let mut corpus = corpus(&[
        "alpha beta",
        "alpha blocked",
        "alpha banned phrase",
        "beta",
        "beta",
    ]);
    corpus.documents[0].creator = "outside".to_owned();
    corpus.documents[4].video_id = corpus.documents[3].video_id.clone();
    let mut query = request("alpha beta -blocked -\"banned phrase\"", 10);
    query.creator = Some("INSIDE".to_owned());
    let result = selected(
        select_exact_retained_query(&corpus, &query).unwrap(),
        RetainedRange::Suffix { start: 1 },
    );
    let mut exact = query.clone();
    exact.query = "beta -blocked -\"banned phrase\"".to_owned();
    assert_eq!(result, evaluate_exhaustively(&corpus, &exact).unwrap());
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.hits[0].document_ids.len(), 2);
    let scoped = FixtureCorpus {
        documents: corpus.documents[1..].to_vec(),
    };
    assert_ne!(
        result.hits[0].score_bits,
        evaluate_exhaustively(&scoped, &exact).unwrap().hits[0].score_bits
    );
}

#[test]
fn selected_full_set_and_counts_do_not_depend_on_the_page() {
    let corpus = corpus(&["alpha beta", "alpha beta", "alpha", "beta gamma"]);
    let mut query = request("alpha beta gamma", 10);
    let baseline = selected(
        select_exact_retained_query(&corpus, &query).unwrap(),
        RetainedRange::Prefix { end: 2 },
    );
    for (page, per_page) in [(1, 0), (1, 1), (2, 1), (50, 10), (usize::MAX, usize::MAX)] {
        query.page = page;
        query.per_page = per_page;
        let result = selected(
            select_exact_retained_query(&corpus, &query).unwrap(),
            RetainedRange::Prefix { end: 2 },
        );
        let mut exact = query.clone();
        exact.query = "alpha beta".to_owned();
        assert_eq!(result, evaluate_exhaustively(&corpus, &exact).unwrap());
        assert_eq!(result.total_videos, baseline.total_videos);
        assert_eq!(result.by_creator, baseline.by_creator);
        assert_eq!(result.total_moments, baseline.total_moments);
    }
}

#[test]
fn full_and_exhausted_outcomes_never_broaden_negative_only_or_wildcard() {
    let corpus = corpus(&["alpha beta"]);
    for query in ["alpha beta", "*", "-absent"] {
        let query = request(query, 0);
        let result = selected(
            select_exact_retained_query(&corpus, &query).unwrap(),
            RetainedRange::Full,
        );
        assert_eq!(result.total_videos, 1);
        assert!(result.hits.is_empty());
    }
    for query in ["absent missing", "-alpha", "alpha missing -beta"] {
        let query = request(query, 10);
        assert_eq!(
            select_exact_retained_query(&corpus, &query).unwrap(),
            Selection::Exhausted {
                result: evaluate_exhaustively(&corpus, &query).unwrap()
            }
        );
    }
    assert!(matches!(
        select_exact_retained_query(&FixtureCorpus { documents: vec![] }, &request("*", 10))
            .unwrap(),
        Selection::Exhausted { .. }
    ));
    assert_eq!(
        select_exact_retained_query(&corpus, &request("\"alpha beta\" missing", 10)),
        Err(OracleError::UnsupportedReferenceSemantics(
            "retained_query_with_positive_phrase"
        ))
    );
    let mut query = request("alpha", 10);
    query.expansion = ExpansionPolicy::TokenDrop;
    assert_eq!(
        evaluate_exhaustively(&corpus, &query),
        Err(OracleError::UnsupportedReferenceSemantics("token_drop"))
    );
    assert_eq!(
        select_exact_retained_query(&corpus, &query),
        Err(OracleError::UnsupportedReferenceSemantics(
            "retained_query_requires_exact_only"
        ))
    );
}
