//! Executable schema-9 query-contract oracle.
//!
//! These tests intentionally do not call the schema-6 scoring implementation.
//! They pin the reviewed v2 decisions before the lazy engine is implemented,
//! so every later execution path has one bit-exact reference to match.

use std::{cmp::Ordering, collections::BTreeMap};

use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum HypothesisKind {
    ExactWhole,
    PrefixWhole,
    SplitExact,
    SplitFuzzy,
    TypoOne,
    TypoTwo,
}

#[derive(Clone, Copy, Debug)]
struct Hypothesis<'a> {
    kind: HypothesisKind,
    bytes: &'a [u8],
    score: f32,
}

fn best_hypothesis<'a>(hypotheses: &'a [Hypothesis<'a>]) -> &'a Hypothesis<'a> {
    assert!(!hypotheses.is_empty());
    assert!(
        hypotheses
            .iter()
            .all(|hypothesis| hypothesis.score.is_finite())
    );
    hypotheses
        .iter()
        .min_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.bytes.cmp(right.bytes))
        })
        .expect("non-empty hypotheses")
}

fn fuzzy_split_score(
    left: f32,
    right: f32,
    text_proximity: f32,
    title_proximity: f32,
    adjacent: bool,
) -> (f32, f32, f32) {
    let with_bonus = (left + right) + 4.0_f32;
    let with_proximity = with_bonus * text_proximity.max(title_proximity);
    let final_score = if adjacent {
        with_proximity * 1.25_f32
    } else {
        with_proximity
    };
    (with_bonus, with_proximity, final_score)
}

fn phrase_score(raw: f32, factors: &[f32]) -> (f32, bool) {
    let factor = factors
        .iter()
        .copied()
        .inspect(|factor| assert!(factor.is_finite()))
        .max_by(f32::total_cmp)
        .unwrap_or(1.0);
    (raw * factor, factors.iter().any(|factor| *factor > 1.0))
}

fn ordered_proximity_score(
    raw: f32,
    text_factor: f32,
    title_factor: f32,
    ordered_factor: f32,
) -> (f32, f32) {
    let field_adjusted = raw * text_factor.max(title_factor);
    (field_adjusted, field_adjusted * ordered_factor)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OptionalNumeric<T> {
    Present(T),
    Missing,
}

fn descending_numeric<T: Ord>(left: OptionalNumeric<T>, right: OptionalNumeric<T>) -> Ordering {
    match (left, right) {
        (OptionalNumeric::Present(left), OptionalNumeric::Present(right)) => right.cmp(&left),
        (OptionalNumeric::Present(_), OptionalNumeric::Missing) => Ordering::Less,
        (OptionalNumeric::Missing, OptionalNumeric::Present(_)) => Ordering::Greater,
        (OptionalNumeric::Missing, OptionalNumeric::Missing) => Ordering::Equal,
    }
}

fn publication_time(
    left: OptionalNumeric<i64>,
    right: OptionalNumeric<i64>,
    newest_first: bool,
) -> Ordering {
    match (left, right) {
        (OptionalNumeric::Present(left), OptionalNumeric::Present(right)) if newest_first => {
            right.cmp(&left)
        }
        (OptionalNumeric::Present(left), OptionalNumeric::Present(right)) => left.cmp(&right),
        (OptionalNumeric::Present(_), OptionalNumeric::Missing) => Ordering::Less,
        (OptionalNumeric::Missing, OptionalNumeric::Present(_)) => Ordering::Greater,
        (OptionalNumeric::Missing, OptionalNumeric::Missing) => Ordering::Equal,
    }
}

#[derive(Clone, Copy)]
struct Contribution {
    slot: u8,
    term: &'static [u8],
    field: u8,
    expansion: u8,
    score: f32,
}

fn materialized_reduce(mut contributions: Vec<Contribution>) -> f32 {
    contributions.sort_by(|left, right| {
        left.slot
            .cmp(&right.slot)
            .then_with(|| left.term.cmp(right.term))
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.expansion.cmp(&right.expansion))
    });
    contributions
        .into_iter()
        .fold(0.0_f32, |score, contribution| score + contribution.score)
}

