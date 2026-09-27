use super::{EnabledFields, IndexedField, OracleError, contains_phrase, proximity};
use proximity::{FieldProximity, OrderedEvidence, PerFieldProximity, ProximityEvidence};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct PhraseSlop(u8);

impl TryFrom<i64> for PhraseSlop {
    type Error = OracleError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if !(0..=8).contains(&value) {
            return Err(OracleError::InvalidPhraseSlop);
        }
        Ok(Self(value as u8))
    }
}

pub(super) fn admits(
    terms: &[String],
    fields: EnabledFields,
    title: &[String],
    text: &[String],
    slop: PhraseSlop,
) -> OrderedEvidence {
    OrderedEvidence {
        text: fields.contains(IndexedField::Text) && admits_field(terms, text, slop),
        title: fields.contains(IndexedField::Title) && admits_field(terms, title, slop),
    }
}

fn admits_field(terms: &[String], tokens: &[String], slop: PhraseSlop) -> bool {
    if slop.0 == 0 {
        return contains_phrase(tokens, terms);
    }
    let Some(first_term) = terms.first() else {
        return false;
    };
    for (first, token) in tokens.iter().enumerate() {
        if token != first_term {
            continue;
        }
        let mut next = first + 1;
        let mut complete = true;
        for term in terms.iter().skip(1) {
            let Some(offset) = tokens[next..].iter().position(|token| token == term) else {
                complete = false;
                break;
            };
            next += offset + 1;
        }
        if complete && next - first - terms.len() <= usize::from(slop.0) {
            return true;
        }
    }
    false
}

pub(super) fn evaluate(
    terms: &[String],
    fields: EnabledFields,
    title: &[String],
    text: &[String],
    slop: PhraseSlop,
) -> ProximityEvidence {
    // Preserve the established zero-slop implementation and its score bits.
    if slop.0 == 0 {
        return proximity::exact_phrase(terms, fields, title, text);
    }
    let text = fields
        .contains(IndexedField::Text)
        .then(|| occurrence(terms, text, slop))
        .flatten();
    let title = fields
        .contains(IndexedField::Title)
        .then(|| occurrence(terms, title, slop))
        .flatten();
    ProximityEvidence {
        fields: PerFieldProximity { text, title },
        ordered: OrderedEvidence {
            text: text.is_some(),
            title: title.is_some(),
        },
    }
}

fn occurrence(terms: &[String], tokens: &[String], slop: PhraseSlop) -> Option<FieldProximity> {
    if terms.is_empty() {
        return None;
    }
    fn visit(
        terms: &[String],
        tokens: &[String],
        budget: usize,
        positions: &mut Vec<usize>,
        best: &mut Option<(FieldProximity, Vec<usize>)>,
    ) {
        if positions.len() == terms.len() {
            let max_offset = *positions.last().unwrap();
            let displacement = max_offset - positions[0];
            let factor = if terms.len() == 1 {
                1.0
            } else {
                proximity::factor(displacement, max_offset, terms.len())
            };
            assert!(factor.is_finite(), "phrase factor must remain finite");
            let candidate = FieldProximity {
                displacement,
                max_offset,
                factor,
            };
            if best.as_ref().is_none_or(|(current, sequence)| {
                factor > current.factor
                    || (factor == current.factor
                        && (displacement, max_offset, positions.as_slice())
                            < (
                                current.displacement,
                                current.max_offset,
                                sequence.as_slice(),
                            ))
            }) {
                *best = Some((candidate, positions.clone()));
            }
            return;
        }
        let depth = positions.len();
        let start = positions.last().map_or(0, |position| position + 1);
        for (position, token) in tokens.iter().enumerate().skip(start) {
            // Every intervening token already spent is unrecoverable later.
            if positions
                .first()
                .is_some_and(|first| position - first - depth > budget)
            {
                break;
            }
            if token == &terms[depth] {
                positions.push(position);
                visit(terms, tokens, budget, positions, best);
                positions.pop();
            }
        }
    }
    let mut best = None;
    visit(
        terms,
        tokens,
        usize::from(slop.0),
        &mut Vec::new(),
        &mut best,
    );
    best.map(|(evidence, _)| evidence)
}

#[cfg(test)]
mod tests {
    use super::super::{FIXTURE, FixtureCorpus, evaluate_exhaustively, request, tokenize};
    use super::*;

