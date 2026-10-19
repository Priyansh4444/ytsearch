//! `mode=hybrid`: answer a natural-language question from both engines.
//!
//! A question is rewritten into a keyword query for Raw, Raw and the semantic
//! service are asked concurrently, and the two video rankings are fused with
//! weighted reciprocal rank fusion. Fusion leans lexical (3:1) when Raw's top
//! video matched at least 75% of the rewritten query's content words, and
//! semantic (1:3) otherwise. This is `gated_rw` in
//! docs/experiments/semantic/ranking-eval-2026-10-17.md; scripts/semantic-eval.py
//! is the reference implementation the tests below compare against.
use super::*;
use std::collections::BTreeSet;

const RRF_K: f64 = 60.0;
const GATE_COVERAGE: f64 = 0.75;
/// Videos taken from each engine before fusion, matching the evaluation.
const CANDIDATE_VIDEOS: u8 = 100;
/// Lexical hits per video the gate reads, matching the evaluation.
const GATE_HITS: usize = 3;

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "do", "does", "did", "for",
    "from", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it", "its", "it's", "me",
    "my", "no", "not", "of", "on", "or", "our", "so", "than", "that", "the", "their", "them",
    "then", "there", "these", "they", "this", "to", "too", "up", "us", "was", "we", "were", "what",
    "when", "where", "which", "who", "why", "will", "with", "would", "you", "your",
];
const QUESTION_WORDS: &[&str] = &[
    "what", "whats", "what's", "why", "how", "hows", "how's", "when", "where", "who", "whose",
    "whom", "which",
];
const AUXILIARIES: &[&str] = &[
    "do",
    "does",
    "did",
    "is",
    "are",
    "was",
    "were",
    "am",
    "be",
    "been",
    "being",
    "can",
    "could",
    "should",
    "would",
    "will",
    "shall",
    "may",
    "might",
    "must",
    "has",
    "have",
    "had",
    "don't",
    "doesn't",
    "didn't",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "can't",
    "couldn't",
    "won't",
    "wouldn't",
    "shouldn't",
];
/// Words a person uses to ask about a video that its transcript will not contain.
const ASKING_WORDS: &[&str] = &[
    "theo",
    "theos",
    "he",
    "his",
    "him",
    "video",
    "videos",
    "talk",
    "talks",
    "talked",
    "talking",
    "say",
    "says",
    "said",
    "saying",
    "think",
    "thinks",
    "thought",
    "opinion",
    "take",
    "explain",
    "explains",
    "explained",
    "mean",
    "means",
    "meant",
    "tell",
    "tells",
    "told",
    "about",
    "really",
    "actually",
    "please",
    "someone",
    "something",
];
const STRIPPED: &[char] = &['?', '!', '.', ',', ';', ':', '(', ')', '[', ']'];

fn listed(lists: &[&[&str]], word: &str) -> bool {
    lists.iter().any(|list| list.contains(&word))
}

fn normalize_apostrophes(text: &str) -> String {
    text.replace('\u{2019}', "'")
}

pub(super) fn is_question(query: &str) -> bool {
    let lowered = normalize_apostrophes(&query.to_lowercase());
    lowered
        .split_whitespace()
        .next()
        .is_some_and(|first| listed(&[QUESTION_WORDS, AUXILIARIES], first))
        || query.trim_end().ends_with('?')
}

/// Whitespace-separated tokens, except that `"..."` with at least one character
/// inside is one token even if it contains spaces.
fn tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let quoted = rest
            .strip_prefix('"')
            .and_then(|inner| inner.find('"').filter(|&close| close > 0))
            .map(|close| close + 2);
        let end = quoted.unwrap_or_else(|| rest.find(char::is_whitespace).unwrap_or(rest.len()));
        tokens.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    tokens
}

/// The keyword query Raw should see. Non-questions are returned unchanged, so
/// exact phrases and keyword searches keep their lexical behavior.
pub(super) fn lexical_query(query: &str) -> String {
    if !is_question(query) {
        return query.to_string();
    }
    let normalized = normalize_apostrophes(query);
    let mut kept = Vec::new();
    for token in tokens(&normalized) {
        if token.starts_with('"') {
            kept.push(token.to_string());
            continue;
        }
        let mut word = token.trim_matches(STRIPPED);
        if word.to_lowercase().ends_with("'s") {
            word = &word[..word.len() - 2];
        }
        let lowered = word.to_lowercase();
        if !word.is_empty()
            && !listed(
                &[STOPWORDS, QUESTION_WORDS, AUXILIARIES, ASKING_WORDS],
                &lowered,
            )
        {
            kept.push(word.to_string());
        }
    }
    if kept.is_empty() {
        query.to_string()
    } else {
        kept.join(" ")
    }
}

