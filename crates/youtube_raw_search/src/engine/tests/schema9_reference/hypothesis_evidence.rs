//! Membership evidence must survive a different hypothesis winning the score.

use super::*;

fn two_literal_moments() -> FixtureCorpus {
    FixtureCorpus::from_jsonl(
        r#"{"id":"first","video_id":"video","creator":"fixture","title":"black hole","text":"blackhole","timestamp_ms":0,"content_kind":"video"}
{"id":"second","video_id":"video","creator":"fixture","title":"black hole","text":"blackhole","timestamp_ms":30000,"content_kind":"video"}"#,
    )
    .unwrap()
}

#[test]
fn split_winner_preserves_losing_whole_transcript_membership() {
    let corpus = two_literal_moments();
    let mut query = request("blackhole", 10);
    query.expansion = ExpansionPolicy::Split(EnabledFields::TextAndTitle);
    let plan =
        build_expansion_plan(&corpus, parse_query(&query.query).unwrap(), query.expansion).unwrap();
    let trace = evaluate_plan(&plan, &tokenize("black hole"), &tokenize("blackhole"));
    assert_eq!(
        trace.selected_hypothesis,
        Some(HypothesisSelection {
            kind: HypothesisKind::SplitExact,
            canonical_bytes: b"black\0hole".to_vec(),
        })
    );
    let result = evaluate_exhaustively(&corpus, &query).unwrap();
    eprintln!("winning trace: {trace:?}; result: {result:?}");
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.hits[0].document_ids.len(), 2);
    // Both literal transcript chunks satisfy membership, independently of the
    // title-only split's higher score. They are outside the 20-second window.
    assert_eq!(
        (result.total_moments, result.hits[0].provenance),
        (
            2,
            Provenance {
                title: true,
                transcript: true,
                phrase: false,
            },
        )
    );
}

#[test]
fn exact_only_keeps_both_literal_transcript_moments() {
    let result = evaluate_exhaustively(&two_literal_moments(), &request("blackhole", 10)).unwrap();
    assert_eq!(result.total_moments, 2);
    assert!(result.hits[0].provenance.transcript);
    assert!(!result.hits[0].provenance.title);
}

#[test]
fn title_only_split_has_one_synthetic_moment() {
    let mut query = request("blackhole", 10);
    query.expansion = ExpansionPolicy::Split(EnabledFields::Title);
    let result = evaluate_exhaustively(&two_literal_moments(), &query).unwrap();
    assert_eq!(result.total_moments, 1);
    assert!(result.hits[0].provenance.title);
    assert!(!result.hits[0].provenance.transcript);
}