    fn corpus(rows: &[(&str, &str, &str, &str)]) -> FixtureCorpus {
        let prototype = FixtureCorpus::from_jsonl(FIXTURE).unwrap().documents[0].clone();
        FixtureCorpus {
            documents: rows
                .iter()
                .map(|(id, video, title, text)| {
                    let mut document = prototype.clone();
                    document.id = (*id).to_owned();
                    document.video_id = (*video).to_owned();
                    document.title = (*title).to_owned();
                    document.text = (*text).to_owned();
                    document
                })
                .collect(),
        }
    }

    #[test]
    fn admission_truth_matches_scored_phrase_evidence() {
        let cases = [
            (
                vec!["alpha".into(), "beta".into()],
                "alpha beta",
                "beta alpha",
            ),
            (
                vec!["alpha".into(), "beta".into()],
                "alpha gap beta",
                "alpha beta",
            ),
            (
                vec!["alpha".into(), "alpha".into()],
                "alpha gap alpha",
                "alpha",
            ),
            (
                vec!["alpha".into(), "beta".into(), "gamma".into()],
                "alpha gap beta gap gamma",
                "alpha beta gamma",
            ),
        ];
        for slop in 0..=8 {
            let slop = PhraseSlop::try_from(slop).unwrap();
            for fields in [
                EnabledFields::Text,
                EnabledFields::Title,
                EnabledFields::TextAndTitle,
            ] {
                for (terms, title, text) in &cases {
                    let title = super::super::tokenize(title);
                    let text = super::super::tokenize(text);
                    let admitted = admits(terms, fields, &title, &text, slop);
                    let scored = evaluate(terms, fields, &title, &text, slop).ordered;
                    assert_eq!(
                        admitted, scored,
                        "fields={fields:?} slop={slop:?} terms={terms:?}"
                    );
                }
            }
        }
    }

    fn run(corpus: &FixtureCorpus, query: &str, slop: i64) -> super::super::OracleResult {
        let mut request = request(query, 100);
        request.phrase_slop = Some(PhraseSlop::try_from(slop).unwrap());
        evaluate_exhaustively(corpus, &request).unwrap()
    }

    #[test]
    fn typed_boundary_distinguishes_absence_and_explicit_zero() {
        let corpus = corpus(&[("a", "a", "", "alpha")]);
        assert!(evaluate_exhaustively(&corpus, &request("alpha", 10)).is_ok());
        for value in 0..=8 {
            for query in ["alpha", "*", "-\"alpha beta\"", "alpha -\"beta gamma\""] {
                let mut request = request(query, 10);
                request.phrase_slop = Some(PhraseSlop::try_from(value).unwrap());
                assert_eq!(
                    evaluate_exhaustively(&corpus, &request),
                    Err(OracleError::PhraseSlopRequiresPositivePhrase)
                );
            }
            assert_eq!(
                run(&corpus, "\"alpha\"", value),
                run(&corpus, "\"alpha\"", 0)
            );
        }
        for value in [-1, 9, i64::MIN, i64::MAX] {
            assert_eq!(
                PhraseSlop::try_from(value),
                Err(OracleError::InvalidPhraseSlop)
            );
        }
        assert_eq!(
            run(&corpus, "\"alpha\"", 0),
            evaluate_exhaustively(&corpus, &request("\"alpha\"", 100)).unwrap()
        );
    }

    #[test]
    fn total_gap_budget_order_repetition_and_boundaries() {
        let corpus = corpus(&[
            ("exact", "exact", "", "alpha beta gamma"),
            ("gapped", "gapped", "", "alpha gap beta gap gamma"),
            ("reverse", "reverse", "", "gamma beta alpha"),
            ("field", "field", "alpha", "beta gamma"),
            ("chunk-a", "chunk", "", "alpha"),
            ("chunk-b", "chunk", "", "beta gamma"),
            ("repeat", "repeat", "", "alpha gap alpha"),
        ]);
        assert_eq!(run(&corpus, "\"alpha beta gamma\"", 1).total_videos, 1);
        let admitted = run(&corpus, "\"alpha beta gamma\"", 2);
        assert_eq!(
            admitted
                .hits
                .iter()
                .map(|hit| hit.video_id.as_str())
                .collect::<Vec<_>>(),
            ["exact", "gapped"]
        );
        assert!(
            admitted
                .hits
                .iter()
                .all(|hit| hit.ordered_phrase && hit.provenance.phrase)
        );
        assert_eq!(run(&corpus, "\"alpha alpha\"", 0).total_videos, 0);
        assert_eq!(
            run(&corpus, "\"alpha alpha\"", 1).hits[0].video_id,
            "repeat"
        );
        assert_eq!(run(&corpus, "\"alpha alpha alpha\"", 8).total_videos, 0);
        assert_eq!(run(&corpus, "\"alpha beta gamma\"", 8).total_videos, 2);
    }

