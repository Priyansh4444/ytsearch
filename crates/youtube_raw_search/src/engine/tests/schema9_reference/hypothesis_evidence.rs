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
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.hits[0].document_ids.len(), 2);
    assert_eq!(result.hits[0].exact_evidence, [true]);
    assert_eq!(trace.score.to_bits(), 0x4127_7cd6);
    assert_eq!(result.hits[0].score_bits, 0x4127_7cd6);
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
fn incomplete_split_does_not_publish_partial_provenance() {
    let corpus = FixtureCorpus::from_jsonl(
        r#"{"id":"one","video_id":"one","creator":"fixture","title":"blackhole","text":"black","timestamp_ms":0,"content_kind":"video"}
{"id":"lexicon","video_id":"lexicon","creator":"fixture","title":"","text":"hole","timestamp_ms":0,"content_kind":"video"}"#,
    )
    .unwrap();
    let mut query = request("blackhole", 10);
    query.expansion = ExpansionPolicy::Split(EnabledFields::TextAndTitle);
    let plan =
        build_expansion_plan(&corpus, parse_query(&query.query).unwrap(), query.expansion).unwrap();
    let ExpansionPlan::Hypotheses { slots, .. } = plan else {
        panic!("expected split plan")
    };
    assert!(
        slots["blackhole"]
            .iter()
            .any(|hypothesis| matches!(hypothesis.plan, HypothesisPlan::Split { .. }))
    );
    let result = evaluate_exhaustively(&corpus, &query).unwrap();
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.hits[0].exact_evidence, [true]);
    assert_eq!(
        result.hits[0].provenance,
        Provenance {
            title: true,
            transcript: false,
            phrase: false
        }
    );
}

#[test]
fn failed_and_or_exclusion_cannot_publish_partial_evidence() {
    let corpus = two_literal_moments();
    let mut query = request("blackhole missing", 10);
    query.expansion = ExpansionPolicy::Split(EnabledFields::TextAndTitle);
    assert_eq!(
        evaluate_exhaustively(&corpus, &query).unwrap().total_videos,
        0
    );
    query.query = "blackhole -hole".into();
    assert_eq!(
        evaluate_exhaustively(&corpus, &query).unwrap().total_videos,
        0
    );
}

#[test]
fn phrase_admission_carries_literal_bits_without_negative_evidence() {
    let corpus = FixtureCorpus::from_jsonl(
        r#"{"id":"one","video_id":"one","creator":"fixture","title":"alpha beta","text":"gamma","timestamp_ms":0,"content_kind":"video"}"#,
    )
    .unwrap();
    let result = evaluate_exhaustively(&corpus, &request("\"alpha beta\" -missing", 10)).unwrap();
    assert_eq!(result.hits[0].exact_evidence, [true, true]);
    assert_eq!(
        result.hits[0].provenance,
        Provenance {
            title: true,
            transcript: false,
            phrase: true
        }
    );
}

#[test]
fn multi_slot_admission_respects_creator_and_enabled_fields() {
    let corpus = FixtureCorpus::from_jsonl(
        r#"{"id":"keep","video_id":"keep","creator":"alice","title":"black hole alpha","text":"blackhole","timestamp_ms":0,"content_kind":"video"}
{"id":"other","video_id":"other","creator":"bob","title":"black hole alpha","text":"blackhole","timestamp_ms":0,"content_kind":"video"}
{"id":"failed","video_id":"failed","creator":"alice","title":"black hole","text":"blackhole alpha","timestamp_ms":0,"content_kind":"video"}"#,
    )
    .unwrap();
    let mut query = request("blackhole alpha", 10);
    query.creator = Some("alice".into());
    query.expansion = ExpansionPolicy::Split(EnabledFields::Title);
    let title_only = evaluate_exhaustively(&corpus, &query).unwrap();
    assert_eq!(title_only.total_videos, 1);
    assert_eq!(title_only.hits[0].video_id, "keep");
    assert_eq!(title_only.hits[0].exact_evidence, [false, true]);
    assert_eq!(
        title_only.hits[0].provenance,
        Provenance {
            title: true,
            transcript: false,
            phrase: false
        }
    );

    query.expansion = ExpansionPolicy::Split(EnabledFields::TextAndTitle);
    let both_fields = evaluate_exhaustively(&corpus, &query).unwrap();
    assert_eq!(both_fields.total_videos, 2);
    let keep = both_fields
        .hits
        .iter()
        .find(|hit| hit.video_id == "keep")
        .unwrap();
    assert_eq!(keep.exact_evidence, [true, true]);
    assert!(keep.provenance.transcript);
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
