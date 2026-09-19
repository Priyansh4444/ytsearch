//! Independent exhaustive reference for the schema-9 exact-query subset.
//!
//! This module deliberately does not call the production parser, scorer,
//! posting decoder, sorter, or aggregator. It is slow by design: every request
//! scans every fixture document, constructs the complete result, and paginates
//! only after deterministic ordering.

use std::{cmp::Ordering, collections::BTreeMap};

use serde::Deserialize;
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
enum ExpansionPolicy {
    ExactOnly,
    Prefix,
    Fuzzy,
    Split,
    TokenDrop,
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

fn evaluate_exhaustively(
    corpus: &FixtureCorpus,
    request: &OracleRequest,
) -> Result<OracleResult, OracleError> {
    match request.expansion {
        ExpansionPolicy::ExactOnly => {}
        ExpansionPolicy::Prefix => {
            return Err(OracleError::UnsupportedReferenceSemantics("prefix"));
        }
        ExpansionPolicy::Fuzzy => {
            return Err(OracleError::UnsupportedReferenceSemantics("fuzzy"));
        }
        ExpansionPolicy::Split => {
            return Err(OracleError::UnsupportedReferenceSemantics("split"));
        }
        ExpansionPolicy::TokenDrop => {
            return Err(OracleError::UnsupportedReferenceSemantics("token_drop"));
        }
    }
    let query = parse_query(&request.query)?;
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
        let trace = evaluate_query(&query, &title, &text);
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

#[derive(Clone, Copy, Debug, Default)]
struct Trace {
    matched: bool,
    score: f32,
    provenance: Provenance,
}

fn evaluate_query(query: &Query, title: &[String], text: &[String]) -> Trace {
    match query {
        Query::All => Trace {
            matched: true,
            ..Trace::default()
        },
        Query::Term(term) => {
            let title_count = title.iter().filter(|token| *token == term).count() as f32;
            let text_count = text.iter().filter(|token| *token == term).count() as f32;
            Trace {
                matched: title_count > 0.0 || text_count > 0.0,
                score: title_count * 6.0 + text_count * 4.0,
                provenance: Provenance {
                    title: title_count > 0.0,
                    transcript: text_count > 0.0,
                    phrase: false,
                },
            }
        }
        Query::Phrase(terms) => {
            let title_match = contains_phrase(title, terms);
            let text_match = contains_phrase(text, terms);
            Trace {
                matched: title_match || text_match,
                score: (u8::from(title_match) as f32 * 6.0 + u8::from(text_match) as f32 * 4.0)
                    * 1.25,
                provenance: Provenance {
                    title: title_match,
                    transcript: text_match,
                    phrase: title_match || text_match,
                },
            }
        }
        Query::And(children) => {
            let mut combined = Trace {
                matched: true,
                ..Trace::default()
            };
            for child in children {
                let trace = evaluate_query(child, title, text);
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
            let excluded = evaluate_query(child, title, text);
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
    for (expansion, expected) in [
        (ExpansionPolicy::Prefix, "prefix"),
        (ExpansionPolicy::Fuzzy, "fuzzy"),
        (ExpansionPolicy::Split, "split"),
        (ExpansionPolicy::TokenDrop, "token_drop"),
    ] {
        let mut expanded = request("create", 10);
        expanded.expansion = expansion;
        assert_eq!(
            evaluate_exhaustively(&corpus, &expanded),
            Err(OracleError::UnsupportedReferenceSemantics(expected))
        );
    }
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
