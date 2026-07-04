use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    env,
    net::{AddrParseError, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::sync::Mutex;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use youtube_typesense::{
    DEFAULT_COLLECTION, TextSearchQuery, TypesenseSearchClient, TypesenseSearchConfig,
    YoutuberDocument,
};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_PER_PAGE: u8 = 20;
const MAX_PER_PAGE: u8 = 100;
const MERGE_TRANSCRIPT_HIT_WINDOW_MS: u32 = 20_000;
const MAX_SEARCH_PAGE_FETCHES: u32 = 10;
const CACHE_YOUTUBERS: &str = "public, max-age=30, stale-while-revalidate=300";
const CACHE_SEARCH: &str = "public, max-age=30, stale-while-revalidate=300";
const CACHE_NO_STORE: &str = "no-store";
const DEFAULT_SEARCH_RATE_LIMIT_PER_MINUTE: usize = 120;

#[derive(Clone)]
struct AppState {
    search: TypesenseSearchClient,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    search_rate_limit_per_minute: usize,
}

#[derive(Default)]
struct RateLimiter {
    buckets: HashMap<String, VecDeque<Instant>>,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    youtuber: Option<String>,
    per_page: Option<u8>,
    page: Option<u32>,
    sort: Option<SearchSort>,
}

#[derive(Debug, Serialize)]
struct YoutubersResponse {
    channels: Vec<YoutuberDocument>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    total_found: u64,
    out_of: u64,
    page: u32,
    per_page: u8,
    sort: SearchSort,
    has_next_page: bool,
    hits: Vec<SearchHit>,
    videos: Vec<SearchVideoResult>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchSort {
    #[default]
    Relevance,
    PublishedDesc,
    PublishedAsc,
}

#[derive(Clone, Debug, Serialize)]
struct SearchHit {
    id: String,
    video_id: String,
    title: String,
    text: String,
    prev_text: Option<String>,
    next_text: Option<String>,
    text_highlight: Option<String>,
    title_highlight: Option<String>,
    match_kind: MatchKind,
    youtuber_handle: String,
    youtuber_name: String,
    thumbnail: Option<String>,
    published_at: Option<String>,
    t_start_ms: u32,
    t_end_ms: u32,
    timestamp: Option<String>,
    youtube_url: String,
    embed_url: String,
}

#[derive(Clone, Debug, Serialize)]
struct SearchVideoResult {
    video_id: String,
    title: String,
    youtuber_handle: String,
    youtuber_name: String,
    thumbnail: Option<String>,
    published_at: Option<String>,
    match_count: usize,
    youtube_url: String,
    embed_url: String,
    best_timestamp: Option<String>,
    hits: Vec<SearchHit>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MatchKind {
    Transcript,
    Title,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("missing required environment variable {0}")]
    MissingEnv(&'static str),
    #[error("invalid bind address: {0}")]
    InvalidBindAddress(String),
    #[error("query cannot be empty")]
    EmptyQuery,
    #[error("rate limited; retry after {0} seconds")]
    RateLimited(u64),
    #[error("typesense search failed: {0}")]
    Typesense(#[from] youtube_typesense::TypesenseError),
    #[error("search response had an unexpected shape")]
    UnexpectedSearchShape,
    #[error("server failed: {0}")]
    Server(#[from] std::io::Error),
}

#[tokio::main]
async fn main() -> Result<(), ApiError> {
    let _ = dotenvy::dotenv();
    init_tracing();

    let api_url = required_env("TYPESENSE_API_URL")?;
    let search_api_key = required_env("TYPESENSE_SEARCH_API_KEY")?;
    let collection =
        env::var("TYPESENSE_COLLECTION").unwrap_or_else(|_| DEFAULT_COLLECTION.to_string());
    let bind_addr = env::var("YTSEARCH_API_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let bind_addr: SocketAddr = bind_addr
        .parse()
        .map_err(|error: AddrParseError| ApiError::InvalidBindAddress(error.to_string()))?;

    let search_client = TypesenseSearchClient::new(
        TypesenseSearchConfig::new(api_url, search_api_key).with_collection(collection),
    );
    let state = Arc::new(AppState {
        search: search_client,
        rate_limiter: Arc::new(Mutex::new(RateLimiter::default())),
        search_rate_limit_per_minute: env_usize(
            "YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE",
            DEFAULT_SEARCH_RATE_LIMIT_PER_MINUTE,
        ),
    });
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/youtubers", get(youtubers_handler))
        .route("/api/search", get(search_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    tracing::info!(%bind_addr, "starting ytsearch API");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn health() -> Response {
    cache_response(CACHE_NO_STORE, Json(serde_json::json!({ "ok": true })))
}

async fn youtubers_handler(State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    let mut channels = state
        .search
        .list_youtubers()
        .await?
        .into_iter()
        .filter(is_searchable_channel)
        .collect::<Vec<_>>();
    channels.sort_by(|left, right| {
        right
            .indexed_video_count
            .cmp(&left.indexed_video_count)
            .then_with(|| left.handle.cmp(&right.handle))
    });
    Ok(cache_response(
        CACHE_YOUTUBERS,
        Json(YoutubersResponse { channels }),
    ))
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> Result<Response, ApiError> {
    if state.search_rate_limit_per_minute > 0 {
        let client = client_key(&headers);
        state.rate_limiter.lock().await.allow(
            format!("search:{client}"),
            state.search_rate_limit_per_minute,
            Duration::from_secs(60),
        )?;
    }
    let query = params.q.trim();
    if query.is_empty() {
        return Err(ApiError::EmptyQuery);
    }

    let per_page = params
        .per_page
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(1, MAX_PER_PAGE);
    let page = params.page.unwrap_or(1).max(1);
    let sort = params.sort.unwrap_or_default();
    let search_page_size = per_page.saturating_mul(8).min(MAX_PER_PAGE);
    let requested_offset = (page - 1).saturating_mul(u32::from(per_page));
    let typesense_page = 1;
    let local_offset = requested_offset as usize;
    let mut typesense_query = TextSearchQuery::new(query)
        .query_by("text,title")
        .highlight_fields("text,title")
        .per_page(search_page_size)
        .page(typesense_page)
        .without_grouping();
    tracing::info!(
        query,
        per_page,
        page,
        ?sort,
        typesense_page,
        search_page_size,
        youtuber = params.youtuber.as_deref().unwrap_or(""),
        "search request"
    );

    if let Some(youtuber) = params
        .youtuber
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        typesense_query = typesense_query.youtuber_handle(normalize_handle(youtuber));
    }

    let first_raw = state.search.search(&typesense_query).await?;
    let found = first_raw
        .get("found")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let out_of = first_raw
        .get("out_of")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let page_result = collect_visible_videos(
        &state.search,
        &typesense_query,
        first_raw,
        typesense_page,
        local_offset,
        per_page,
        search_page_size,
        sort,
    )
    .await?;
    let videos = page_result.videos;
    let hits = videos
        .iter()
        .flat_map(|video| video.hits.iter().cloned())
        .collect::<Vec<_>>();
    tracing::info!(
        query,
        found,
        returned_videos = videos.len(),
        returned_hits = hits.len(),
        "search response"
    );

    Ok(cache_response(
        CACHE_SEARCH,
        Json(SearchResponse {
            query: query.to_string(),
            total_found: found.max(hits.len() as u64),
            out_of,
            page,
            per_page,
            sort,
            has_next_page: page_result.has_next_page,
            hits,
            videos,
        }),
    ))
}

fn is_searchable_channel(channel: &YoutuberDocument) -> bool {
    channel.indexed_video_count > 0 && channel.indexed_chunk_count > 0
}

impl RateLimiter {
    fn allow(&mut self, key: String, limit: usize, window: Duration) -> Result<(), ApiError> {
        if limit == 0 {
            return Ok(());
        }
        let now = Instant::now();
        let bucket = self.buckets.entry(key).or_default();
        while bucket
            .front()
            .is_some_and(|instant| now.duration_since(*instant) >= window)
        {
            bucket.pop_front();
        }
        if bucket.len() >= limit {
            let oldest = bucket.front().copied().unwrap_or(now);
            let retry_after = window
                .saturating_sub(now.duration_since(oldest))
                .as_secs()
                .max(1);
            return Err(ApiError::RateLimited(retry_after));
        }
        bucket.push_back(now);
        Ok(())
    }
}

struct SearchPageResult {
    videos: Vec<SearchVideoResult>,
    has_next_page: bool,
}

#[allow(clippy::too_many_arguments)]
async fn collect_visible_videos(
    search: &TypesenseSearchClient,
    base_query: &TextSearchQuery,
    first_raw: Value,
    first_typesense_page: u32,
    local_offset: usize,
    per_page: u8,
    search_page_size: u8,
    sort: SearchSort,
) -> Result<SearchPageResult, ApiError> {
    let required_videos = local_offset
        .saturating_add(usize::from(per_page))
        .saturating_add(1);
    let mut combined = Vec::new();
    let mut current_page = first_typesense_page;
    let mut raw = first_raw;

    for fetch_index in 0..MAX_SEARCH_PAGE_FETCHES {
        let page_hits = parse_hits(&raw)?;
        let is_empty = page_hits.is_empty();
        combined.extend(page_hits);
        combined = merge_nearby_transcript_hits(dedupe_title_only_hits(combined));
        let videos = video_results_from_hits(&combined, sort);

        if videos.len() >= required_videos || is_empty {
            return Ok(page_result_from_videos(videos, local_offset, per_page));
        }

        let fetched_raw_hits = raw
            .get("hits")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if fetched_raw_hits < usize::from(search_page_size) {
            break;
        }

        if fetch_index + 1 == MAX_SEARCH_PAGE_FETCHES {
            break;
        }

        current_page = current_page.saturating_add(1);
        raw = search
            .search(&base_query.clone().page(current_page))
            .await?;
    }

    Ok(page_result_from_videos(
        video_results_from_hits(&combined, sort),
        local_offset,
        per_page,
    ))
}

fn page_result_from_videos(
    videos: Vec<SearchVideoResult>,
    local_offset: usize,
    per_page: u8,
) -> SearchPageResult {
    let per_page = usize::from(per_page);
    let has_next_page = videos.len() > local_offset.saturating_add(per_page);
    SearchPageResult {
        videos: videos
            .into_iter()
            .skip(local_offset)
            .take(per_page)
            .collect(),
        has_next_page,
    }
}

fn video_results_from_hits(hits: &[SearchHit], sort: SearchSort) -> Vec<SearchVideoResult> {
    let mut order = Vec::new();
    let mut by_video: HashMap<String, SearchVideoResult> = HashMap::new();

    for hit in hits {
        let entry = by_video.entry(hit.video_id.clone()).or_insert_with(|| {
            order.push(hit.video_id.clone());
            SearchVideoResult::from_hit(hit)
        });
        entry.hits.push(hit.clone());
    }

    let mut videos = order
        .into_iter()
        .filter_map(|video_id| by_video.remove(&video_id))
        .map(|mut video| {
            video.hits.sort_by_key(|hit| hit.t_start_ms);
            video.match_count = video.hits.len();
            if let Some(best_hit) = video
                .hits
                .iter()
                .find(|hit| hit.match_kind == MatchKind::Transcript)
                .or_else(|| video.hits.first())
            {
                video.youtube_url = best_hit.youtube_url.clone();
                video.embed_url = best_hit.embed_url.clone();
                video.best_timestamp = best_hit.timestamp.clone();
            }
            video
        })
        .collect::<Vec<_>>();

    match sort {
        SearchSort::Relevance => {}
        SearchSort::PublishedDesc => videos.sort_by(|left, right| {
            right
                .published_at
                .cmp(&left.published_at)
                .then_with(|| left.title.cmp(&right.title))
        }),
        SearchSort::PublishedAsc => videos.sort_by(|left, right| {
            left.published_at
                .cmp(&right.published_at)
                .then_with(|| left.title.cmp(&right.title))
        }),
    }

    videos
}

impl SearchVideoResult {
    fn from_hit(hit: &SearchHit) -> Self {
        Self {
            video_id: hit.video_id.clone(),
            title: hit.title.clone(),
            youtuber_handle: hit.youtuber_handle.clone(),
            youtuber_name: hit.youtuber_name.clone(),
            thumbnail: hit.thumbnail.clone(),
            published_at: hit.published_at.clone(),
            match_count: 0,
            youtube_url: hit.youtube_url.clone(),
            embed_url: hit.embed_url.clone(),
            best_timestamp: hit.timestamp.clone(),
            hits: Vec::new(),
        }
    }
}

fn parse_hits(raw: &Value) -> Result<Vec<SearchHit>, ApiError> {
    let Some(hits) = raw.get("hits").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut parsed = Vec::with_capacity(hits.len());
    for hit in hits {
        let document = hit.get("document").ok_or(ApiError::UnexpectedSearchShape)?;
        parsed.push(SearchHit::from_typesense_hit(document, hit)?);
    }

    Ok(parsed)
}

fn merge_nearby_transcript_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut merged: Vec<SearchHit> = Vec::with_capacity(hits.len());

    for hit in hits {
        if hit.match_kind != MatchKind::Transcript {
            merged.push(hit);
            continue;
        }

        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| is_same_transcript_moment(existing, &hit))
        {
            if hit.t_start_ms < existing.t_start_ms {
                *existing = hit;
            }
            continue;
        }

        merged.push(hit);
    }

    merged
}

fn is_same_transcript_moment(left: &SearchHit, right: &SearchHit) -> bool {
    left.match_kind == MatchKind::Transcript
        && right.match_kind == MatchKind::Transcript
        && left.video_id == right.video_id
        && left.t_start_ms.abs_diff(right.t_start_ms) <= MERGE_TRANSCRIPT_HIT_WINDOW_MS
}

fn dedupe_title_only_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let transcript_videos = hits
        .iter()
        .filter(|hit| hit.match_kind == MatchKind::Transcript)
        .map(|hit| hit.video_id.clone())
        .collect::<std::collections::HashSet<_>>();
    let mut seen_title_videos = std::collections::HashSet::new();
    let mut deduped = Vec::with_capacity(hits.len());

    for hit in hits {
        match hit.match_kind {
            MatchKind::Transcript => deduped.push(hit),
            MatchKind::Title => {
                if transcript_videos.contains(&hit.video_id) {
                    continue;
                }
                if seen_title_videos.insert(hit.video_id.clone()) {
                    deduped.push(hit);
                }
            }
        }
    }

    deduped
}

impl SearchHit {
    fn from_typesense_hit(document: &Value, hit: &Value) -> Result<Self, ApiError> {
        let id = string_field(document, "id")?;
        let video_id = string_field(document, "video_id")?;
        let t_start_ms = u32_field(document, "t_start_ms")?;
        let t_end_ms = u32_field(document, "t_end_ms")?;
        let text_highlight = highlight_for_field(hit, "text");
        let title_highlight = highlight_for_field(hit, "title");
        let match_kind = if title_highlight.is_some() && text_highlight.is_none() {
            MatchKind::Title
        } else {
            MatchKind::Transcript
        };
        let timestamp =
            matches!(match_kind, MatchKind::Transcript).then(|| format_timestamp(t_start_ms));
        let youtube_url = match match_kind {
            MatchKind::Transcript => format!(
                "https://www.youtube.com/watch?v={video_id}&t={}s",
                t_start_ms / 1000
            ),
            MatchKind::Title => format!("https://www.youtube.com/watch?v={video_id}"),
        };
        let embed_url = match match_kind {
            MatchKind::Transcript => format!(
                "https://www.youtube-nocookie.com/embed/{video_id}?start={}&rel=0",
                t_start_ms / 1000
            ),
            MatchKind::Title => format!("https://www.youtube-nocookie.com/embed/{video_id}?rel=0"),
        };

        Ok(Self {
            id,
            video_id,
            title: string_field(document, "title")?,
            text: string_field(document, "text")?,
            prev_text: optional_string_field(document, "prev_text"),
            next_text: optional_string_field(document, "next_text"),
            text_highlight,
            title_highlight,
            match_kind,
            youtuber_handle: string_field(document, "youtuber_handle")?,
            youtuber_name: string_field(document, "youtuber_name")?,
            thumbnail: optional_string_field(document, "thumbnail"),
            published_at: optional_string_field(document, "published_at"),
            t_start_ms,
            t_end_ms,
            timestamp,
            youtube_url,
            embed_url,
        })
    }
}

fn highlight_for_field(hit: &Value, field: &str) -> Option<String> {
    hit.get("highlights")
        .and_then(Value::as_array)
        .and_then(|highlights| {
            highlights.iter().find_map(|highlight| {
                let matches_field = highlight.get("field").and_then(Value::as_str) == Some(field);
                matches_field
                    .then(|| highlight.get("snippet").and_then(Value::as_str))
                    .flatten()
            })
        })
        .or_else(|| {
            hit.get("highlight")
                .and_then(|highlight| highlight.get(field))
                .and_then(|field_highlight| field_highlight.get("snippet"))
                .and_then(Value::as_str)
        })
        .map(ToOwned::to_owned)
}

fn string_field(document: &Value, field: &'static str) -> Result<String, ApiError> {
    document
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or(ApiError::UnexpectedSearchShape)
}

fn optional_string_field(document: &Value, field: &'static str) -> Option<String> {
    document.get(field)?.as_str().map(ToOwned::to_owned)
}

fn u32_field(document: &Value, field: &'static str) -> Result<u32, ApiError> {
    document
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(ApiError::UnexpectedSearchShape)
}

fn format_timestamp(milliseconds: u32) -> String {
    let total_seconds = milliseconds / 1000;
    let seconds = total_seconds % 60;
    let minutes = total_seconds / 60 % 60;
    let hours = total_seconds / 3600;

    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

fn normalize_handle(handle: &str) -> String {
    let trimmed = handle.trim();
    if trimmed.starts_with('@') {
        trimmed.to_string()
    } else {
        format!("@{trimmed}")
    }
}

fn client_key(headers: &HeaderMap) -> String {
    ["cf-connecting-ip", "fly-client-ip", "x-forwarded-for"]
        .iter()
        .filter_map(|name| headers.get(*name))
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            value
                .split(',')
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("unknown")
        .to_string()
}

fn cache_response<T: IntoResponse>(cache_control: &'static str, response: T) -> Response {
    let mut response = response.into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    response
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn required_env(name: &'static str) -> Result<String, ApiError> {
    env::var(name).map_err(|_| ApiError::MissingEnv(name))
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "youtube_api=info,tower_http=info".into());
    let subscriber = tracing_subscriber::fmt().with_env_filter(env_filter);

    if env::var("LOG_FORMAT").is_ok_and(|value| value.eq_ignore_ascii_case("json")) {
        subscriber.json().init();
    } else {
        subscriber.init();
    }
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to install Ctrl+C handler");
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::EmptyQuery => StatusCode::BAD_REQUEST,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::MissingEnv(_) | Self::InvalidBindAddress(_) | Self::Server(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Typesense(_) | Self::UnexpectedSearchShape => StatusCode::BAD_GATEWAY,
        };

        let retry_after = match &self {
            Self::RateLimited(seconds) => Some(*seconds),
            _ => None,
        };
        let body = Json(serde_json::json!({ "error": self.to_string() }));
        let mut response = (status, body).into_response();
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(CACHE_NO_STORE),
        );
        if let Some(seconds) = retry_after {
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}