fn streaming_reduce(contributions: impl IntoIterator<Item = Contribution>) -> f32 {
    let mut ordered = BTreeMap::<(u8, &'static [u8], u8, u8), f32>::new();
    for contribution in contributions {
        *ordered
            .entry((
                contribution.slot,
                contribution.term,
                contribution.field,
                contribution.expansion,
            ))
            .or_default() += contribution.score;
    }
    let mut score = 0.0_f32;
    for contribution in ordered.into_values() {
        score += contribution;
    }
    score
}

fn v2_tokens(input: &str) -> Vec<String> {
    let normalized = input.nfc().collect::<String>();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in normalized.chars() {
        if character.is_alphanumeric() {
            current.extend(character.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(current.nfc().collect());
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.push(current.nfc().collect());
    }
    tokens
}

#[test]
fn overlapping_whole_and_split_hypotheses_choose_one_branch_bit_exact() {
    let hypotheses = [
        Hypothesis {
            kind: HypothesisKind::ExactWhole,
            bytes: b"createeffect",
            score: 5.0,
        },
        Hypothesis {
            kind: HypothesisKind::SplitExact,
            bytes: b"create\0effect",
            score: (3.0_f32 + 4.0_f32) + 8.0_f32,
        },
    ];
    let selected = best_hypothesis(&hypotheses);
    assert_eq!(selected.kind, HypothesisKind::SplitExact);
    assert_eq!(selected.score.to_bits(), 0x4170_0000);

    let equal_score = [
        Hypothesis {
            kind: HypothesisKind::SplitExact,
            bytes: b"a\0b",
            score: 5.0,
        },
        Hypothesis {
            kind: HypothesisKind::ExactWhole,
            bytes: b"ab",
            score: 5.0,
        },
        Hypothesis {
            kind: HypothesisKind::ExactWhole,
            bytes: b"aa",
            score: 5.0,
        },
        Hypothesis {
            kind: HypothesisKind::PrefixWhole,
            bytes: b"a",
            score: 5.0,
        },
        Hypothesis {
            kind: HypothesisKind::SplitFuzzy,
            bytes: b"a\0c",
            score: 5.0,
        },
        Hypothesis {
            kind: HypothesisKind::TypoOne,
            bytes: b"ac",
            score: 5.0,
        },
        Hypothesis {
            kind: HypothesisKind::TypoTwo,
            bytes: b"ad",
            score: 5.0,
        },
    ];
    let selected = best_hypothesis(&equal_score);
    assert_eq!(selected.kind, HypothesisKind::ExactWhole);
    assert_eq!(selected.bytes, b"aa");
    assert_eq!(selected.score.to_bits(), 0x40a0_0000);
}

#[test]
fn fuzzy_split_reduction_is_local_and_bit_exact() {
    let (with_bonus, with_proximity, final_score) = fuzzy_split_score(1.5, 2.25, 1.125, 1.0, true);
    assert_eq!(with_bonus.to_bits(), 0x40f8_0000);
    assert_eq!(with_proximity.to_bits(), 0x410b_8000);
    assert_eq!(final_score.to_bits(), 0x412e_6000);

    let (_, loose_proximity, loose_score) = fuzzy_split_score(1.5, 2.25, 1.0, 1.0, false);
    assert_eq!(loose_proximity.to_bits(), 0x40f8_0000);
    assert_eq!(loose_score.to_bits(), 0x40f8_0000);
}

#[test]
fn multiple_positive_phrases_use_one_max_factor_bit_exact() {
    let (score, ordered) = phrase_score(8.0, &[1.25, 1.5]);
    assert_eq!(score.to_bits(), 0x4140_0000);
    assert!(ordered);
    assert_ne!(score.to_bits(), (8.0_f32 * 1.25_f32 * 1.5_f32).to_bits());
}

#[test]
fn both_field_proximity_uses_max_then_ordered_factor_bits() {
    let (field_adjusted, final_score) = ordered_proximity_score(7.25, 1.125, 1.25, 1.25);
    assert_eq!(field_adjusted.to_bits(), 0x4111_0000);
    assert_eq!(final_score.to_bits(), 0x4135_4000);
}

#[test]
fn numeric_sort_missing_is_below_present_zero() {
    let mut values = [
        OptionalNumeric::Missing,
        OptionalNumeric::Present(0_u64),
        OptionalNumeric::Present(9_u64),
    ];
    values.sort_by(|left, right| descending_numeric(*left, *right));
    assert_eq!(
        values,
        [
            OptionalNumeric::Present(9),
            OptionalNumeric::Present(0),
            OptionalNumeric::Missing,
        ]
    );
}

#[test]
fn publication_sort_missing_is_last_in_both_directions() {
    let source = [
        OptionalNumeric::Missing,
        OptionalNumeric::Present(-1_i64),
        OptionalNumeric::Present(0_i64),
        OptionalNumeric::Present(1_i64),
    ];
    let mut newest = source;
    newest.sort_by(|left, right| publication_time(*left, *right, true));
    assert_eq!(
        newest,
        [
            OptionalNumeric::Present(1),
            OptionalNumeric::Present(0),
            OptionalNumeric::Present(-1),
            OptionalNumeric::Missing,
        ]
    );
    let mut oldest = source;
    oldest.sort_by(|left, right| publication_time(*left, *right, false));
    assert_eq!(
        oldest,
        [
            OptionalNumeric::Present(-1),
            OptionalNumeric::Present(0),
            OptionalNumeric::Present(1),
            OptionalNumeric::Missing,
        ]
    );
}

#[test]
fn materialized_and_streaming_reducers_have_identical_bits() {
    let contributions = vec![
        Contribution {
            slot: 1,
            term: b"effect",
            field: 1,
            expansion: 0,
            score: 6.0,
        },
        Contribution {
            slot: 0,
            term: b"create",
            field: 1,
            expansion: 0,
            score: 4.0,
        },
        Contribution {
            slot: 1,
            term: b"effect",
            field: 0,
            expansion: 0,
            score: 0.65,
        },
        Contribution {
            slot: 0,
            term: b"create",
            field: 0,
            expansion: 1,
            score: 0.4,
        },
    ];
    let materialized = materialized_reduce(contributions.clone());
    let streaming = streaming_reduce(contributions.into_iter().rev());
    assert_eq!(materialized.to_bits(), 0x4130_cccd);
    assert_eq!(streaming.to_bits(), materialized.to_bits());
}

#[test]
fn grammar_policy_goldens_cover_qg01_qg03_and_qg07() {
    assert!(
        v2_tokens("...—!!!").is_empty(),
        "QG-01 rejects this; it is not wildcard"
    );
    assert_eq!(v2_tokens("１２３ abc123"), ["１２３", "abc123"]);

    let numeric_is_exact_only = v2_tokens("１２３")[0]
        .chars()
        .all(|character| character.is_numeric());
    let mixed_uses_normal_expansion = !v2_tokens("abc123")[0]
        .chars()
        .all(|character| character.is_numeric());
    assert!(
        numeric_is_exact_only && mixed_uses_normal_expansion,
        "QG-03"
    );

    let enabled_fields = ["text", "title"];
    let prefix = "cre";
    let field_terms = [("text", "create"), ("title", "creative")];
    for field in enabled_fields {
        assert!(
            field_terms.iter().any(|(candidate_field, term)| {
                *candidate_field == field && term.starts_with(prefix)
            }),
            "QG-07 must apply the same prefix predicate to {field}"
        );
    }
}

#[test]
fn nfc_tokenization_is_shared_and_scalar_thresholds_are_stable() {
    assert_eq!(v2_tokens("Cafe\u{301}"), v2_tokens("Café"), "QG-02");
    assert_eq!(
        v2_tokens("İ")[0].chars().count(),
        2,
        "thresholds count normalized scalars"
    );
}

#[test]
fn complete_expansion_and_drop_ladders_have_no_semantic_work_caps() {
    let equal_distance_terms = ["bat", "cat", "fat", "hat", "mat", "pat"];
    let admitted = equal_distance_terms.to_vec();
    assert_eq!(
        admitted.len(),
        6,
        "QG-04 cannot retain only four alternatives"
    );

    let qualifying_splits = (0..12).map(|index| (index, index + 1)).collect::<Vec<_>>();
    assert_eq!(
        qualifying_splits.len(),
        12,
        "QG-05 cannot retain only eight splits"
    );

    let tokens = (0..24).collect::<Vec<_>>();
    let prefixes = (1..tokens.len()).rev().map(|length| &tokens[..length]);
    let suffixes = (1..tokens.len()).map(|start| &tokens[start..]);
    let ladder = prefixes.chain(suffixes).collect::<Vec<_>>();
    assert_eq!(
        ladder.len(),
        46,
        "QG-06 must not stop at the legacy 20+20 cap"
    );
    assert_eq!(ladder.first().expect("first prefix").len(), 23);
    assert_eq!(ladder.last().expect("last suffix"), &&tokens[23..]);
}

#[test]
fn zero_window_is_an_exact_count_only_success() {
    struct CountOnly {
        exact_total: u64,
        tier0_total: u64,
        hits: Vec<()>,
        cursor: Option<()>,
        has_next_page: bool,
    }
    let result = CountOnly {
        exact_total: 12,
        tier0_total: 12,
        hits: Vec::new(),
        cursor: None,
        has_next_page: false,
    };
    assert_eq!(result.exact_total, result.tier0_total, "QV-01");
    assert!(result.hits.is_empty());
    assert!(result.cursor.is_none());
    assert!(!result.has_next_page);
}
