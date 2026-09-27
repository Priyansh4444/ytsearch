use super::{EnabledFields, IndexedField};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FieldProximity {
    pub(super) displacement: usize,
    pub(super) max_offset: usize,
    pub(super) factor: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct PerFieldProximity {
    pub(super) text: Option<FieldProximity>,
    pub(super) title: Option<FieldProximity>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct OrderedEvidence {
    pub(super) text: bool,
    pub(super) title: bool,
}

impl OrderedEvidence {
    pub(super) fn any(self) -> bool {
        self.text || self.title
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct ProximityEvidence {
    pub(super) fields: PerFieldProximity,
    pub(super) ordered: OrderedEvidence,
}

impl ProximityEvidence {
    pub(super) fn include(&mut self, other: Self) {
        fn strongest(
            left: Option<FieldProximity>,
            right: Option<FieldProximity>,
        ) -> Option<FieldProximity> {
            match (left, right) {
                (Some(left), Some(right)) if right.factor > left.factor => Some(right),
                (Some(left), _) => Some(left),
                (None, right) => right,
            }
        }
        self.fields.text = strongest(self.fields.text, other.fields.text);
        self.fields.title = strongest(self.fields.title, other.fields.title);
        self.ordered.text |= other.ordered.text;
        self.ordered.title |= other.ordered.title;
    }

    pub(super) fn field_factor(self) -> f32 {
        [self.fields.text, self.fields.title]
            .into_iter()
            .flatten()
            .map(|field| field.factor)
            .max_by(f32::total_cmp)
            .unwrap_or(1.0)
    }

    pub(super) fn ordered_factor(self) -> f32 {
        if self.ordered.any() { 1.25 } else { 1.0 }
    }
}

/// Exact phrase windows retain repeated occurrences and never cross fields.
pub(super) fn exact_phrase(
    terms: &[String],
    fields: EnabledFields,
    title: &[String],
    text: &[String],
) -> ProximityEvidence {
    fn occurrence(terms: &[String], tokens: &[String]) -> Option<FieldProximity> {
        if terms.is_empty() {
            return None;
        }
        tokens
            .windows(terms.len())
            .enumerate()
            .find_map(|(start, window)| {
                (window == terms).then(|| {
                    let displacement = terms.len() - 1;
                    let max_offset = start + displacement;
                    FieldProximity {
                        displacement,
                        max_offset,
                        factor: if terms.len() == 1 {
                            1.0
                        } else {
                            factor(displacement, max_offset, terms.len())
                        },
                    }
                })
            })
    }
    let text = fields
        .contains(IndexedField::Text)
        .then(|| occurrence(terms, text))
        .flatten();
    let title = fields
        .contains(IndexedField::Title)
        .then(|| occurrence(terms, title))
        .flatten();
    ProximityEvidence {
        fields: PerFieldProximity { text, title },
        ordered: OrderedEvidence {
            text: text.is_some(),
            title: title.is_some(),
        },
    }
}

pub(super) fn evaluate(
    terms: &[&str],
    fields: EnabledFields,
    title: &[String],
    text: &[String],
) -> ProximityEvidence {
    if terms.len() < 2 {
        return ProximityEvidence::default();
    }

    let text_window = fields
        .contains(IndexedField::Text)
        .then(|| minimum_window(terms, text))
        .flatten();
    let title_window = fields
        .contains(IndexedField::Title)
        .then(|| minimum_window(terms, title))
        .flatten();
    let ordered = OrderedEvidence {
        text: fields.contains(IndexedField::Text) && has_full_adjacency(terms, text),
        title: fields.contains(IndexedField::Title) && has_full_adjacency(terms, title),
    };

    ProximityEvidence {
        fields: PerFieldProximity {
            text: text_window.map(|(displacement, max_offset)| FieldProximity {
                displacement,
                max_offset,
                factor: factor(displacement, max_offset, terms.len()),
            }),
            title: title_window.map(|(displacement, max_offset)| FieldProximity {
                displacement,
                max_offset,
                factor: factor(displacement, max_offset, terms.len()),
            }),
        },
        ordered,
    }
}

fn minimum_window(terms: &[&str], tokens: &[String]) -> Option<(usize, usize)> {
    let positions = terms
        .iter()
        .map(|term| {
            tokens
                .iter()
                .enumerate()
                .filter_map(|(position, token)| (token == term).then_some(position))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if positions.iter().any(Vec::is_empty) {
        return None;
    }

    fn visit(
        positions: &[Vec<usize>],
        depth: usize,
        minimum: usize,
        maximum: usize,
        best: &mut Option<(usize, usize)>,
    ) {
        if depth == positions.len() {
            let candidate = (maximum - minimum, maximum);
            if best.is_none_or(|current| candidate < current) {
                *best = Some(candidate);
            }
            return;
        }
        for &position in &positions[depth] {
            let minimum = if depth == 0 {
                position
            } else {
                minimum.min(position)
            };
            let maximum = if depth == 0 {
                position
            } else {
                maximum.max(position)
            };
            visit(positions, depth + 1, minimum, maximum, best);
        }
    }

    let mut best = None;
    visit(&positions, 0, 0, 0, &mut best);
    best
}

pub(super) fn factor(displacement: usize, max_offset: usize, term_count: usize) -> f32 {
    let minimum_displacement = term_count.saturating_sub(1);
    let extra_gap = displacement.saturating_sub(minimum_displacement);
    let proximity = 0.20_f32 / (extra_gap as f32 + 1.0_f32).sqrt();
    let early_position = 0.05_f32 / (max_offset as f32 + 1.0_f32).sqrt();
    (1.0_f32 + proximity) + early_position
}

fn has_full_adjacency(terms: &[&str], tokens: &[String]) -> bool {
    tokens
        .windows(terms.len())
        .any(|window| window.iter().zip(terms).all(|(token, term)| token == term))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<String> {
        source.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn exact_phrase_occurrences_preserve_multiplicity_and_choose_earliest() {
        let phrase = tokens("alpha alpha");
        let early = exact_phrase(
            &phrase,
            EnabledFields::Text,
            &[],
            &tokens("alpha alpha gap alpha alpha"),
        );
        assert_eq!(early.fields.text.unwrap().max_offset, 1);
        assert_eq!(early.field_factor().to_bits(), 0x3f9e_2020);
        let late = exact_phrase(
            &phrase,
            EnabledFields::Text,
            &[],
            &tokens("gap gap alpha alpha"),
        );
        assert!(early.field_factor() > late.field_factor());
        assert_eq!(
            exact_phrase(
                &phrase,
                EnabledFields::Text,
                &[],
                &tokens("alpha gap alpha")
            ),
            ProximityEvidence::default()
        );
        assert_eq!(
            exact_phrase(
                &phrase,
                EnabledFields::TextAndTitle,
                &tokens("alpha"),
                &tokens("alpha")
            ),
            ProximityEvidence::default()
        );
    }

    #[test]
    fn exact_phrase_fields_single_tokens_and_long_phrases() {
        let phrase = tokens("alpha beta");
        let both = exact_phrase(
            &phrase,
            EnabledFields::TextAndTitle,
            &phrase,
            &tokens("gap gap alpha beta"),
        );
        assert_eq!(both.field_factor().to_bits(), 0x3f9e_2020);
        assert!(both.ordered.text && both.ordered.title);
        let disabled = exact_phrase(&phrase, EnabledFields::Text, &phrase, &[]);
        assert_eq!(disabled, ProximityEvidence::default());
        let single = exact_phrase(
            &tokens("alpha"),
            EnabledFields::Title,
            &tokens("alpha"),
            &[],
        );
        assert_eq!(single.field_factor().to_bits(), 1.0_f32.to_bits());
        assert_eq!(single.ordered_factor().to_bits(), 1.25_f32.to_bits());
        let long = (0..12).map(|i| format!("t{i}")).collect::<Vec<_>>();
        let evidence = exact_phrase(&long, EnabledFields::Text, &[], &long);
        assert_eq!(evidence.field_factor().to_bits(), 0x3f9b_7291);
    }

    #[test]
    fn gap_and_early_position_are_bit_exact() {
        let adjacent = evaluate(
            &["alpha", "beta"],
            EnabledFields::Text,
            &[],
            &tokens("alpha beta"),
        );
        let gapped = evaluate(
            &["alpha", "beta"],
            EnabledFields::Text,
            &[],
            &tokens("skip skip alpha gap beta"),
        );
        assert_eq!(
            adjacent.fields.text.expect("text window").factor.to_bits(),
            0x3f9e_2020
        );
        assert_eq!(
            gapped.fields.text.expect("text window").factor.to_bits(),
            0x3f94_f6cf
        );
        assert!(adjacent.field_factor() > gapped.field_factor());

        let late_adjacent = evaluate(
            &["alpha", "beta"],
            EnabledFields::Text,
            &[],
            &tokens("skip skip alpha beta"),
        );
        assert_eq!(
            late_adjacent
                .fields
                .text
                .expect("late text window")
                .displacement,
            adjacent
                .fields
                .text
                .expect("early text window")
                .displacement
        );
        assert!(adjacent.field_factor() > late_adjacent.field_factor());
    }

    #[test]
    fn field_max_and_full_order_are_independent() {
        let evidence = evaluate(
            &["alpha", "beta"],
            EnabledFields::TextAndTitle,
            &tokens("skip alpha beta"),
            &tokens("alpha gap gap beta"),
        );
        assert_eq!(
            evidence.ordered,
            OrderedEvidence {
                text: false,
                title: true
            }
        );
        assert_eq!(
            evidence.field_factor().to_bits(),
            evidence
                .fields
                .title
                .expect("title window")
                .factor
                .to_bits()
        );

        let reversed = evaluate(
            &["alpha", "beta"],
            EnabledFields::Text,
            &[],
            &tokens("beta alpha"),
        );
        assert_eq!(reversed.ordered, OrderedEvidence::default());
        assert!(reversed.field_factor() > 1.0);

        let text_only = evaluate(
            &["alpha", "beta"],
            EnabledFields::Text,
            &tokens("alpha beta"),
            &tokens("beta alpha"),
        );
        assert!(text_only.fields.title.is_none());
        assert_eq!(text_only.ordered, OrderedEvidence::default());

        let title_only = evaluate(
            &["alpha", "beta"],
            EnabledFields::Title,
            &tokens("alpha beta"),
            &tokens("beta alpha"),
        );
        assert!(title_only.fields.text.is_none());
        assert_eq!(
            title_only.ordered,
            OrderedEvidence {
                text: false,
                title: true,
            }
        );
    }

    #[test]
    fn more_than_ten_terms_and_displacement_over_ten_are_not_capped() {
        let terms = (0..12).map(|index| format!("t{index}")).collect::<Vec<_>>();
        let refs = terms.iter().map(String::as_str).collect::<Vec<_>>();
        let evidence = evaluate(&refs, EnabledFields::Text, &[], &terms);
        assert_eq!(
            evidence.ordered,
            OrderedEvidence {
                text: true,
                title: false
            }
        );
        assert_eq!(evidence.fields.text.expect("text window").displacement, 11);
        assert_eq!(evidence.field_factor().to_bits(), 0x3f9b_7291);
        assert_eq!(evidence.ordered_factor().to_bits(), 1.25_f32.to_bits());
    }
}