fn content_words(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '\''))
        .filter(|word| !word.is_empty() && !STOPWORDS.contains(word))
        .map(str::to_string)
        .collect()
}

fn marked_words(highlight: &str) -> Vec<&str> {
    highlight
        .split("<mark>")
        .skip(1)
        .filter_map(|after| after.split_once("</mark>").map(|(marked, _)| marked))
        .filter(|marked| !marked.is_empty() && !marked.contains('<'))
        .collect()
}

/// Share of the query's content words that the top lexical video matched,
/// read from Raw's highlights (which include fuzzy matches).
fn lexical_coverage(lexical_query: &str, lexical: &[SearchVideoResult]) -> f64 {
    let wanted = content_words(lexical_query);
    let Some(top) = lexical.first() else {
        return 0.0;
    };
    if wanted.is_empty() {
        return 0.0;
    }
    let mut marked = BTreeSet::new();
    for hit in top.hits.iter().take(GATE_HITS) {
        let words = marked_words(hit.text_highlight.as_deref().unwrap_or_default()).join(" ");
        marked.extend(content_words(&words));
    }
    (marked.len() as f64 / wanted.len() as f64).min(1.0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lead {
    Lexical,
    Semantic,
}
impl Lead {
    fn of(lexical_query: &str, lexical: &[SearchVideoResult]) -> Self {
        if lexical_coverage(lexical_query, lexical) >= GATE_COVERAGE {
            Self::Lexical
        } else {
            Self::Semantic
        }
    }
    fn weights(self) -> (f64, f64) {
        match self {
            Self::Lexical => (3.0, 1.0),
            Self::Semantic => (1.0, 3.0),
        }
    }
}

/// Weighted reciprocal rank fusion of two video rankings. Ties keep first-seen
/// order (lexical, then semantic). A video found by both keeps semantic's hits.
fn fuse(
    lexical: Vec<SearchVideoResult>,
    semantic: Vec<SearchVideoResult>,
    lead: Lead,
) -> Vec<SearchVideoResult> {
    let (lexical_weight, semantic_weight) = lead.weights();
    let mut order = Vec::<(String, f64)>::new();
    let mut position = HashMap::<String, usize>::new();
    for (ranked, weight) in [(&lexical, lexical_weight), (&semantic, semantic_weight)] {
        for (rank, video) in ranked.iter().enumerate() {
            let index = *position.entry(video.video_id.clone()).or_insert_with(|| {
                order.push((video.video_id.clone(), 0.0));
                order.len() - 1
            });
            order[index].1 += weight / (RRF_K + (rank + 1) as f64);
        }
    }
    order.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut videos: HashMap<String, SearchVideoResult> = lexical
        .into_iter()
        .map(|video| (video.video_id.clone(), video))
        .collect();
    videos.extend(
        semantic
            .into_iter()
            .map(|video| (video.video_id.clone(), video)),
    );
    order
        .into_iter()
        .filter_map(|(video_id, _)| videos.remove(&video_id))
        .collect()
}

async fn lexical_candidates(
    state: &AppState,
    params: &SearchParams,
    lexical_query: &str,
) -> Result<Vec<SearchVideoResult>, ApiError> {
    let mut query = TextSearchQuery::new(close_unterminated_phrase(lexical_query))
        .query_by("text,title")
        .highlight_fields("text,title")
        .window(0, CANDIDATE_VIDEOS + 1)
        .group_limit(INITIAL_GROUP_LIMIT);
    if let Some(youtuber) = non_empty(params.youtuber.as_deref()) {
        query = query.youtuber_handle(normalize_handle(youtuber));
    }
    let mut raw = state.search.search(&query).await?;
    truncate_grouped_lookahead(&mut raw, CANDIDATE_VIDEOS);
    let mut timings = SearchCollectionTimings::default();
    let hits = collect_hits(
        &state.search,
        &query,
        raw,
        CANDIDATE_VIDEOS + 1,
        1,
        usize::from(CANDIDATE_VIDEOS) + 1,
        &mut timings,
    )
    .await?;
    let mut videos = video_results_from_hits_with_policy(
        hits,
        SearchSort::Relevance,
        lexical_query,
        state.relevance_ordering,
    );
    videos.truncate(usize::from(CANDIDATE_VIDEOS));
    Ok(videos)
}

async fn semantic_candidates(
    state: &AppState,
    params: &SearchParams,
    query: &str,
) -> Result<Vec<SearchVideoResult>, ApiError> {
    let client = state
        .semantic
        .as_ref()
        .ok_or(ApiError::SemanticUnavailable)?;
    let hits = client.hits(query, params).await?;
    Ok(video_results_from_hits_with_policy(
        hits,
        SearchSort::Relevance,
        query,
        RelevanceOrdering::Backend,
    ))
}

/// Fused answer, or the rewritten lexical answer when semantic is unavailable.
pub(super) async fn response(
    state: &AppState,
    params: &SearchParams,
    query: &str,
    per_page: u8,
    page: u32,
) -> Result<Response, ApiError> {
    let started = Instant::now();
    let lexical_query = lexical_query(query);
    let (lexical, semantic) = tokio::join!(
        lexical_candidates(state, params, &lexical_query),
        semantic_candidates(state, params, query)
    );
    let lexical = lexical?;
    let (videos, mode, lead) = match semantic {
        Ok(semantic) => {
            let lead = Lead::of(&lexical_query, &lexical);
            (fuse(lexical, semantic, lead), "hybrid", Some(lead))
        }
        Err(ApiError::SemanticUnavailable) => {
            tracing::warn!("semantic search unavailable; answering hybrid lexically");
            (lexical, "lexical", None)
        }
        Err(error) => return Err(error),
    };
    tracing::info!(query, lexical_query, ?lead, "hybrid search");
    let total_videos = videos.len() as u64;
    let total_moments = videos.iter().map(|video| video.hits.len() as u64).sum();
    let offset = (page - 1).saturating_mul(u32::from(per_page)) as usize;
    let (videos, has_next_page) = page_of_videos(videos, offset, per_page);
    Ok(no_store_json(SearchResponse {
        query,
        total_moments,
        total_videos,
        totals_approximate: true,
        total_found: total_moments,
        out_of: total_moments,
        time_to_first_result_ms: started.elapsed().as_millis() as u64,
        page,
        per_page,
        sort: SearchSort::Relevance,
        mode,
        has_next_page,
        hits: [],
        videos: &videos,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_agrees_with_the_evaluated_reference_implementation() {
        let cases = include_str!("hybrid_rewrite_cases.tsv");
        let mut checked = 0;
        for line in cases.lines().filter(|line| !line.starts_with('#')) {
            let (query, expected) = line.split_once('\t').expect("query<TAB>rewrite");
            assert_eq!(lexical_query(query), expected, "query: {query:?}");
            checked += 1;
        }
        assert!(checked > 150, "fixture lost its cases");
    }

    #[test]
    fn rewrite_leaves_non_questions_and_quoted_phrases_alone() {
        for query in [
            "they made their own language",
            "fable on high",
            "I",
            "react server components",
        ] {
            assert_eq!(lexical_query(query), query);
        }
        let rewritten = lexical_query(r#"why did he say "it's never just that simple"?"#);
        assert!(
            rewritten.contains(r#""it's never just that simple""#),
            "{rewritten}"
        );
        // A question made only of filler stays answerable rather than becoming empty.
        assert_eq!(lexical_query("what do you think?"), "what do you think?");
    }

    fn video(id: &str, highlights: &[&str]) -> SearchVideoResult {
        let hit: SearchHit = SearchHit {
            id: format!("{id}-0"),
            video_id: id.into(),
            title: id.into(),
            text: String::new(),
            prev_text: None,
            next_text: None,
            text_highlight: None,
            title_highlight: None,
            match_kind: MatchKind::Transcript,
            youtuber_handle: "@t".into(),
            youtuber_name: "T".into(),
            thumbnail: None,
            published_at: None,
            view_count: 0,
            like_count: 0,
            comment_count: 0,
            content_kind: ContentKind::Video,
            t_start_ms: 0,
            t_end_ms: 1,
            timestamp: None,
            youtube_url: String::new(),
            embed_url: String::new(),
            group_match_count: None,
        };
        let mut result = SearchVideoResult::from_hit(&hit);
        result.hits = highlights
            .iter()
            .map(|highlight| SearchHit {
                text_highlight: Some((*highlight).into()),
                ..hit.clone()
            })
            .collect();
        result
    }

    fn ids(videos: &[SearchVideoResult]) -> Vec<&str> {
        videos.iter().map(|video| video.video_id.as_str()).collect()
    }

    #[test]
    fn gate_counts_distinct_marked_content_words_of_the_top_video() {
        let full = [video(
            "a",
            &[
                "<mark>code</mark> <mark>training</mark> <mark>data</mark>",
                "by <mark>themselves</mark>",
            ],
        )];
        assert_eq!(
            lexical_coverage("code training data by themselves", &full),
            1.0
        );
        assert_eq!(
            Lead::of("code training data by themselves", &full),
            Lead::Lexical
        );
        // Stopwords marked by Raw do not count; two of three content words is below the gate.
        let partial = [video(
            "a",
            &["<mark>GitHub</mark> <mark>actions</mark> <mark>are</mark> <mark>why</mark>"],
        )];
        assert!((lexical_coverage("GitHub Actions slow", &partial) - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(Lead::of("GitHub Actions slow", &partial), Lead::Semantic);
        // A fuzzy match counts as a matched word even though its spelling differs.
        let fuzzy = [video(
            "a",
            &["<mark>utilizing</mark> your <mark>unused</mark> <mark>bandwidth</mark>"],
        )];
        assert_eq!(Lead::of("utilising unused bandwith", &fuzzy), Lead::Lexical);
        // Only the first three hits are read, as in the evaluation.
        let late = [video("a", &["x", "y", "z", "<mark>code</mark>"])];
        assert_eq!(lexical_coverage("code", &late), 0.0);
        assert_eq!(lexical_coverage("code", &[]), 0.0);
    }

    #[test]
    fn fusion_follows_the_lead_and_prefers_semantic_moments() {
        let lexical = || {
            vec![
                video("l1", &["lex"]),
                video("both", &["lex"]),
                video("l3", &["lex"]),
            ]
        };
        let semantic = || vec![video("s1", &["sem"]), video("both", &["sem"])];
        // Lexical-led (3:1): both = 3/62 + 1/62 beats l1 = 3/61; s1 = 1/61 comes last.
        let led = fuse(lexical(), semantic(), Lead::Lexical);
        assert_eq!(ids(&led), ["both", "l1", "l3", "s1"]);
        // Semantic-led: semantic's first result wins.
        let led = fuse(lexical(), semantic(), Lead::Semantic);
        assert_eq!(ids(&led), ["both", "s1", "l1", "l3"]);
        let both = led.iter().find(|video| video.video_id == "both").unwrap();
        assert_eq!(both.hits[0].text_highlight.as_deref(), Some("sem"));
        // Every candidate appears exactly once.
        assert_eq!(led.len(), 4);
    }

    #[test]
    fn fusion_agrees_with_the_evaluated_reference_on_recorded_rankings() {
        // Each line: lead, lexical ids, semantic ids, Python's fused top 10 (ids are
        // comma-separated; the inputs are each engine's recorded top 30).
        let cases = include_str!("hybrid_fusion_cases.tsv");
        let mut checked = 0;
        for line in cases.lines().filter(|line| !line.starts_with('#')) {
            let fields: Vec<&str> = line.split('\t').collect();
            let lead = match fields[0] {
                "lexical" => Lead::Lexical,
                "semantic" => Lead::Semantic,
                other => panic!("unknown lead {other}"),
            };
            let list = |field: &str| -> Vec<SearchVideoResult> {
                field
                    .split(',')
                    .filter(|id| !id.is_empty())
                    .map(|id| video(id, &[]))
                    .collect()
            };
            let fused = fuse(list(fields[1]), list(fields[2]), lead);
            let expected: Vec<&str> = fields[3].split(',').filter(|id| !id.is_empty()).collect();
            assert_eq!(
                ids(&fused)[..expected.len()],
                expected[..],
                "case {checked}"
            );
            checked += 1;
        }
        assert!(checked >= 60, "fixture lost its cases");
    }
}
