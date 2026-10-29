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

/// Caption lines a focused semantic moment shows, about as much as one Raw hit
/// with its neighbors.
const FOCUS_LINES: usize = 2;

/// Byte ranges of the words `content_words` would see in `text`.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '\'' || c == '\u{2019}';
    let mut spans = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (start, is_word(c)) {
            (None, true) => start = Some(index),
            (Some(from), false) => {
                spans.push((from, index));
                start = None;
            }
            _ => {}
        }
    }
    spans
}

/// The wanted word `token` spells, allowing a plural or possessive ending. No
/// prefixes: `react` must not mark `Preact` or `reactive`.
fn wanted_word<'a>(token: &str, wanted: &'a BTreeSet<String>) -> Option<&'a String> {
    let lowered = normalize_apostrophes(&token.to_ascii_lowercase());
    let word = lowered.trim_matches('\'');
    [
        Some(word),
        word.strip_suffix("'s"),
        word.strip_suffix('s'),
        word.strip_suffix("es"),
    ]
    .into_iter()
    .flatten()
    .find_map(|candidate| wanted.get(candidate))
}

fn push_escaped(text: &str, output: &mut String) {
    for c in text.chars() {
        match c {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(c),
        }
    }
}

/// `text` escaped as Raw escapes snippets, with wanted words marked; `None`
/// when nothing matched, as Raw omits highlights.
fn highlight(text: &str, wanted: &BTreeSet<String>) -> Option<String> {
    let mut output = String::new();
    let mut last = 0;
    for (start, end) in word_spans(text) {
        if wanted_word(&text[start..end], wanted).is_some() {
            push_escaped(&text[last..start], &mut output);
            output.push_str("<mark>");
            push_escaped(&text[start..end], &mut output);
            output.push_str("</mark>");
            last = end;
        }
    }
    (last > 0).then(|| {
        push_escaped(&text[last..], &mut output);
        output
    })
}

fn matched_words(lines: &[&str], wanted: &BTreeSet<String>) -> usize {
    let mut matched = BTreeSet::new();
    for line in lines {
        for (start, end) in word_spans(line) {
            matched.extend(wanted_word(&line[start..end], wanted));
        }
    }
    matched.len()
}

/// Narrow a semantic passage (about a minute of captions, one Raw chunk per
/// line) to the moment that answers. A Raw hit of the same video inside the
/// passage is used as is: its time and highlight are exact. Otherwise the
/// `FOCUS_LINES` lines matching the most query words are kept, with the lines
/// around them as context, and the start time is interpolated from the
/// position of those lines in the passage. With no match the passage opening
/// is kept, whose start time is exact.
fn focus(
    mut hit: SearchHit,
    wanted: &BTreeSet<String>,
    lexical: &[&SearchHit],
    used: &HashSet<String>,
) -> SearchHit {
    if let Some(exact) = lexical.iter().find(|candidate| {
        candidate.match_kind == MatchKind::Transcript
            && (hit.t_start_ms..hit.t_end_ms).contains(&candidate.t_start_ms)
            && !used.contains(&candidate.id)
    }) {
        return (*exact).clone();
    }
    let text = std::mem::take(&mut hit.text);
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let width = FOCUS_LINES.min(lines.len());
    let best = (0..=lines.len() - width)
        .max_by_key(|&first| {
            (
                matched_words(&lines[first..first + width], wanted),
                Reverse(first),
            )
        })
        .unwrap_or(0);
    let length = |lines: &[&str]| {
        lines
            .iter()
            .map(|line| line.chars().count() as u64)
            .sum::<u64>()
    };
    let at = |chars: u64| {
        let span = u64::from(hit.t_end_ms.saturating_sub(hit.t_start_ms));
        hit.t_start_ms + (span * chars / length(&lines).max(1)) as u32
    };
    let (t_start_ms, t_end_ms) = (
        at(length(&lines[..best])),
        at(length(&lines[..best + width])),
    );
    hit.text = lines[best..best + width].join(" ");
    hit.prev_text = best.checked_sub(1).map(|before| lines[before].to_string());
    hit.next_text = lines.get(best + width).map(|after| after.to_string());
    hit.text_highlight = highlight(&hit.text, wanted);
    (hit.timestamp, hit.youtube_url, hit.embed_url) = {
        let (timestamp, youtube_url, embed_url) = moment_links(&hit.video_id, t_start_ms);
        (Some(timestamp), youtube_url, embed_url)
    };
    (hit.t_start_ms, hit.t_end_ms) = (t_start_ms, t_end_ms);
    hit
}