    #[test]
    fn budget_eight_and_phrase_length_thirty_two_are_inclusive() {
        let gap = std::iter::repeat_n("gap", 8).collect::<Vec<_>>().join(" ");
        let text = format!("alpha {gap} beta");
        let corpus = corpus(&[("eight", "eight", "", &text)]);
        assert_eq!(run(&corpus, "\"alpha beta\"", 7).total_videos, 0);
        assert_eq!(run(&corpus, "\"alpha beta\"", 8).total_videos, 1);
        let terms = (0..32).map(|i| format!("t{i}")).collect::<Vec<_>>();
        let mut tokens = terms.clone();
        tokens.splice(16..16, vec!["gap".to_owned(); 8]);
        assert!(occurrence(&terms, &tokens, PhraseSlop::try_from(7).unwrap()).is_none());
        assert_eq!(
            occurrence(&terms, &tokens, PhraseSlop::try_from(8).unwrap())
                .unwrap()
                .displacement,
            39
        );
    }

    #[test]
    fn every_phrase_gets_its_own_budget_and_negatives_stay_contiguous() {
        let corpus = corpus(&[
            (
                "gapped",
                "gapped",
                "",
                "alpha gap beta gamma gap delta red gap blue",
            ),
            (
                "excluded",
                "excluded",
                "",
                "alpha gap beta gamma gap delta red blue",
            ),
        ]);
        let query = "\"alpha beta\" \"gamma delta\"";
        assert_eq!(run(&corpus, query, 1).total_videos, 2);
        assert_eq!(
            run(&corpus, &format!("{query} \"alpha beta\""), 1),
            run(&corpus, query, 1)
        );
        let remaining = run(&corpus, &format!("{query} -\"red blue\""), 8);
        assert_eq!(remaining.total_videos, 1);
        assert_eq!(remaining.hits[0].video_id, "gapped");
        let positive = run(&corpus, query, 8);
        assert_eq!(
            remaining.hits[0],
            *positive
                .hits
                .iter()
                .find(|hit| hit.video_id == "gapped")
                .unwrap()
        );
    }

    #[test]
    fn maximum_factor_can_prefer_an_early_wider_occurrence() {
        let terms = tokenize("alpha beta");
        let mut tokens = vec!["gap".to_owned(); 104];
        tokens[0] = "alpha".to_owned();
        tokens[4] = "beta".to_owned();
        tokens[100] = "alpha".to_owned();
        tokens[103] = "beta".to_owned();
        let tighter = occurrence(&terms, &tokens, PhraseSlop::try_from(2).unwrap()).unwrap();
        let wider = occurrence(&terms, &tokens, PhraseSlop::try_from(3).unwrap()).unwrap();
        assert_eq!((tighter.displacement, tighter.max_offset), (3, 103));
        assert_eq!((wider.displacement, wider.max_offset), (4, 4));
        assert!(wider.factor > tighter.factor);
        let mut prior = 1.0;
        for slop in 0..=8 {
            let evidence = evaluate(
                &terms,
                EnabledFields::Text,
                &[],
                &tokens,
                PhraseSlop::try_from(slop).unwrap(),
            );
            assert!(evidence.field_factor() >= prior);
            prior = evidence.field_factor();
        }
        assert_eq!(wider.factor.to_bits(), 0x3f8f_a984);
    }

    #[test]
    fn enabled_fields_control_gapped_phrase_evidence() {
        let terms = tokenize("alpha beta");
        let title = tokenize("alpha gap beta");
        let text = tokenize("alpha gap gap beta");
        let slop = PhraseSlop::try_from(1).unwrap();
        assert!(
            !evaluate(&terms, EnabledFields::Text, &title, &text, slop)
                .ordered
                .any()
        );
        let evidence = evaluate(&terms, EnabledFields::TextAndTitle, &title, &text, slop);
        assert_eq!(
            evidence.ordered,
            OrderedEvidence {
                text: false,
                title: true
            }
        );
    }

