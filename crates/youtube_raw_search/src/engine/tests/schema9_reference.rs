//! Independent exhaustive reference for the schema-9 exact-query subset.
//!
//! This module deliberately does not call the production parser, scorer,
//! posting decoder, sorter, or aggregator. It is slow by design: every request
//! scans every fixture document, constructs the complete result, and paginates
//! only after deterministic ordering.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

use serde::Deserialize;
use unicode_categories::UnicodeCategories;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ContentKind {
    Video,
    Short,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Present<T> {
    #[default]
    Missing,
    Value(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Present<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(deserializer).map(|value| match value {
            Some(value) => Self::Value(value),
            None => Self::Missing,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureDocument {
    id: String,
    video_id: String,
    creator: String,
    title: String,
    text: String,
    timestamp_ms: u32,
    content_kind: ContentKind,
    #[serde(default)]
    published_at: Present<i64>,
    #[serde(default)]
    views: Present<u64>,
    #[serde(default)]
    likes: Present<u64>,
    #[serde(default)]
    comments: Present<u64>,
}

#[derive(Clone, Debug)]
struct FixtureCorpus {
    documents: Vec<FixtureDocument>,
}

impl FixtureCorpus {
    fn from_jsonl(source: &str) -> Result<Self, String> {
        let mut documents = source
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<FixtureDocument>(line).map_err(|error| error.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        documents.sort_unstable_by(|left, right| left.id.cmp(&right.id));
        if let Some(document) = documents
            .iter()
            .find(|document| document.id.is_empty() || document.video_id.is_empty())
        {
            return Err(format!(
                "fixture contains an empty identity: {}",
                document.id
            ));
        }
        for pair in documents.windows(2) {
            if pair[0].id == pair[1].id {
                return Err(format!("duplicate fixture document ID: {}", pair[0].id));
            }
        }
        let mut videos = BTreeMap::<
            &str,
            (
                &str,
                ContentKind,
                Present<i64>,
                Present<u64>,
                Present<u64>,
                Present<u64>,
            ),
        >::new();
        for document in &documents {
            let metadata = (
                document.creator.as_str(),
                document.content_kind,
                document.published_at,
                document.views,
                document.likes,
                document.comments,
            );
            if let Some(existing) = videos.insert(&document.video_id, metadata)
                && existing != metadata
            {
                return Err(format!("conflicting video metadata: {}", document.video_id));
            }
        }
        Ok(Self { documents })
    }

    fn lexicon(&self, field: IndexedField) -> BTreeSet<String> {
        self.documents
            .iter()
            .flat_map(|document| match field {
                IndexedField::Text => tokenize(&document.text),
                IndexedField::Title => tokenize(&document.title),
            })
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Query {
    All,
    Term(String),
    Phrase(Vec<String>),
    And(Vec<Query>),
    Not(Box<Query>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Sort {
    Relevance,
    Newest,
    Oldest,
    Views,
    Likes,
    Comments,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnabledFields {
    Text,
    Title,
    TextAndTitle,
}

impl EnabledFields {
    fn contains(self, field: IndexedField) -> bool {
        matches!(
            (self, field),
            (Self::Text, IndexedField::Text)
                | (Self::Title, IndexedField::Title)
                | (Self::TextAndTitle, _)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpansionPolicy {
    ExactOnly,
    Prefix(EnabledFields),
    Fuzzy(EnabledFields),
    Split(EnabledFields),
    TokenDrop,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum IndexedField {
    Text,
    Title,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ExpansionKind {
    Exact,
    Prefix,
    TypoOne,
    TypoTwo,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TermAlternative {
    term: String,
    field: IndexedField,
    kind: ExpansionKind,
    distance: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TermPlan {
    alternatives: Vec<TermAlternative>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum HypothesisKind {
    ExactWhole,
    SplitExact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum HypothesisPlan {
    Whole(TermPlan),
    Split { left: String, right: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SlotHypothesis {
    kind: HypothesisKind,
    canonical_bytes: Vec<u8>,
    plan: HypothesisPlan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExpansionPlan {
    Terms {
        query: Query,
        fields: EnabledFields,
        terms: BTreeMap<String, TermPlan>,
    },
    Hypotheses {
        query: Query,
        fields: EnabledFields,
        slots: BTreeMap<String, Vec<SlotHypothesis>>,
    },
}

#[derive(Clone, Debug)]
struct OracleRequest {
    query: String,
    creator: Option<String>,
    sort: Sort,
    expansion: ExpansionPolicy,
    page: usize,
    per_page: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Provenance {
    title: bool,
    transcript: bool,
    phrase: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OracleHit {
    video_id: String,
    document_ids: Vec<String>,
    score_bits: u32,
    provenance: Provenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OracleResult {
    total_videos: usize,
    total_moments: usize,
    by_creator: BTreeMap<String, usize>,
    by_content_kind: BTreeMap<&'static str, usize>,
    hits: Vec<OracleHit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OracleError {
    EmptyQuery,
    PunctuationOnly,
    UnterminatedPhrase,
    UnsupportedReferenceSemantics(&'static str),
}

#[derive(Clone, Debug)]
struct DocumentMatch<'a> {
    document: &'a FixtureDocument,
    score: f32,
    provenance: Provenance,
}

fn build_expansion_plan(
    corpus: &FixtureCorpus,
    query: Query,
    policy: ExpansionPolicy,
) -> Result<ExpansionPlan, OracleError> {
    if let ExpansionPolicy::Split(fields) = policy {
        return build_split_plan(corpus, query, fields);
    }
    let fields = match policy {
        ExpansionPolicy::ExactOnly => {
            let terms = positive_terms(&query)
                .into_iter()
                .map(|term| {
                    (
                        term.to_string(),
                        exact_term_plan(term, EnabledFields::TextAndTitle),
                    )
                })
                .collect();
            return Ok(ExpansionPlan::Terms {
                query,
                fields: EnabledFields::TextAndTitle,
                terms,
            });
        }
        ExpansionPolicy::Prefix(fields) | ExpansionPolicy::Fuzzy(fields) => fields,
        ExpansionPolicy::Split(_) => unreachable!("split returned above"),
        ExpansionPolicy::TokenDrop => {
            return Err(OracleError::UnsupportedReferenceSemantics("token_drop"));
        }
    };
    let terms = positive_terms(&query);
    let final_term = terms.last().copied();
    let mut plans = BTreeMap::new();
    for term in terms {
        let mut alternatives = Vec::new();
        for field in [IndexedField::Text, IndexedField::Title] {
            if !fields.contains(field) {
                continue;
            }
            alternatives.push(TermAlternative {
                term: term.to_string(),
                field,
                kind: ExpansionKind::Exact,
                distance: 0,
            });
            match policy {
                ExpansionPolicy::Prefix(_)
                    if Some(term) == final_term && !is_pure_decimal(term) =>
                {
                    for candidate in corpus.lexicon(field) {
                        if candidate != term && candidate.starts_with(term) {
                            alternatives.push(TermAlternative {
                                term: candidate,
                                field,
                                kind: ExpansionKind::Prefix,
                                distance: 0,
                            });
                        }
                    }
                }
                ExpansionPolicy::Fuzzy(_) if !is_pure_decimal(term) => {
                    let scalar_count = term.chars().count();
                    let maximum_distance = if scalar_count >= 7 {
                        2
                    } else if scalar_count >= 4 {
                        1
                    } else {
                        0
                    };
                    for candidate in corpus.lexicon(field) {
                        if candidate == term {
                            continue;
                        }
                        let distance = levenshtein_scalars(term, &candidate);
                        if distance > 0 && distance <= maximum_distance {
                            alternatives.push(TermAlternative {
                                term: candidate,
                                field,
                                kind: if distance == 1 {
                                    ExpansionKind::TypoOne
                                } else {
                                    ExpansionKind::TypoTwo
                                },
                                distance,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        alternatives.sort_by(|left, right| {
            left.distance
                .cmp(&right.distance)
                .then_with(|| left.term.as_bytes().cmp(right.term.as_bytes()))
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.field.cmp(&right.field))
        });
        alternatives.dedup();
        plans.insert(term.to_string(), TermPlan { alternatives });
    }
    Ok(ExpansionPlan::Terms {
        query,
        fields,
        terms: plans,
    })
}

fn build_split_plan(
    corpus: &FixtureCorpus,
    query: Query,
    fields: EnabledFields,
) -> Result<ExpansionPlan, OracleError> {
    if query_contains_positive_phrase(&query) {
        return Err(OracleError::UnsupportedReferenceSemantics(
            "split_with_phrase",
        ));
    }
    let lexicons = [
        (IndexedField::Text, corpus.lexicon(IndexedField::Text)),
        (IndexedField::Title, corpus.lexicon(IndexedField::Title)),
    ];
    let appears_in_an_enabled_lexicon = |term: &str| {
        lexicons
            .iter()
            .any(|(field, lexicon)| fields.contains(*field) && lexicon.contains(term))
    };
    let mut slots = BTreeMap::new();
    for term in positive_terms(&query) {
        let mut hypotheses = vec![SlotHypothesis {
            kind: HypothesisKind::ExactWhole,
            canonical_bytes: term.as_bytes().to_vec(),
            plan: HypothesisPlan::Whole(exact_term_plan(term, fields)),
        }];
        if is_pure_decimal(term) {
            slots.insert(term.to_string(), hypotheses);
            continue;
        }
        let boundaries = term
            .char_indices()
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>();
        let scalar_count = boundaries.len();
        for (scalar_index, &byte_index) in boundaries.iter().enumerate().skip(1) {
            if scalar_index < 3 || scalar_count.saturating_sub(scalar_index) < 3 {
                continue;
            }
            let left = &term[..byte_index];
            let right = &term[byte_index..];
            if !appears_in_an_enabled_lexicon(left) || !appears_in_an_enabled_lexicon(right) {
                continue;
            }
            let mut canonical_bytes = left.as_bytes().to_vec();
            canonical_bytes.push(0);
            canonical_bytes.extend_from_slice(right.as_bytes());
            hypotheses.push(SlotHypothesis {
                kind: HypothesisKind::SplitExact,
                canonical_bytes,
                plan: HypothesisPlan::Split {
                    left: left.to_string(),
                    right: right.to_string(),
                },
            });
        }
        hypotheses.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.canonical_bytes.cmp(&right.canonical_bytes))
        });
        slots.insert(term.to_string(), hypotheses);
    }
    Ok(ExpansionPlan::Hypotheses {
        query,
        fields,
        slots,
    })
}

fn query_contains_positive_phrase(query: &Query) -> bool {
    match query {
        Query::Phrase(_) => true,
        Query::And(children) => children
            .iter()
            .filter(|child| !matches!(child, Query::Not(_)))
            .any(query_contains_positive_phrase),
        Query::All | Query::Term(_) | Query::Not(_) => false,
    }
}

fn exact_term_plan(term: &str, fields: EnabledFields) -> TermPlan {
    TermPlan {
        alternatives: [IndexedField::Text, IndexedField::Title]
            .into_iter()
            .filter(|field| fields.contains(*field))
            .map(|field| TermAlternative {
                term: term.to_string(),
                field,
                kind: ExpansionKind::Exact,
                distance: 0,
            })
            .collect(),
    }
}

fn positive_terms(query: &Query) -> Vec<&str> {
    fn visit<'a>(query: &'a Query, output: &mut Vec<&'a str>) {
        match query {
            Query::Term(term) => output.push(term),
            Query::Phrase(terms) => output.extend(terms.iter().map(String::as_str)),
            Query::And(children) => {
                for child in children {
                    if !matches!(child, Query::Not(_)) {
                        visit(child, output);
                    }
                }
            }
            Query::All | Query::Not(_) => {}
        }
    }
    let mut output = Vec::new();
    visit(query, &mut output);
    let mut seen = BTreeSet::new();
    output.retain(|term| seen.insert(*term));
    output
}

fn evaluate_plan(plan: &ExpansionPlan, title: &[String], text: &[String]) -> Trace {
    match plan {
        ExpansionPlan::Terms {
            query,
            fields,
            terms,
        } => evaluate_document_plan(query, *fields, SlotPlans::Terms(terms), title, text),
        ExpansionPlan::Hypotheses {
            query,
            fields,
            slots,
        } => evaluate_document_plan(query, *fields, SlotPlans::Hypotheses(slots), title, text),
    }
}

#[derive(Clone, Copy)]
enum SlotPlans<'a> {
    Terms(&'a BTreeMap<String, TermPlan>),
    Hypotheses(&'a BTreeMap<String, Vec<SlotHypothesis>>),
}

fn evaluate_document_plan(
    query: &Query,
    fields: EnabledFields,
    slots: SlotPlans<'_>,
    title: &[String],
    text: &[String],
) -> Trace {
    let membership = evaluate_membership(query, fields, slots, title, text);
    if !membership.matched {
        return Trace::default();
    }

    let positive_slots = positive_terms(query);
    let mut reduced = Trace {
        matched: true,
        provenance: membership.provenance,
        ..Trace::default()
    };
    for (index, term) in positive_slots.iter().enumerate() {
        let trace = evaluate_slot(term, fields, slots, title, text);
        if !trace.matched {
            return Trace::default();
        }
        reduced.score += trace.score;
        reduced.provenance.title |= trace.provenance.title;
        reduced.provenance.transcript |= trace.provenance.transcript;
        if positive_slots.len() == 1 && index == 0 {
            reduced.selected_hypothesis = trace.selected_hypothesis;
        }
    }
    if query_contains_positive_phrase(query) {
        reduced.score *= 1.25_f32;
        reduced.provenance.phrase = true;
    }
    reduced
}

fn evaluate_membership(
    query: &Query,
    fields: EnabledFields,
    slots: SlotPlans<'_>,
    title: &[String],
    text: &[String],
) -> Trace {
    match query {
        Query::All => Trace {
            matched: true,
            ..Trace::default()
        },
        Query::Term(term) => {
            let trace = evaluate_slot(term, fields, slots, title, text);
            Trace {
                matched: trace.matched,
                provenance: trace.provenance,
                ..Trace::default()
            }
        }
        Query::Phrase(_) => {
            let trace = evaluate_query_in_fields(query, title, text, fields);
            Trace {
                matched: trace.matched,
                provenance: trace.provenance,
                ..Trace::default()
            }
        }
        Query::And(children) => {
            let mut combined = Trace {
                matched: true,
                ..Trace::default()
            };
            for child in children {
                let trace = if matches!(child, Query::Not(_)) {
                    let exact = evaluate_query_in_fields(child, title, text, fields);
                    Trace {
                        matched: exact.matched,
                        ..Trace::default()
                    }
                } else {
                    evaluate_membership(child, fields, slots, title, text)
                };
                if !trace.matched {
                    return Trace::default();
                }
                combined.provenance.title |= trace.provenance.title;
                combined.provenance.transcript |= trace.provenance.transcript;
                combined.provenance.phrase |= trace.provenance.phrase;
            }
            combined
        }
        Query::Not(_) => {
            let exact = evaluate_query_in_fields(query, title, text, fields);
            Trace {
                matched: exact.matched,
                ..Trace::default()
            }
        }
    }
}

fn evaluate_slot(
    term: &str,
    fields: EnabledFields,
    slots: SlotPlans<'_>,
    title: &[String],
    text: &[String],
) -> Trace {
    match slots {
        SlotPlans::Terms(plans) => plans.get(term).map_or_else(
            || evaluate_query_in_fields(&Query::Term(term.to_string()), title, text, fields),
            |plan| evaluate_term_plan(plan, title, text),
        ),
        SlotPlans::Hypotheses(plans) => plans.get(term).map_or_else(
            || evaluate_query_in_fields(&Query::Term(term.to_string()), title, text, fields),
            |hypotheses| select_hypothesis(hypotheses, fields, title, text),
        ),
    }
}

fn select_hypothesis(
    hypotheses: &[SlotHypothesis],
    fields: EnabledFields,
    title: &[String],
    text: &[String],
) -> Trace {
    let mut candidates = hypotheses
        .iter()
        .filter_map(|hypothesis| {
            let mut trace = match &hypothesis.plan {
                HypothesisPlan::Whole(plan) => evaluate_term_plan(plan, title, text),
                HypothesisPlan::Split { left, right } => {
                    evaluate_exact_split(left, right, fields, title, text)
                }
            };
            if !trace.matched {
                return None;
            }
            assert!(trace.score.is_finite(), "reference score must be finite");
            trace.selected_hypothesis = Some(HypothesisSelection {
                kind: hypothesis.kind,
                canonical_bytes: hypothesis.canonical_bytes.clone(),
            });
            Some(trace)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right.score.total_cmp(&left.score).then_with(|| {
            let left = left
                .selected_hypothesis
                .as_ref()
                .expect("candidate selection");
            let right = right
                .selected_hypothesis
                .as_ref()
                .expect("candidate selection");
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.canonical_bytes.cmp(&right.canonical_bytes))
        })
    });
    candidates.into_iter().next().unwrap_or_default()
}

fn evaluate_exact_split(
    left: &str,
    right: &str,
    fields: EnabledFields,
    title: &[String],
    text: &[String],
) -> Trace {
    let occurrences = |field, term: &str| {
        let tokens = match field {
            IndexedField::Text => text,
            IndexedField::Title => title,
        };
        tokens.iter().filter(|token| token.as_str() == term).count() as f32
    };
    let left_fields = [IndexedField::Text, IndexedField::Title]
        .into_iter()
        .filter(|field| fields.contains(*field) && occurrences(*field, left) > 0.0)
        .collect::<Vec<_>>();
    let right_fields = [IndexedField::Text, IndexedField::Title]
        .into_iter()
        .filter(|field| fields.contains(*field) && occurrences(*field, right) > 0.0)
        .collect::<Vec<_>>();
    if left_fields.is_empty() || right_fields.is_empty() {
        return Trace::default();
    }
    let weight = |field| match field {
        IndexedField::Text => 4.0_f32,
        IndexedField::Title => 6.0_f32,
    };
    let mut score = 0.0_f32;
    for field in &left_fields {
        score += occurrences(*field, left) * weight(*field);
    }
    for field in &right_fields {
        score += occurrences(*field, right) * weight(*field);
    }
    score += 8.0_f32;
    let mut provenance = Provenance::default();
    for field in left_fields.into_iter().chain(right_fields) {
        provenance.text_or_title(field);
    }
    Trace {
        matched: true,
        score,
        provenance,
        selected_hypothesis: None,
    }
}

fn evaluate_term_plan(plan: &TermPlan, title: &[String], text: &[String]) -> Trace {
    let mut combined = Trace::default();
    for alternative in &plan.alternatives {
        let tokens = match alternative.field {
            IndexedField::Text => text,
            IndexedField::Title => title,
        };
        let frequency = tokens
            .iter()
            .filter(|token| *token == &alternative.term)
            .count() as f32;
        if frequency == 0.0 {
            continue;
        }
        let weight = match alternative.field {
            IndexedField::Text => 4.0,
            IndexedField::Title => 6.0,
        };
        let multiplier = match alternative.kind {
            ExpansionKind::Exact => 1.0,
            ExpansionKind::Prefix => 0.9,
            ExpansionKind::TypoOne => 0.65,
            ExpansionKind::TypoTwo => 0.4,
        };
        combined.matched = true;
        combined.score += (frequency * weight) * multiplier;
        combined.provenance.text_or_title(alternative.field);
    }
    combined
}

impl Provenance {
    fn text_or_title(&mut self, field: IndexedField) {
        match field {
            IndexedField::Text => self.transcript = true,
            IndexedField::Title => self.title = true,
        }
    }
}

fn is_pure_decimal(term: &str) -> bool {
    !term.is_empty()
        && term
            .chars()
            .all(|character| character.is_number_decimal_digit())
}

fn levenshtein_scalars(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != *right_character);
            current[right_index + 1] = (current[right_index] + 1)
                .min(previous[right_index + 1] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn evaluate_exhaustively(
    corpus: &FixtureCorpus,
    request: &OracleRequest,
) -> Result<OracleResult, OracleError> {
    if matches!(request.expansion, ExpansionPolicy::TokenDrop) {
        return Err(OracleError::UnsupportedReferenceSemantics("token_drop"));
    }
    let query = parse_query(&request.query)?;
    let plan = build_expansion_plan(corpus, query, request.expansion)?;
    let creator = request.creator.as_deref().map(normalize_handle);
    let mut matches = Vec::new();
    for document in &corpus.documents {
        if creator
            .as_deref()
            .is_some_and(|expected| normalize_handle(&document.creator) != expected)
        {
            continue;
        }
        let title = tokenize(&document.title);
        let text = tokenize(&document.text);
        let trace = evaluate_plan(&plan, &title, &text);
        if trace.matched {
            matches.push(DocumentMatch {
                document,
                score: trace.score,
                provenance: trace.provenance,
            });
        }
    }

    let total_moments = count_moments(&matches);
    let mut groups = BTreeMap::<&str, Vec<DocumentMatch<'_>>>::new();
    for matched in matches {
        groups
            .entry(&matched.document.video_id)
            .or_default()
            .push(matched);
    }

    let mut by_creator = BTreeMap::new();
    let mut by_content_kind = BTreeMap::new();
    for documents in groups.values() {
        let video = documents.first().expect("non-empty video group").document;
        *by_creator
            .entry(normalize_handle(&video.creator))
            .or_insert(0) += 1;
        let kind = match video.content_kind {
            ContentKind::Video => "video",
            ContentKind::Short => "short",
        };
        *by_content_kind.entry(kind).or_insert(0) += 1;
    }

    let mut ranked = groups
        .into_values()
        .map(|mut documents| {
            documents.sort_unstable_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| left.document.id.cmp(&right.document.id))
            });
            documents
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| compare_groups(left, right, request.sort));
    let total_videos = ranked.len();
    let start = request
        .page
        .saturating_sub(1)
        .saturating_mul(request.per_page);
    let hits = ranked
        .into_iter()
        .skip(start)
        .take(request.per_page)
        .map(|documents| {
            let best = &documents[0];
            OracleHit {
                video_id: best.document.video_id.clone(),
                document_ids: documents
                    .iter()
                    .map(|matched| matched.document.id.clone())
                    .collect(),
                score_bits: best.score.to_bits(),
                provenance: documents
                    .iter()
                    .fold(Provenance::default(), |mut all, matched| {
                        all.title |= matched.provenance.title;
                        all.transcript |= matched.provenance.transcript;
                        all.phrase |= matched.provenance.phrase;
                        all
                    }),
            }
        })
        .collect();
    Ok(OracleResult {
        total_videos,
        total_moments,
        by_creator,
        by_content_kind,
        hits,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HypothesisSelection {
    kind: HypothesisKind,
    canonical_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
struct Trace {
    matched: bool,
    score: f32,
    provenance: Provenance,
    selected_hypothesis: Option<HypothesisSelection>,
}

fn evaluate_query_in_fields(
    query: &Query,
    title: &[String],
    text: &[String],
    fields: EnabledFields,
) -> Trace {
    match query {
        Query::All => Trace {
            matched: true,
            ..Trace::default()
        },
        Query::Term(term) => {
            let title_count = if fields.contains(IndexedField::Title) {
                title.iter().filter(|token| *token == term).count() as f32
            } else {
                0.0
            };
            let text_count = if fields.contains(IndexedField::Text) {
                text.iter().filter(|token| *token == term).count() as f32
            } else {
                0.0
            };
            Trace {
                matched: title_count > 0.0 || text_count > 0.0,
                score: title_count * 6.0 + text_count * 4.0,
                provenance: Provenance {
                    title: title_count > 0.0,
                    transcript: text_count > 0.0,
                    phrase: false,
                },
                selected_hypothesis: None,
            }
        }
        Query::Phrase(terms) => {
            let title_match = fields.contains(IndexedField::Title) && contains_phrase(title, terms);
            let text_match = fields.contains(IndexedField::Text) && contains_phrase(text, terms);
            Trace {
                matched: title_match || text_match,
                score: (u8::from(title_match) as f32 * 6.0 + u8::from(text_match) as f32 * 4.0)
                    * 1.25,
                provenance: Provenance {
                    title: title_match,
                    transcript: text_match,
                    phrase: title_match || text_match,
                },
                selected_hypothesis: None,
            }
        }
        Query::And(children) => {
            let mut combined = Trace {
                matched: true,
                ..Trace::default()
            };
            for child in children {
                let trace = evaluate_query_in_fields(child, title, text, fields);
                if !trace.matched {
                    combined.matched = false;
                    return combined;
                }
                combined.score += trace.score;
                combined.provenance.title |= trace.provenance.title;
                combined.provenance.transcript |= trace.provenance.transcript;
                combined.provenance.phrase |= trace.provenance.phrase;
            }
            combined
        }
        Query::Not(child) => {
            let excluded = evaluate_query_in_fields(child, title, text, fields);
            Trace {
                matched: !excluded.matched,
                ..Trace::default()
            }
        }
    }
}

fn parse_query(source: &str) -> Result<Query, OracleError> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err(OracleError::EmptyQuery);
    }
    if trimmed == "*" {
        return Ok(Query::All);
    }
    if trimmed.contains('*') {
        return Err(OracleError::UnsupportedReferenceSemantics("prefix"));
    }
    if trimmed.contains('~') {
        return Err(OracleError::UnsupportedReferenceSemantics("fuzzy"));
    }
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    let mut cursor = 0;
    let chars = trimmed.char_indices().collect::<Vec<_>>();
    while cursor < chars.len() {
        while cursor < chars.len() && chars[cursor].1.is_whitespace() {
            cursor += 1;
        }
        if cursor == chars.len() {
            break;
        }
        let mut excluded = false;
        if chars[cursor].1 == '-' {
            excluded = true;
            cursor += 1;
            if cursor == chars.len() {
                return Err(OracleError::PunctuationOnly);
            }
        }
        let node = if chars[cursor].1 == '"' {
            cursor += 1;
            let start = cursor;
            while cursor < chars.len() && chars[cursor].1 != '"' {
                cursor += 1;
            }
            if cursor == chars.len() {
                return Err(OracleError::UnterminatedPhrase);
            }
            let byte_start = chars.get(start).map_or(trimmed.len(), |entry| entry.0);
            let byte_end = chars[cursor].0;
            cursor += 1;
            let terms = tokenize(&trimmed[byte_start..byte_end]);
            if terms.is_empty() {
                return Err(OracleError::PunctuationOnly);
            }
            Query::Phrase(terms)
        } else {
            let start = cursor;
            while cursor < chars.len() && !chars[cursor].1.is_whitespace() {
                cursor += 1;
            }
            let byte_start = chars[start].0;
            let byte_end = chars.get(cursor).map_or(trimmed.len(), |entry| entry.0);
            let mut terms = tokenize(&trimmed[byte_start..byte_end]);
            if terms.is_empty() {
                return Err(OracleError::PunctuationOnly);
            }
            if terms.len() == 1 {
                Query::Term(terms.remove(0))
            } else {
                Query::And(terms.into_iter().map(Query::Term).collect())
            }
        };
        if excluded {
            negative.push(Query::Not(Box::new(node)));
        } else {
            positive.push(node);
        }
    }
    if positive.is_empty() && negative.is_empty() {
        return Err(OracleError::PunctuationOnly);
    }
    if positive.is_empty() {
        positive.push(Query::All);
    }
    positive.extend(negative);
    Ok(if positive.len() == 1 {
        positive.remove(0)
    } else {
        Query::And(positive)
    })
}

fn tokenize(source: &str) -> Vec<String> {
    let normalized = source.nfc().collect::<String>();
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

fn contains_phrase(tokens: &[String], phrase: &[String]) -> bool {
    !phrase.is_empty() && tokens.windows(phrase.len()).any(|window| window == phrase)
}

fn normalize_handle(handle: &str) -> String {
    handle.trim_start_matches('@').to_ascii_lowercase()
}

fn count_moments(matches: &[DocumentMatch<'_>]) -> usize {
    let mut by_video = BTreeMap::<&str, Vec<u32>>::new();
    for matched in matches {
        let times = by_video.entry(&matched.document.video_id).or_default();
        if matched.provenance.transcript {
            times.push(matched.document.timestamp_ms);
        }
    }
    by_video
        .into_values()
        .map(|mut times| {
            if times.is_empty() {
                return 1;
            }
            times.sort_unstable();
            let mut count = 0;
            let mut anchor = None;
            for time in times {
                if anchor.is_none_or(|start| time.saturating_sub(start) > 20_000) {
                    count += 1;
                    anchor = Some(time);
                }
            }
            count
        })
        .sum()
}

fn compare_groups(left: &[DocumentMatch<'_>], right: &[DocumentMatch<'_>], sort: Sort) -> Ordering {
    let left_best = &left[0];
    let right_best = &right[0];
    let primary = match sort {
        Sort::Relevance => right_best.score.total_cmp(&left_best.score),
        Sort::Newest => compare_optional(
            left_best.document.published_at,
            right_best.document.published_at,
            true,
        ),
        Sort::Oldest => compare_optional(
            left_best.document.published_at,
            right_best.document.published_at,
            false,
        ),
        Sort::Views => compare_optional(left_best.document.views, right_best.document.views, true),
        Sort::Likes => compare_optional(left_best.document.likes, right_best.document.likes, true),
        Sort::Comments => compare_optional(
            left_best.document.comments,
            right_best.document.comments,
            true,
        ),
    };
    primary
        .then_with(|| right_best.score.total_cmp(&left_best.score))
        .then_with(|| {
            left_best
                .document
                .video_id
                .cmp(&right_best.document.video_id)
        })
}

fn compare_optional<T: Ord>(left: Present<T>, right: Present<T>, descending: bool) -> Ordering {
    match (left, right) {
        (Present::Value(left), Present::Value(right)) if descending => right.cmp(&left),
        (Present::Value(left), Present::Value(right)) => left.cmp(&right),
        (Present::Value(_), Present::Missing) => Ordering::Less,
        (Present::Missing, Present::Value(_)) => Ordering::Greater,
        (Present::Missing, Present::Missing) => Ordering::Equal,
    }
}

trait CandidateImplementation {
    fn search(&self, request: &OracleRequest) -> Result<OracleResult, OracleError>;
}

fn first_mismatch(
    expected: &OracleResult,
    candidate: &dyn CandidateImplementation,
    request: &OracleRequest,
) -> Option<&'static str> {
    let actual = match candidate.search(request) {
        Ok(actual) => actual,
        Err(_) => return Some("candidate_error"),
    };
    if expected.total_videos != actual.total_videos {
        return Some("total_videos");
    }
    if expected.total_moments != actual.total_moments {
        return Some("total_moments");
    }
    if expected.hits != actual.hits {
        return Some("hits");
    }
    if expected.by_creator != actual.by_creator {
        return Some("by_creator");
    }
    if expected.by_content_kind != actual.by_content_kind {
        return Some("by_content_kind");
    }
    None
}

const FIXTURE: &str = include_str!("../../../../../tests/schema9/fixtures/reference-small.jsonl");

fn request(query: &str, per_page: usize) -> OracleRequest {
    OracleRequest {
        query: query.to_string(),
        creator: None,
        sort: Sort::Relevance,
        expansion: ExpansionPolicy::ExactOnly,
        page: 1,
        per_page,
    }
}

#[test]
fn fixture_preserves_missing_and_real_zero() {
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    assert!(
        corpus
            .documents
            .iter()
            .any(|document| document.views == Present::Missing)
    );
    assert!(
        corpus
            .documents
            .iter()
            .any(|document| document.views == Present::Value(0))
    );
}

#[test]
fn normalization_and_repeated_phrase_are_exhaustive() {
    assert_eq!(tokenize("Cafe\u{301}"), tokenize("Café"));
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let nfc = evaluate_exhaustively(&corpus, &request("Café", 10)).unwrap();
    let nfd = evaluate_exhaustively(&corpus, &request("Cafe\u{301}", 10)).unwrap();
    assert_eq!(nfc, nfd);
    assert_eq!(nfc.hits[0].video_id, "cafe");
    let result = evaluate_exhaustively(&corpus, &request("\"the the\"", 10)).unwrap();
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.hits[0].video_id, "repeat");
    assert!(result.hits[0].provenance.phrase);
}

#[test]
fn negation_is_applied_before_totals() {
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let all = evaluate_exhaustively(&corpus, &request("create", 10)).unwrap();
    let excluded = evaluate_exhaustively(&corpus, &request("create -visual", 10)).unwrap();
    assert_eq!(all.total_videos, 2);
    assert_eq!(excluded.total_videos, 1);
    assert_eq!(excluded.hits[0].video_id, "solid");
}

#[test]
fn missing_numeric_values_sort_after_real_zero() {
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let mut by_views = request("rank", 10);
    by_views.sort = Sort::Views;
    let result = evaluate_exhaustively(&corpus, &by_views).unwrap();
    let videos = result
        .hits
        .iter()
        .map(|hit| hit.video_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(videos, ["rank-high", "rank-zero", "rank-missing"]);
}

#[test]
fn all_sort_variants_keep_missing_last() {
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    for sort in [
        Sort::Newest,
        Sort::Oldest,
        Sort::Views,
        Sort::Likes,
        Sort::Comments,
    ] {
        let mut ranked = request("rank", 10);
        ranked.sort = sort;
        let result = evaluate_exhaustively(&corpus, &ranked).unwrap();
        assert_eq!(result.hits.last().unwrap().video_id, "rank-missing");
    }
}

#[test]
fn complete_order_is_independent_of_page_size() {
    let mut corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let prototype = corpus
        .documents
        .iter()
        .find(|document| document.video_id == "rank-high")
        .unwrap()
        .clone();
    for index in 0..22 {
        let mut document = prototype.clone();
        document.id = format!("rank-extra-{index:02}");
        document.video_id = format!("rank-extra-{index:02}");
        document.views = Present::Value(index);
        corpus.documents.push(document);
    }
    let expected = evaluate_exhaustively(&corpus, &request("rank", 100)).unwrap();
    let expected_ids = expected
        .hits
        .iter()
        .map(|hit| hit.video_id.clone())
        .collect::<Vec<_>>();
    for per_page in [1, 3, 10, 20] {
        let mut ids = Vec::new();
        let pages = expected.total_videos.div_ceil(per_page);
        for page in 1..=pages {
            let mut page_request = request("rank", per_page);
            page_request.page = page;
            ids.extend(
                evaluate_exhaustively(&corpus, &page_request)
                    .unwrap()
                    .hits
                    .into_iter()
                    .map(|hit| hit.video_id),
            );
        }
        assert_eq!(ids, expected_ids, "per_page={per_page}");
    }
}

#[test]
fn creator_scope_and_zero_window_keep_exact_complete_totals() {
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let mut scoped = request("create", 0);
    scoped.creator = Some("@RyanSolid".to_string());
    let result = evaluate_exhaustively(&corpus, &scoped).unwrap();
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.total_moments, 1);
    assert_eq!(
        result.by_creator,
        BTreeMap::from([("ryansolid".to_string(), 1)])
    );
    assert_eq!(result.by_content_kind, BTreeMap::from([("video", 1)]));
    assert!(result.hits.is_empty());

    let title_only = evaluate_exhaustively(&corpus, &request("visual", 10)).unwrap();
    assert_eq!(title_only.total_videos, 1);
    assert_eq!(title_only.total_moments, 1);
    assert!(title_only.hits[0].provenance.title);
    assert!(!title_only.hits[0].provenance.transcript);
}

#[test]
fn punctuation_and_unsupported_semantics_fail_closed() {
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    assert_eq!(
        evaluate_exhaustively(&corpus, &request("🦀", 10)),
        Err(OracleError::PunctuationOnly)
    );
    assert_eq!(
        evaluate_exhaustively(&corpus, &request("creat~1", 10)),
        Err(OracleError::UnsupportedReferenceSemantics("fuzzy"))
    );
    assert_eq!(
        evaluate_exhaustively(&corpus, &request("creat*", 10)),
        Err(OracleError::UnsupportedReferenceSemantics("prefix"))
    );
    assert!(evaluate_exhaustively(&corpus, &request("create-effect", 10)).is_ok());
    let mut expanded = request("create", 10);
    expanded.expansion = ExpansionPolicy::TokenDrop;
    assert_eq!(
        evaluate_exhaustively(&corpus, &expanded),
        Err(OracleError::UnsupportedReferenceSemantics("token_drop"))
    );
    let mut phrase_split = request("\"create effect\"", 10);
    phrase_split.expansion = ExpansionPolicy::Split(EnabledFields::TextAndTitle);
    assert_eq!(
        evaluate_exhaustively(&corpus, &phrase_split),
        Err(OracleError::UnsupportedReferenceSemantics(
            "split_with_phrase"
        ))
    );
}

#[test]
fn prefix_expansion_is_complete_and_field_specific() {
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let mut text = request("creat", 10);
    text.expansion = ExpansionPolicy::Prefix(EnabledFields::Text);
    let text_result = evaluate_exhaustively(&corpus, &text).unwrap();
    assert_eq!(text_result.total_videos, 1);
    assert_eq!(text_result.hits[0].video_id, "solid");

    let mut title = text.clone();
    title.expansion = ExpansionPolicy::Prefix(EnabledFields::Title);
    let title_result = evaluate_exhaustively(&corpus, &title).unwrap();
    assert_eq!(title_result.total_videos, 2);

    let mut both = text;
    both.expansion = ExpansionPolicy::Prefix(EnabledFields::TextAndTitle);
    let both_result = evaluate_exhaustively(&corpus, &both).unwrap();
    assert_eq!(both_result.total_videos, 2);
    assert!(both_result.hits.iter().any(|hit| hit.provenance.title));
    assert!(both_result.hits.iter().any(|hit| hit.provenance.transcript));

    let phrase_query = parse_query("creat \"effect systems\"").unwrap();
    let phrase_plan = build_expansion_plan(
        &corpus,
        phrase_query,
        ExpansionPolicy::Prefix(EnabledFields::TextAndTitle),
    )
    .unwrap();
    let ExpansionPlan::Terms { terms, .. } = phrase_plan else {
        panic!("prefix must produce a typed expansion plan");
    };
    assert!(
        terms["creat"]
            .alternatives
            .iter()
            .all(|alternative| alternative.kind == ExpansionKind::Exact)
    );
    assert_eq!(
        positive_terms(&parse_query("alpha alpha \"beta gamma\"").unwrap()),
        ["alpha", "beta", "gamma"]
    );
}

#[test]
fn prefix_keeps_unicode_decimal_terms_exact_but_expands_mixed_alphanumeric() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let prototype = base.documents[0].clone();
    let mut documents = Vec::new();
    for (field_name, in_title) in [("text", false), ("title", true)] {
        for (name, term) in [
            ("ascii-exact", "2024"),
            ("ascii-extended", "20245"),
            ("unicode-exact", "٢٠٢٤"),
            ("unicode-extended", "٢٠٢٤٥"),
            ("mixed-exact", "ab12"),
            ("mixed-extended", "ab123"),
        ] {
            let mut document = prototype.clone();
            document.id = format!("{field_name}-{name}");
            document.video_id = document.id.clone();
            document.title = if in_title {
                term.to_string()
            } else {
                "fixture".to_string()
            };
            document.text = if in_title {
                "fixture".to_string()
            } else {
                term.to_string()
            };
            documents.push(document);
        }
    }
    let corpus = FixtureCorpus { documents };

    for (fields, exact_count, mixed_count) in [
        (EnabledFields::Text, 1, 2),
        (EnabledFields::Title, 1, 2),
        (EnabledFields::TextAndTitle, 2, 4),
    ] {
        for query in ["2024", "٢٠٢٤"] {
            let mut numeric = request(query, 10);
            numeric.expansion = ExpansionPolicy::Prefix(fields);
            assert_eq!(
                evaluate_exhaustively(&corpus, &numeric)
                    .unwrap()
                    .total_videos,
                exact_count,
                "query={query}, fields={fields:?}"
            );
        }

        let mut mixed = request("ab12", 10);
        mixed.expansion = ExpansionPolicy::Prefix(fields);
        assert_eq!(
            evaluate_exhaustively(&corpus, &mixed).unwrap().total_videos,
            mixed_count,
            "mixed fields={fields:?}"
        );
    }
}

#[test]
fn fuzzy_expansion_obeys_complete_unicode_threshold_and_numeric_contract() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let prototype = base.documents[0].clone();
    let mut documents = Vec::new();
    let terms = [
        "aaab",
        "aaac",
        "aaad",
        "aaae",
        "aaaf",
        "aaag",
        "cafe",
        "ééééééab",
        "ab13",
        "2024",
        "2025",
        "٢٠٢٤",
        "٢٠٢٥",
    ];
    for (index, term) in terms.into_iter().enumerate() {
        let mut document = prototype.clone();
        document.id = format!("fuzzy-{index}");
        document.video_id = format!("fuzzy-{index}");
        document.title = "fixture".to_string();
        document.text = term.to_string();
        documents.push(document);
    }
    let corpus = FixtureCorpus { documents };

    let mut fuzzy = request("aaaa", 10);
    fuzzy.expansion = ExpansionPolicy::Fuzzy(EnabledFields::Text);
    let fuzzy_result = evaluate_exhaustively(&corpus, &fuzzy).unwrap();
    assert_eq!(fuzzy_result.total_videos, 6);

    for (query, expected_video) in [
        ("café", "fuzzy-6"),
        ("ééééééé", "fuzzy-7"),
        ("ab12", "fuzzy-8"),
    ] {
        let mut request = request(query, 10);
        request.expansion = ExpansionPolicy::Fuzzy(EnabledFields::Text);
        let result = evaluate_exhaustively(&corpus, &request).unwrap();
        assert_eq!(result.total_videos, 1, "query={query}");
        assert_eq!(result.hits[0].video_id, expected_video, "query={query}");
    }

    for (query, expected_video) in [("2024", "fuzzy-9"), ("٢٠٢٤", "fuzzy-11")] {
        let mut request = request(query, 10);
        request.expansion = ExpansionPolicy::Fuzzy(EnabledFields::Text);
        let result = evaluate_exhaustively(&corpus, &request).unwrap();
        assert_eq!(result.total_videos, 1, "query={query}");
        assert_eq!(result.hits[0].video_id, expected_video, "query={query}");
    }
}

#[test]
fn expansion_order_scoring_and_negation_are_exact() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let prototype = base.documents[0].clone();
    let mut document = prototype.clone();
    document.id = "combined".to_string();
    document.video_id = "combined".to_string();
    document.title = "aaab".to_string();
    document.text = "aaac aaab aaac".to_string();
    let corpus = FixtureCorpus {
        documents: vec![document],
    };

    let query = parse_query("aaaa").unwrap();
    let plan = build_expansion_plan(
        &corpus,
        query,
        ExpansionPolicy::Fuzzy(EnabledFields::TextAndTitle),
    )
    .unwrap();
    let ExpansionPlan::Terms { terms, .. } = &plan else {
        panic!("fuzzy must produce a typed expansion plan");
    };
    assert_eq!(
        terms["aaaa"]
            .alternatives
            .iter()
            .map(|alternative| {
                (
                    alternative.distance,
                    alternative.term.as_str(),
                    alternative.field,
                    alternative.kind,
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (0, "aaaa", IndexedField::Text, ExpansionKind::Exact),
            (0, "aaaa", IndexedField::Title, ExpansionKind::Exact),
            (1, "aaab", IndexedField::Text, ExpansionKind::TypoOne),
            (1, "aaab", IndexedField::Title, ExpansionKind::TypoOne),
            (1, "aaac", IndexedField::Text, ExpansionKind::TypoOne),
        ]
    );
    let trace = evaluate_plan(&plan, &tokenize("aaab"), &tokenize("aaac aaab aaac"));
    assert_eq!(
        trace.score.to_bits(),
        (3.0_f32 * 4.0 * 0.65 + 6.0 * 0.65).to_bits()
    );

    let mut negative_neighbor = prototype.clone();
    negative_neighbor.id = "negative-neighbor".to_string();
    negative_neighbor.video_id = "negative-neighbor".to_string();
    negative_neighbor.title = "fixture".to_string();
    negative_neighbor.text = "aaad aaac".to_string();
    let mut exact_negative = prototype;
    exact_negative.id = "exact-negative".to_string();
    exact_negative.video_id = "exact-negative".to_string();
    exact_negative.title = "fixture".to_string();
    exact_negative.text = "aaad aaab".to_string();
    let negation_corpus = FixtureCorpus {
        documents: vec![negative_neighbor, exact_negative],
    };
    let mut request = request("aaad -aaab", 10);
    request.expansion = ExpansionPolicy::Fuzzy(EnabledFields::Text);
    let result = evaluate_exhaustively(&negation_corpus, &request).unwrap();
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.hits[0].video_id, "negative-neighbor");
}

#[test]
fn expansion_field_scope_applies_to_phrases_and_exact_negation() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let prototype = base.documents[0].clone();
    let mut title_phrase = prototype.clone();
    title_phrase.id = "title-phrase".to_string();
    title_phrase.video_id = "title-phrase".to_string();
    title_phrase.title = "anchor alpha bet beta".to_string();
    title_phrase.text = "fixture".to_string();
    let mut text_phrase = prototype;
    text_phrase.id = "text-phrase".to_string();
    text_phrase.video_id = "text-phrase".to_string();
    text_phrase.title = "anchor".to_string();
    text_phrase.text = "alpha bet beta".to_string();
    let corpus = FixtureCorpus {
        documents: vec![title_phrase, text_phrase],
    };

    let mut phrase = request("\"alpha bet\"", 10);
    phrase.expansion = ExpansionPolicy::Prefix(EnabledFields::Title);
    let phrase_result = evaluate_exhaustively(&corpus, &phrase).unwrap();
    assert_eq!(phrase_result.total_videos, 1);
    assert_eq!(phrase_result.hits[0].video_id, "title-phrase");
    let exact_result = evaluate_exhaustively(&corpus, &request("\"alpha bet\"", 10)).unwrap();
    assert!(phrase_result.hits[0].score_bits != exact_result.hits[0].score_bits);

    let mut negation = request("anchor -\"alpha bet\"", 10);
    negation.expansion = ExpansionPolicy::Fuzzy(EnabledFields::Title);
    let negation_result = evaluate_exhaustively(&corpus, &negation).unwrap();
    assert_eq!(negation_result.total_videos, 1);
    assert_eq!(negation_result.hits[0].video_id, "text-phrase");
}

#[test]
fn positive_slots_are_global_distinct_and_negative_slots_never_score() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let prototype = base.documents[0].clone();
    let mut text_document = prototype.clone();
    text_document.id = "slot-text".to_string();
    text_document.video_id = "slot-text".to_string();
    text_document.title = "fixture".to_string();
    text_document.text = "alpha alpha beta gamma".to_string();
    let mut title_document = prototype;
    title_document.id = "slot-title".to_string();
    title_document.video_id = "slot-title".to_string();
    title_document.title = "alpha alpha beta gamma".to_string();
    title_document.text = "fixture".to_string();
    let corpus = FixtureCorpus {
        documents: vec![text_document, title_document],
    };

    for policy in [
        ExpansionPolicy::ExactOnly,
        ExpansionPolicy::Prefix(EnabledFields::Text),
        ExpansionPolicy::Prefix(EnabledFields::Title),
        ExpansionPolicy::Prefix(EnabledFields::TextAndTitle),
        ExpansionPolicy::Fuzzy(EnabledFields::Text),
        ExpansionPolicy::Fuzzy(EnabledFields::Title),
        ExpansionPolicy::Fuzzy(EnabledFields::TextAndTitle),
        ExpansionPolicy::Split(EnabledFields::Text),
        ExpansionPolicy::Split(EnabledFields::Title),
        ExpansionPolicy::Split(EnabledFields::TextAndTitle),
    ] {
        let run = |query: &str| {
            let mut request = request(query, 10);
            request.expansion = policy;
            evaluate_exhaustively(&corpus, &request).unwrap()
        };
        assert_eq!(run("alpha alpha"), run("alpha"), "policy={policy:?}");
        assert_eq!(run("alpha -delta"), run("alpha"), "policy={policy:?}");
    }

    for policy in [
        ExpansionPolicy::ExactOnly,
        ExpansionPolicy::Prefix(EnabledFields::Text),
        ExpansionPolicy::Prefix(EnabledFields::Title),
        ExpansionPolicy::Prefix(EnabledFields::TextAndTitle),
        ExpansionPolicy::Fuzzy(EnabledFields::Text),
        ExpansionPolicy::Fuzzy(EnabledFields::Title),
        ExpansionPolicy::Fuzzy(EnabledFields::TextAndTitle),
    ] {
        let run = |query: &str| {
            let mut request = request(query, 10);
            request.expansion = policy;
            evaluate_exhaustively(&corpus, &request).unwrap()
        };
        let phrase = run("\"alpha beta\"");
        assert_eq!(run("alpha \"alpha beta\""), phrase, "policy={policy:?}");
        assert_eq!(
            run("\"alpha beta\" \"alpha beta\""),
            phrase,
            "phrase factor must be applied once; policy={policy:?}"
        );
    }

    let text_phrase_plan = build_expansion_plan(
        &corpus,
        parse_query("\"alpha beta\"").unwrap(),
        ExpansionPolicy::Prefix(EnabledFields::Text),
    )
    .unwrap();
    let phrase_trace = evaluate_plan(
        &text_phrase_plan,
        &tokenize("fixture"),
        &tokenize("alpha alpha beta gamma"),
    );
    assert_eq!(phrase_trace.score.to_bits(), 0x4170_0000);
}

#[test]
fn repeated_phrase_occurrences_remain_a_positional_predicate() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let mut separated = base.documents[0].clone();
    separated.id = "separated".to_string();
    separated.video_id = "separated".to_string();
    separated.title = "fixture".to_string();
    separated.text = "alpha beta alpha".to_string();
    let corpus = FixtureCorpus {
        documents: vec![separated],
    };
    assert_eq!(
        evaluate_exhaustively(&corpus, &request("\"alpha alpha\"", 10))
            .unwrap()
            .total_videos,
        0
    );
    assert_eq!(
        evaluate_exhaustively(&corpus, &request("alpha alpha", 10))
            .unwrap()
            .total_videos,
        1
    );
}

#[test]
fn split_admits_every_qualifying_boundary_and_breaks_ties_canonically() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let mut document = base.documents[0].clone();
    document.id = "many-splits".to_string();
    document.video_id = "many-splits".to_string();
    document.title = "fixture".to_string();
    let compact = "abcdefghijklmn";
    let mut components = Vec::new();
    for boundary in 3..=11 {
        components.push(compact[..boundary].to_string());
        components.push(compact[boundary..].to_string());
    }
    document.text = components.join(" ");
    let corpus = FixtureCorpus {
        documents: vec![document],
    };

    let plan = build_expansion_plan(
        &corpus,
        parse_query(compact).unwrap(),
        ExpansionPolicy::Split(EnabledFields::Text),
    )
    .unwrap();
    let ExpansionPlan::Hypotheses { slots, .. } = &plan else {
        panic!("split must produce slot hypotheses");
    };
    let split_count = slots[compact]
        .iter()
        .filter(|hypothesis| hypothesis.kind == HypothesisKind::SplitExact)
        .count();
    assert_eq!(
        split_count, 9,
        "legacy's eight-alternative cap must not leak"
    );

    let trace = evaluate_plan(&plan, &[], &tokenize(&components.join(" ")));
    assert_eq!(trace.score.to_bits(), 0x4180_0000);
    assert_eq!(
        trace.selected_hypothesis,
        Some(HypothesisSelection {
            kind: HypothesisKind::SplitExact,
            canonical_bytes: b"abc\0defghijklmn".to_vec(),
        })
    );
}

#[test]
fn split_boundaries_count_unicode_scalars_and_score_left_then_right_then_bonus() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let mut document = base.documents[0].clone();
    document.id = "unicode-split".to_string();
    document.video_id = "unicode-split".to_string();
    document.title = "fixture".to_string();
    document.text = "ééé ééé abcdef".to_string();
    let corpus = FixtureCorpus {
        documents: vec![document],
    };
    let compact = "éééabcdef";
    let plan = build_expansion_plan(
        &corpus,
        parse_query(compact).unwrap(),
        ExpansionPolicy::Split(EnabledFields::Text),
    )
    .unwrap();
    let ExpansionPlan::Hypotheses { slots, .. } = &plan else {
        panic!("split must produce slot hypotheses");
    };
    let splits = slots[compact]
        .iter()
        .filter(|hypothesis| hypothesis.kind == HypothesisKind::SplitExact)
        .collect::<Vec<_>>();
    assert_eq!(splits.len(), 1);
    assert_eq!(splits[0].canonical_bytes, "ééé\0abcdef".as_bytes());

    let trace = evaluate_plan(&plan, &[], &tokenize("ééé ééé abcdef"));
    assert_eq!(trace.score.to_bits(), 0x41a0_0000);

    let mut numeric_document = corpus.documents[0].clone();
    numeric_document.id = "numeric-components".to_string();
    numeric_document.video_id = "numeric-components".to_string();
    numeric_document.text = "123 456".to_string();
    let numeric_corpus = FixtureCorpus {
        documents: vec![numeric_document],
    };
    let mut numeric = request("123456", 10);
    numeric.expansion = ExpansionPolicy::Split(EnabledFields::Text);
    assert_eq!(
        evaluate_exhaustively(&numeric_corpus, &numeric)
            .unwrap()
            .total_videos,
        0,
        "pure Unicode-decimal terms are exact-only"
    );
    let mut unicode_numeric_document = numeric_corpus.documents[0].clone();
    unicode_numeric_document.id = "unicode-numeric-components".to_string();
    unicode_numeric_document.video_id = "unicode-numeric-components".to_string();
    unicode_numeric_document.text = "١٢٣ ٤٥٦".to_string();
    let unicode_numeric_corpus = FixtureCorpus {
        documents: vec![unicode_numeric_document],
    };
    let mut unicode_numeric = request("١٢٣٤٥٦", 10);
    unicode_numeric.expansion = ExpansionPolicy::Split(EnabledFields::Text);
    assert_eq!(
        evaluate_exhaustively(&unicode_numeric_corpus, &unicode_numeric)
            .unwrap()
            .total_videos,
        0,
        "non-ASCII Unicode-decimal terms are exact-only"
    );
}

#[test]
fn split_requires_one_document_but_allows_components_across_enabled_fields() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let prototype = base.documents[0].clone();
    let mut cross_field = prototype.clone();
    cross_field.id = "cross-field".to_string();
    cross_field.video_id = "cross-field".to_string();
    cross_field.title = "black".to_string();
    cross_field.text = "hole".to_string();
    let mut left_only = prototype.clone();
    left_only.id = "left-only".to_string();
    left_only.video_id = "left-only".to_string();
    left_only.title = "fixture".to_string();
    left_only.text = "black".to_string();
    let mut right_only = prototype;
    right_only.id = "right-only".to_string();
    right_only.video_id = "right-only".to_string();
    right_only.title = "fixture".to_string();
    right_only.text = "hole".to_string();
    let corpus = FixtureCorpus {
        documents: vec![cross_field, left_only, right_only],
    };

    let mut both = request("blackhole", 10);
    both.expansion = ExpansionPolicy::Split(EnabledFields::TextAndTitle);
    let result = evaluate_exhaustively(&corpus, &both).unwrap();
    assert_eq!(result.total_videos, 1);
    assert_eq!(result.hits[0].video_id, "cross-field");
    assert!(result.hits[0].provenance.title);
    assert!(result.hits[0].provenance.transcript);

    for fields in [EnabledFields::Text, EnabledFields::Title] {
        let mut scoped = both.clone();
        scoped.expansion = ExpansionPolicy::Split(fields);
        assert_eq!(
            evaluate_exhaustively(&corpus, &scoped)
                .unwrap()
                .total_videos,
            0
        );
    }
}

#[test]
fn whole_token_wins_an_exact_score_tie_and_negation_stays_exact() {
    let base = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let mut tied = base.documents[0].clone();
    tied.id = "tied".to_string();
    tied.video_id = "tied".to_string();
    tied.title = "fixture".to_string();
    tied.text = "blackhole blackhole blackhole blackhole black hole".to_string();
    let corpus = FixtureCorpus {
        documents: vec![tied],
    };
    let plan = build_expansion_plan(
        &corpus,
        parse_query("blackhole").unwrap(),
        ExpansionPolicy::Split(EnabledFields::Text),
    )
    .unwrap();
    let trace = evaluate_plan(&plan, &[], &tokenize(&corpus.documents[0].text));
    assert_eq!(trace.score.to_bits(), 0x4180_0000);
    assert_eq!(
        trace.selected_hypothesis,
        Some(HypothesisSelection {
            kind: HypothesisKind::ExactWhole,
            canonical_bytes: b"blackhole".to_vec(),
        })
    );

    let mut excluded = request("blackhole -black", 10);
    excluded.expansion = ExpansionPolicy::Split(EnabledFields::Text);
    assert_eq!(
        evaluate_exhaustively(&corpus, &excluded)
            .unwrap()
            .total_videos,
        0
    );

    let mut repeated = request("blackhole blackhole", 10);
    repeated.expansion = ExpansionPolicy::Split(EnabledFields::Text);
    let mut single = request("blackhole", 10);
    single.expansion = ExpansionPolicy::Split(EnabledFields::Text);
    assert_eq!(
        evaluate_exhaustively(&corpus, &repeated).unwrap(),
        evaluate_exhaustively(&corpus, &single).unwrap()
    );

    let mut literal_negative_document = corpus.documents[0].clone();
    literal_negative_document.id = "literal-negative".to_string();
    literal_negative_document.video_id = "literal-negative".to_string();
    literal_negative_document.text = "anchor black hole".to_string();
    let literal_negative_corpus = FixtureCorpus {
        documents: vec![literal_negative_document],
    };
    let mut literal_negative = request("anchor -blackhole", 10);
    literal_negative.expansion = ExpansionPolicy::Split(EnabledFields::Text);
    let literal_negative_result =
        evaluate_exhaustively(&literal_negative_corpus, &literal_negative).unwrap();
    assert_eq!(literal_negative_result.total_videos, 1);
    assert_eq!(literal_negative_result.hits[0].video_id, "literal-negative");
}

#[test]
fn differential_detects_a_deliberately_wrong_candidate() {
    struct Wrong;
    impl CandidateImplementation for Wrong {
        fn search(&self, _request: &OracleRequest) -> Result<OracleResult, OracleError> {
            Ok(OracleResult {
                total_videos: 0,
                total_moments: 0,
                by_creator: BTreeMap::new(),
                by_content_kind: BTreeMap::new(),
                hits: Vec::new(),
            })
        }
    }
    let corpus = FixtureCorpus::from_jsonl(FIXTURE).unwrap();
    let query = request("rank", 10);
    let expected = evaluate_exhaustively(&corpus, &query).unwrap();
    assert_eq!(
        first_mismatch(&expected, &Wrong, &query),
        Some("total_videos")
    );

    struct Broken;
    impl CandidateImplementation for Broken {
        fn search(&self, _request: &OracleRequest) -> Result<OracleResult, OracleError> {
            Err(OracleError::UnsupportedReferenceSemantics("broken"))
        }
    }
    assert_eq!(
        first_mismatch(&expected, &Broken, &query),
        Some("candidate_error")
    );
}