/// Focus each semantic moment on the words of `query`, in semantic order. A Raw
/// hit serves one moment: passages overlap at their edges.
pub(super) fn focus_moments(
    hits: Vec<SearchHit>,
    query: &str,
    lexical: &[SearchVideoResult],
) -> Vec<SearchHit> {
    let wanted = content_words(&lexical_query(query));
    let mut by_video = HashMap::<&str, Vec<&SearchHit>>::new();
    for video in lexical {
        by_video.insert(&video.video_id, video.hits.iter().collect());
    }
    let mut used = HashSet::new();
    let mut moments = Vec::with_capacity(hits.len());
    for hit in hits {
        let candidates = by_video
            .get(hit.video_id.as_str())
            .map_or(&[][..], Vec::as_slice);
        let moment = focus(hit, &wanted, candidates, &used);
        if used.insert(moment.id.clone()) {
            moments.push(moment);
        }
    }
    moments
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
) -> Result<Vec<SearchHit>, ApiError> {
    let client = state
        .semantic
        .as_ref()
        .ok_or(ApiError::SemanticUnavailable)?;
    client.hits(query, params).await
}

/// Semantic moments listed for one video: the ones its card shows. Semantic
/// ranks every passage of the video, so past its best few the list is the rest
/// of the transcript.
const VIDEO_SEMANTIC_MOMENTS: usize = INITIAL_GROUP_LIMIT as usize;

/// Moments of one video, the request behind "show all moments": semantic's
/// best few, focused, then every Raw match of the rewritten query not already
/// shown. Without semantic, only the Raw matches.
fn video_moments(
    semantic: Result<Vec<SearchHit>, ApiError>,
    query: &str,
    lexical: Vec<SearchHit>,
) -> Result<(Vec<SearchHit>, &'static str), ApiError> {
    let semantic = match semantic {
        Ok(mut hits) => {
            hits.truncate(VIDEO_SEMANTIC_MOMENTS);
            hits
        }
        Err(ApiError::SemanticUnavailable) => {
            tracing::warn!("semantic search unavailable; answering video moments lexically");
            return Ok((lexical, "lexical"));
        }
        Err(error) => return Err(error),
    };
    let lexical_video = video_results_from_hits_with_policy(
        lexical.clone(),
        SearchSort::Relevance,
        query,
        RelevanceOrdering::Backend,
    );
    let mut moments = focus_moments(semantic, query, &lexical_video);
    let shown: HashSet<String> = moments.iter().map(|hit| hit.id.clone()).collect();
    moments.extend(lexical.into_iter().filter(|hit| !shown.contains(&hit.id)));
    Ok((moments, "hybrid"))
}