    #[test]
    fn phrase_score_bits_follow_single_document_arithmetic_and_zero_is_unchanged() {
        // Independently evaluated binary32 arithmetic: N=DF=1, TF=1,
        // length/average=1, hence BM25 TF=1 and IDF=ln(1+0.5/1.5).
        // Score = (4*IDF*1.15 + 4*IDF*(1+0.15/sqrt(end+1)))
        //       * ((1+0.20/sqrt(end))+0.05/sqrt(end+1)) * 1.25.
        for (text, slop, factor_bits, score_bits) in [
            ("alpha beta", 0, 0x3f9e_2020, 0x4080_4903),
            ("alpha gap beta", 1, 0x3f95_cc06, 0x4070_f5b9),
            ("alpha gap gap beta", 2, 0x3f91_faec, 0x4069_9a24),
        ] {
            let corpus = corpus(&[("one", "one", "", text)]);
            let result = run(&corpus, "\"alpha beta\"", slop);
            assert_eq!(result.hits.len(), 1);
            assert_eq!(result.hits[0].score_bits, score_bits);
            let evidence = evaluate(
                &tokenize("alpha beta"),
                EnabledFields::Text,
                &[],
                &tokenize(text),
                PhraseSlop::try_from(slop).unwrap(),
            );
            assert_eq!(evidence.field_factor().to_bits(), factor_bits);
            assert_eq!(
                run(&corpus, "\"alpha beta\"", 0),
                evaluate_exhaustively(&corpus, &request("\"alpha beta\"", 100)).unwrap()
            );
        }
        let fixture = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
        for query in [
            "\"create effect\"",
            "\"this the\"",
            "\"alpha\"",
            "\"alpha alpha\"",
        ] {
            assert_eq!(
                run(&fixture, query, 0),
                evaluate_exhaustively(&fixture, &request(query, 100)).unwrap()
            );
        }
    }

    #[test]
    fn equal_factor_occurrences_and_equal_score_videos_have_stable_ties() {
        // Single-token factors are exactly 1, so the earliest endpoint wins
        // even though every occurrence has identical numeric evidence.
        let evidence = occurrence(
            &tokenize("alpha"),
            &tokenize("gap alpha alpha"),
            PhraseSlop::try_from(8).unwrap(),
        )
        .unwrap();
        assert_eq!((evidence.displacement, evidence.max_offset), (0, 1));
        assert_eq!(evidence.factor.to_bits(), 1.0_f32.to_bits());
        let mut corpus = corpus(&[
            ("z", "z", "", "alpha gap beta"),
            ("a", "a", "", "alpha gap beta"),
        ]);
        let result = run(&corpus, "\"alpha beta\"", 1);
        assert_eq!(result.hits[0].score_bits, result.hits[1].score_bits);
        assert_eq!(
            result
                .hits
                .iter()
                .map(|hit| hit.video_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );
        corpus.documents.reverse();
        assert_eq!(run(&corpus, "\"alpha beta\"", 1), result);
    }

    #[test]
    fn matrix_slop_membership_rank_pages_and_aggregates_subset() {
        // This exercises the local fixture subset, not the complete matrix
        // assertion vector (notably highlights and cursor encoding).
        let matrix: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/schema9/query-matrix.json"
        )))
        .unwrap();
        let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
        let mut checked = 0;
        for case in matrix["cases"].as_array().unwrap() {
            let Some(slop) = case["request"]["phrase_slop"].as_i64() else {
                continue;
            };
            assert_eq!(case["request"]["scope"], serde_json::json!({"kind": "all"}));
            assert_eq!(case["request"]["sort"], "relevance");
            assert_eq!(case["request"]["filters"], serde_json::json!([]));
            assert!(case["request"]["cursor"].is_null());
            assert_eq!(case["request"]["grammar_version"], 2);
            assert_eq!(case["request"]["rank_version"], 2);
            let query = case["request"]["q"].as_str().unwrap();
            let complete = run(&corpus, query, slop);
            for size in case["request"]["page_sizes"].as_array().unwrap() {
                let mut request = request(query, size.as_u64().unwrap() as usize);
                request.phrase_slop = Some(PhraseSlop::try_from(slop).unwrap());
                let mut hits = Vec::new();
                loop {
                    let page = evaluate_exhaustively(&corpus, &request).unwrap();
                    assert_eq!(page.total_videos, complete.total_videos);
                    assert_eq!(page.total_moments, complete.total_moments);
                    assert_eq!(page.by_creator, complete.by_creator);
                    assert_eq!(page.by_content_kind, complete.by_content_kind);
                    let empty = page.hits.is_empty();
                    hits.extend(page.hits);
                    if empty {
                        break;
                    }
                    request.page += 1;
                }
                assert_eq!(hits, complete.hits);
            }
            checked += 1;
        }
        assert_eq!(checked, 5);
    }
}
