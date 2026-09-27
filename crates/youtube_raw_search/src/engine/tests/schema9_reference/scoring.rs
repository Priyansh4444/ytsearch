use std::collections::{BTreeMap, BTreeSet};

use super::{EnabledFields, IndexedField, tokenize};

const K1: f32 = 1.2;
const B: f32 = 0.75;
const TEXT_WEIGHT: f32 = 4.0;
const TITLE_WEIGHT: f32 = 6.0;

#[derive(Clone, Copy)]
pub(super) struct ScoringDocument<'a> {
    pub(super) video_id: &'a str,
    pub(super) title: &'a str,
    pub(super) text: &'a str,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TermStatistics {
    text_document_frequency: usize,
    title_video_frequency: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ScoringContext {
    document_count: usize,
    video_count: usize,
    average_text_tokens: f32,
    terms: BTreeMap<String, TermStatistics>,
}

impl ScoringContext {
    pub(super) fn from_documents<'a>(
        documents: impl IntoIterator<Item = ScoringDocument<'a>>,
    ) -> Self {
        let documents = documents.into_iter().collect::<Vec<_>>();
        let document_count = documents.len();
        let video_count = documents
            .iter()
            .map(|document| document.video_id)
            .collect::<BTreeSet<_>>()
            .len();
        let mut total_text_tokens = 0_usize;
        let mut text_documents = BTreeMap::<String, usize>::new();
        let mut title_videos = BTreeMap::<String, BTreeSet<&str>>::new();
        for document in &documents {
            let text = tokenize(document.text);
            total_text_tokens += text.len();
            for term in text.into_iter().collect::<BTreeSet<_>>() {
                *text_documents.entry(term).or_default() += 1;
            }
            for term in tokenize(document.title)
                .into_iter()
                .collect::<BTreeSet<_>>()
            {
                title_videos
                    .entry(term)
                    .or_default()
                    .insert(document.video_id);
            }
        }
        let mut terms = BTreeMap::new();
        for term in text_documents.keys().chain(title_videos.keys()) {
            terms.entry(term.clone()).or_insert_with(|| TermStatistics {
                text_document_frequency: text_documents.get(term).copied().unwrap_or_default(),
                title_video_frequency: title_videos.get(term).map_or(0, BTreeSet::len),
            });
        }
        let average_text_tokens = if document_count == 0 {
            0.0
        } else {
            total_text_tokens as f32 / document_count as f32
        };
        Self {
            document_count,
            video_count,
            average_text_tokens,
            terms,
        }
    }

    pub(super) fn score_field(
        &self,
        term: &str,
        field: IndexedField,
        enabled_fields: EnabledFields,
        expansion_multiplier: f32,
        title: &[String],
        text: &[String],
    ) -> Option<f32> {
        assert!(
            expansion_multiplier.is_finite(),
            "expansion multiplier must be finite"
        );
        if !enabled_fields.contains(field) {
            return None;
        }
        let tokens = match field {
            IndexedField::Text => text,
            IndexedField::Title => title,
        };
        let mut positions = tokens
            .iter()
            .enumerate()
            .filter_map(|(position, token)| (token == term).then_some(position));
        let first_position = positions.next()?;
        let term_frequency = 1 + positions.count();
        let idf = self.idf(term, enabled_fields);
        let position_multiplier = 1.0_f32 + 0.15_f32 / (first_position as f32 + 1.0_f32).sqrt();
        let score = match field {
            IndexedField::Text => {
                let term_frequency = term_frequency as f32;
                let length_ratio = if self.average_text_tokens == 0.0 {
                    1.0
                } else {
                    text.len() as f32 / self.average_text_tokens
                };
                let denominator = term_frequency + K1 * (1.0_f32 - B + B * length_ratio);
                let bm25_tf = term_frequency * (K1 + 1.0_f32) / denominator;
                let weighted = bm25_tf * idf * TEXT_WEIGHT * position_multiplier;
                weighted * expansion_multiplier
            }
            IndexedField::Title => {
                let frequency_factor = 1.0_f32 + (term_frequency as f32).ln();
                let weighted = idf * TITLE_WEIGHT * frequency_factor * position_multiplier;
                weighted * expansion_multiplier
            }
        };
        assert!(score.is_finite(), "reference field score must be finite");
        Some(score)
    }

    pub(super) fn idf(&self, term: &str, fields: EnabledFields) -> f32 {
        let statistics = self.terms.get(term).copied().unwrap_or_default();
        let title_documents_per_video = self
            .document_count
            .checked_div(self.video_count)
            .unwrap_or_default();
        let title_frequency = statistics
            .title_video_frequency
            .saturating_mul(title_documents_per_video);
        let mut effective_frequency = 0_usize;
        if fields.contains(IndexedField::Text) {
            effective_frequency =
                effective_frequency.saturating_add(statistics.text_document_frequency);
        }
        if fields.contains(IndexedField::Title) {
            effective_frequency = effective_frequency.saturating_add(title_frequency);
        }
        let effective_frequency = effective_frequency.min(self.document_count) as f32;
        let document_count = self.document_count as f32;
        let ratio =
            (document_count - effective_frequency + 0.5_f32) / (effective_frequency + 0.5_f32);
        (1.0_f32 + ratio).ln()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context<'a>(documents: &'a [(&'a str, &'a str, &'a str)]) -> ScoringContext {
        ScoringContext::from_documents(documents.iter().map(|(video_id, title, text)| {
            ScoringDocument {
                video_id,
                title,
                text,
            }
        }))
    }

    #[test]
    fn idf_combines_enabled_fields_and_deduplicates_title_videos() {
        let scoring = context(&[
            ("one", "alpha", "alpha"),
            ("one", "alpha", "other"),
            ("two", "other", "alpha"),
            ("three", "other", "other"),
        ]);
        // N=4: text DF=2, unique-video title DF=1, floor(4/3)=1.
        assert_eq!(
            scoring.idf("alpha", EnabledFields::Text).to_bits(),
            0x3f31_7218
        );
        assert_eq!(
            scoring.idf("alpha", EnabledFields::Title).to_bits(),
            0x3f9a_1bc8
        );
        assert_eq!(
            scoring.idf("alpha", EnabledFields::TextAndTitle).to_bits(),
            0x3eb6_9e1a
        );

        let clamped = context(&[("one", "alpha", "alpha"), ("two", "alpha", "alpha")]);
        assert_eq!(
            clamped.idf("alpha", EnabledFields::TextAndTitle).to_bits(),
            clamped.idf("alpha", EnabledFields::Text).to_bits(),
            "combined effective DF is clamped to N"
        );
    }

    #[test]
    fn text_length_position_and_final_expansion_multiplier_are_bit_exact() {
        let scoring = context(&[
            ("short", "fixture", "alpha"),
            ("long", "fixture", "x x x alpha"),
        ]);
        let early = scoring
            .score_field(
                "alpha",
                IndexedField::Text,
                EnabledFields::Text,
                1.0,
                &[],
                &tokenize("alpha"),
            )
            .unwrap();
        let long_early = scoring
            .score_field(
                "alpha",
                IndexedField::Text,
                EnabledFields::Text,
                1.0,
                &[],
                &tokenize("alpha x x x"),
            )
            .unwrap();
        let long_late = scoring
            .score_field(
                "alpha",
                IndexedField::Text,
                EnabledFields::Text,
                1.0,
                &[],
                &tokenize("x x x alpha"),
            )
            .unwrap();
        let expanded = scoring
            .score_field(
                "alpha",
                IndexedField::Text,
                EnabledFields::Text,
                0.65,
                &[],
                &tokenize("x x x alpha"),
            )
            .unwrap();
        assert!(early > long_early, "length normalization");
        assert!(long_early > long_late, "first-position preference");
        assert_eq!(early.to_bits(), 0x3f8e_45b8);
        assert_eq!(long_early.to_bits(), 0x3f2c_636d);
        assert_eq!(long_late.to_bits(), 0x3f21_254a);
        assert_eq!(expanded.to_bits(), 0x3ed1_7d46);
        assert_eq!(expanded.to_bits(), (long_late * 0.65_f32).to_bits());
        assert!(
            scoring
                .score_field(
                    "alpha",
                    IndexedField::Text,
                    EnabledFields::Title,
                    1.0,
                    &[],
                    &tokenize("alpha")
                )
                .is_none()
        );
    }

    #[test]
    fn title_log_frequency_and_empty_context_are_finite() {
        let scoring = context(&[("one", "alpha alpha", "")]);
        let title = scoring
            .score_field(
                "alpha",
                IndexedField::Title,
                EnabledFields::Title,
                0.9,
                &tokenize("alpha alpha"),
                &[],
            )
            .unwrap();
        assert_eq!(title.to_bits(), 0x4041_969c);
        let zero_average_text = scoring
            .score_field(
                "missing",
                IndexedField::Text,
                EnabledFields::Text,
                1.0,
                &[],
                &tokenize("missing"),
            )
            .unwrap();
        assert!(zero_average_text.is_finite());

        let empty = context(&[]);
        assert_eq!(
            empty.idf("missing", EnabledFields::Text).to_bits(),
            0x3f31_7218
        );
        assert!(
            empty
                .score_field(
                    "missing",
                    IndexedField::Text,
                    EnabledFields::Text,
                    1.0,
                    &[],
                    &[]
                )
                .is_none()
        );
    }
}