pub(super) async fn video_response(
    state: &AppState,
    params: &SearchParams,
    query: &str,
    video_id: &str,
    per_page: u8,
    page: u32,
) -> Result<Response, ApiError> {
    let started = Instant::now();
    let lexical_query = lexical_query(query);
    let mut search_query = TextSearchQuery::new(close_unterminated_phrase(&lexical_query))
        .query_by("text,title")
        .highlight_fields("text,title")
        .per_page(VIDEO_SCOPE_PAGE_SIZE)
        .page(1)
        .without_grouping()
        .filter_by(format!("video_id:={video_id}"));
    if let Some(youtuber) = non_empty(params.youtuber.as_deref()) {
        search_query = search_query.youtuber_handle(normalize_handle(youtuber));
    }
    let lexical = async {
        let raw = state.search.search(&search_query).await?;
        collect_hits(
            &state.search,
            &search_query,
            raw,
            VIDEO_SCOPE_PAGE_SIZE,
            VIDEO_SCOPE_MAX_PAGE_FETCHES,
            usize::MAX,
            &mut SearchCollectionTimings::default(),
        )
        .await
    };
    let (lexical, semantic) = tokio::join!(lexical, semantic_candidates(state, params, query));
    let (moments, mode) = video_moments(semantic, query, lexical?)?;
    let videos = video_results_from_hits_with_policy(
        moments,
        SearchSort::Relevance,
        query,
        RelevanceOrdering::Backend,
    );
    let total_moments = videos.iter().map(|video| video.hits.len() as u64).sum();
    let offset = (page - 1).saturating_mul(u32::from(per_page)) as usize;
    let (videos, has_next_page) = page_of_videos(videos, offset, per_page);
    Ok(no_store_json(SearchResponse {
        query,
        total_moments,
        total_videos: videos.len() as u64,
        totals_approximate: false,
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
            let semantic = video_results_from_hits_with_policy(
                focus_moments(semantic, query, &lexical),
                SearchSort::Relevance,
                query,
                RelevanceOrdering::Backend,
            );
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

    fn hit(video_id: &str, id: &str, text: &str, t_start_ms: u32, t_end_ms: u32) -> SearchHit {
        SearchHit {
            id: id.into(),
            video_id: video_id.into(),
            title: video_id.into(),
            text: text.into(),
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
            t_start_ms,
            t_end_ms,
            timestamp: None,
            youtube_url: String::new(),
            embed_url: String::new(),
            group_match_count: None,
        }
    }

    fn video(id: &str, highlights: &[&str]) -> SearchVideoResult {
        let hit = hit(id, &format!("{id}-0"), "", 0, 1);
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

    fn lexical_video(hits: Vec<SearchHit>) -> SearchVideoResult {
        let mut video = SearchVideoResult::from_hit(&hits[0]);
        video.hits = hits;
        video
    }

    fn ten_char_lines() -> SearchHit {
        // Five 10-character caption lines over 50 s: each line is 10 s.
        let text = "intro line\nmore intro\nbun's fast\nthan nodes\nouttro bit";
        hit("v", "passage", text, 100_000, 150_000)
    }

    #[test]
    fn focus_keeps_the_lines_matching_most_query_words_at_their_time() {
        let [moment] = focus_moments(vec![ten_char_lines()], "is bun faster than node?", &[])
            .try_into()
            .unwrap();
        assert_eq!(moment.text, "bun's fast than nodes");
        assert_eq!(moment.prev_text.as_deref(), Some("more intro"));
        assert_eq!(moment.next_text.as_deref(), Some("outtro bit"));
        // `fast` is not `faster`: only plural and possessive endings match.
        assert_eq!(
            moment.text_highlight.as_deref(),
            Some("<mark>bun&#39;s</mark> fast than <mark>nodes</mark>")
        );
        assert_eq!((moment.t_start_ms, moment.t_end_ms), (120_000, 140_000));
        assert_eq!(moment.timestamp.as_deref(), Some("2:00"));
        assert_eq!(
            moment.youtube_url,
            "https://www.youtube.com/watch?v=v&t=120s"
        );
        assert_eq!(
            moment.embed_url,
            "https://www.youtube-nocookie.com/embed/v?start=120&rel=0"
        );
    }

    #[test]
    fn focus_without_a_matching_word_keeps_the_passage_opening_and_its_exact_time() {
        let [moment] = focus_moments(vec![ten_char_lines()], "why is rust popular?", &[])
            .try_into()
            .unwrap();
        assert_eq!(moment.text, "intro line more intro");
        assert_eq!(
            (moment.prev_text, moment.next_text.as_deref()),
            (None, Some("bun's fast"))
        );
        assert_eq!(moment.text_highlight, None);
        assert_eq!(
            (moment.t_start_ms, moment.timestamp.as_deref()),
            (100_000, Some("1:40"))
        );
    }

    #[test]
    fn highlight_escapes_like_raw_and_marks_whole_words_only() {
        let wanted = content_words("react");
        assert_eq!(
            highlight("Preact & <React's> reactive", &wanted).as_deref(),
            Some("Preact &amp; &lt;<mark>React&#39;s</mark>&gt; reactive")
        );
        assert_eq!(highlight("Preact is reactive", &wanted), None);
    }

    #[test]
    fn overlapping_passages_each_take_a_different_exact_raw_hit() {
        let passages = vec![
            hit("v", "p1", "a\nb", 0, 60_000),
            hit("v", "p2", "c\nd", 58_000, 120_000),
            hit("v", "p3", "e\nf", 300_000, 360_000),
        ];
        // Raw order: the edge hit lies in both p1 and p2, the last in neither.
        let lexical = [lexical_video(vec![
            hit("v", "edge", "rust", 58_500, 62_000),
            hit("v", "inside-p2", "rust", 70_000, 75_000),
            hit("v", "outside", "rust", 200_000, 205_000),
        ])];
        let moments = focus_moments(passages, "why rust?", &lexical);
        let ids: Vec<&str> = moments.iter().map(|moment| moment.id.as_str()).collect();
        assert_eq!(ids, ["edge", "inside-p2", "p3"]);
        assert_eq!(moments[1].t_start_ms, 70_000);
    }

    #[test]
    fn video_moments_list_the_cards_semantic_moments_then_the_other_raw_matches() {
        let semantic: Vec<SearchHit> = (0..5)
            .map(|i| {
                hit(
                    "v",
                    &format!("p{i}"),
                    "x\ny",
                    i * 100_000,
                    i * 100_000 + 60_000,
                )
            })
            .collect();
        let lexical = vec![
            hit("v", "in-p1", "rust", 110_000, 115_000),
            hit("v", "in-p4", "rust", 410_000, 415_000),
        ];
        let (moments, mode) = video_moments(Ok(semantic), "why rust?", lexical.clone()).unwrap();
        let ids: Vec<&str> = moments.iter().map(|moment| moment.id.as_str()).collect();
        assert_eq!((ids, mode), (vec!["p0", "in-p1", "p2", "in-p4"], "hybrid"));

        let (moments, mode) = video_moments(
            Err(ApiError::SemanticUnavailable),
            "why rust?",
            lexical.clone(),
        )
        .unwrap();
        assert_eq!((moments.len(), mode), (2, "lexical"));
        assert!(matches!(
            video_moments(Err(ApiError::BadRequest("q".into())), "why rust?", lexical),
            Err(ApiError::BadRequest(_))
        ));
    }
}
