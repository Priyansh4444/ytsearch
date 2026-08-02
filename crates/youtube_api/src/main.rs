use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::{HashMap, VecDeque},
    env,
    net::{AddrParseError, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::Mutex;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use youtube_search::{
    DEFAULT_COLLECTION, SearchBackendError, TextSearchQuery, TranscriptSearchBackend,
    YoutuberDocument,
};
use youtube_typesense::{TypesenseSearchClient, TypesenseSearchConfig};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_PER_PAGE: u8 = 20;
const MAX_PER_PAGE: u8 = 100;
const MERGE_TRANSCRIPT_HIT_WINDOW_MS: u32 = 20_000;
const MAX_SEARCH_PAGE_FETCHES: u32 = 10;
const CACHE_YOUTUBERS: &str = "public, max-age=30, stale-while-revalidate=300";
const CACHE_SEARCH: &str = "public, max-age=30, stale-while-revalidate=300";
const CACHE_CHANNEL_IMAGE: &str = "public, max-age=86400, stale-while-revalidate=604800";
const CACHE_NO_STORE: &str = "no-store";
const DEFAULT_SEARCH_RATE_LIMIT_PER_MINUTE: usize = 120;

#[derive(Clone)]
struct AppState {
    search: Arc<dyn TranscriptSearchBackend>,
    search_backend: String,
    engagement_sorts_enabled: bool,
    preserve_backend_relevance_order: bool,
    axiom: Option<AxiomLogger>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    search_rate_limit_per_minute: usize,
    index_queue: IndexQueue,
    image_http: reqwest::Client,
    semantic_search_enabled: bool,
    semantic_embedding_field: String,
    semantic_lexical_api_url: Option<String>,
}

#[derive(Clone, Debug)]
struct IndexQueue {
    path: PathBuf,
    token: Option<String>,
}

#[derive(Clone, Debug)]
struct AxiomLogger {
    client: reqwest::Client,
    ingest_url: reqwest::Url,
}

#[derive(Default)]
struct RateLimiter {
    buckets: HashMap<String, VecDeque<Instant>>,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    youtuber: Option<String>,
    video_id: Option<String>,
    per_page: Option<u8>,
    page: Option<u32>,
    sort: Option<SearchSort>,
    mode: Option<SearchMode>,
}

#[derive(Debug, Serialize)]
struct YoutubersResponse {
    channels: Vec<YoutuberDocument>,
}

#[derive(Debug, Deserialize)]
struct IndexRequestInput {
    handle: String,
    note: Option<String>,
    backends: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct IndexQueueItem {
    id: String,
    handle: String,
    note: Option<String>,
    requested_at: u64,
    source_backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target_backend: Option<String>,
    status: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct IndexActivity {
    handle: String,
    started_at: u64,
    target_backend: String,
}

#[derive(Debug, Deserialize)]
struct IndexActivityInput {
    handle: Option<String>,
    target_backend: Option<String>,
}

#[derive(Debug, Serialize)]
struct IndexQueueResponse {
    request: IndexQueueItem,
    requests: Vec<IndexQueueItem>,
}

#[derive(Debug, Deserialize)]
struct IndexQueueParams {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct IndexStatusParams {
    handle: Option<String>,
}

#[derive(Debug, Serialize)]
struct IndexQueueListResponse {
    requests: Vec<IndexQueueItem>,
}

#[derive(Debug, Deserialize, Serialize)]
struct SearchResponse {
    query: String,
    total_moments: u64,
    total_videos: u64,
    total_found: u64,
    out_of: u64,
    time_to_first_result_ms: u64,
    page: u32,
    per_page: u8,
    sort: SearchSort,
    #[serde(default)]
    mode: SearchMode,
    has_next_page: bool,
    hits: Vec<SearchHit>,
    videos: Vec<SearchVideoResult>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchSort {
    #[default]
    Relevance,
    RelevanceEngagement,
    ViewsDesc,
    LikesDesc,
    CommentsDesc,
    PublishedDesc,
    PublishedAsc,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SearchMode {
    #[default]
    Lexical,
    Semantic,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SearchHit {
    id: String,
    video_id: String,
    title: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    prev_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_highlight: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_highlight: Option<String>,
    match_kind: MatchKind,
    youtuber_handle: String,
    youtuber_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_at: Option<String>,
    view_count: u64,
    like_count: u64,
    comment_count: u64,
    content_kind: ContentKind,
    t_start_ms: u32,
    t_end_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    youtube_url: String,
    embed_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SearchVideoResult {
    video_id: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_highlight: Option<String>,
    youtuber_handle: String,
    youtuber_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    published_at: Option<String>,
    view_count: u64,
    like_count: u64,
    comment_count: u64,
    content_kind: ContentKind,
    match_count: usize,
    youtube_url: String,
    embed_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_timestamp: Option<String>,
    hits: Vec<SearchHit>,
}

#[derive(Clone, Debug, Serialize)]
struct SearchLogEvent {
    event: &'static str,
    backend: String,
    query: String,
    youtuber: Option<String>,
    client_country: Option<String>,
    cf_ray: Option<String>,
    user_agent: Option<String>,
    page: u32,
    per_page: u8,
    backend_page: u32,
    backend_page_size: u8,
    sort: SearchSort,
    total_found: u64,
    out_of: u64,
    returned_videos: usize,
    returned_hits: usize,
    has_next_page: bool,
    duration_ms: u64,
    backend_search_ms: u64,
    first_backend_search_ms: u64,
    result_processing_ms: u64,
    backend_search_calls: u32,
    backend_raw_hits: usize,
    backend_grouped_hits: usize,
    response_body_bytes: usize,
    cache_control: &'static str,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MatchKind {
    Transcript,
    Title,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ContentKind {
    Video,
    Short,
    Unknown,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("missing required environment variable {0}")]
    MissingEnv(&'static str),
    #[error("invalid bind address: {0}")]
    InvalidBindAddress(String),
    #[error("invalid search backend: {0}")]
    InvalidSearchBackend(String),
    #[error("query cannot be empty")]
    EmptyQuery,
    #[error("{0}")]
    BadRequest(String),
    #[error("rate limited; retry after {0} seconds")]
    RateLimited(u64),
    #[error("unauthorized")]
    Unauthorized,
    #[error("invalid handle")]
    InvalidHandle,
    #[error("queue failed: {0}")]
    Queue(String),
    #[error("channel image failed: {0}")]
    ChannelImage(String),
    #[error("search backend failed: {0}")]
    SearchBackend(#[from] SearchBackendError),
    #[error("search response had an unexpected shape")]
    UnexpectedSearchShape,
    #[error("server failed: {0}")]
    Server(#[from] std::io::Error),
}

#[tokio::main]
async fn main() -> Result<(), ApiError> {
    let _ = dotenvy::dotenv();
    init_tracing();

    let backend = env::var("YTSEARCH_SEARCH_BACKEND").unwrap_or_else(|_| "typesense".to_string());
    let collection = collection_env(&backend);
    let bind_addr = env::var("YTSEARCH_API_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let bind_addr: SocketAddr = bind_addr
        .parse()
        .map_err(|error: AddrParseError| ApiError::InvalidBindAddress(error.to_string()))?;

    let search_client = search_backend(&backend, collection)?;
    let allowed_origins = allowed_browser_origins();
    let state = Arc::new(AppState {
        search: search_client,
        search_backend: backend,
        engagement_sorts_enabled: env_bool("YTSEARCH_ENGAGEMENT_SORTS", false),
        preserve_backend_relevance_order: env_bool(
            "YTSEARCH_PRESERVE_BACKEND_RELEVANCE_ORDER",
            false,
        ),
        axiom: AxiomLogger::from_env(),
        rate_limiter: Arc::new(Mutex::new(RateLimiter::default())),
        search_rate_limit_per_minute: env_usize(
            "YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE",
            DEFAULT_SEARCH_RATE_LIMIT_PER_MINUTE,
        ),
        index_queue: IndexQueue {
            path: env::var("YTSEARCH_INDEX_QUEUE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data/index-requests.jsonl")),
            token: env::var("YTSEARCH_INDEX_QUEUE_TOKEN").ok(),
        },
        image_http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| ApiError::ChannelImage(format!("client setup failed: {error}")))?,
        semantic_search_enabled: env::var("YTSEARCH_SEMANTIC_SEARCH_ENABLED")
            .is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "yes")),
        semantic_embedding_field: env::var("YTSEARCH_SEMANTIC_EMBEDDING_FIELD")
            .unwrap_or_else(|_| "embedding".to_string()),
        semantic_lexical_api_url: env::var("YTSEARCH_SEMANTIC_LEXICAL_API_URL")
            .ok()
            .map(|value| value.trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty()),
    });
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/youtubers", get(youtubers_handler))
        .route(
            "/api/channel-images/{channel_id}",
            get(channel_image_handler),
        )
        .route("/api/search", get(search_handler))
        .route(
            "/api/index-requests",
            post(create_index_request).get(list_index_requests),
        )
        .route("/api/index-status", get(index_status))
        .route("/api/index-activity", post(update_index_activity))
        .route("/api/index-requests/{id}", delete(ack_index_request))
        .route(
            "/api/index-requests/{id}/approve",
            post(approve_index_request),
        )
        .route(
            "/api/index-requests/{id}/reject",
            post(reject_index_request),
        )
        .route(
            "/api/index-requests/{id}/status",
            axum::routing::patch(update_index_request_status),
        )
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(move |origin, _| {
                    browser_origin_allowed(origin, &allowed_origins)
                }))
                .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
                .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]),
        )
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    tracing::info!(%bind_addr, "starting ytsearch API");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn allowed_browser_origins() -> Vec<String> {
    env::var("YTSEARCH_ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "https://search.pronsh.dev".to_string())
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(|origin| origin.trim_end_matches('/').to_string())
        .collect()
}

fn browser_origin_allowed(origin: &HeaderValue, configured: &[String]) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    if configured.iter().any(|allowed| allowed == origin) {
        return true;
    }
    let Ok(url) = reqwest::Url::parse(origin) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && matches!(
            url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        )
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

async fn channel_image_handler(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<String>,
) -> Result<Response, ApiError> {
    if !is_valid_channel_id(&channel_id) {
        return Err(ApiError::BadRequest("invalid channel image id".to_string()));
    }

    let avatar_url = state
        .search
        .list_youtubers()
        .await?
        .into_iter()
        .find(|channel| channel.channel_id == channel_id)
        .and_then(|channel| channel.avatar_url)
        .filter(|url| is_allowed_youtube_image_url(url))
        .ok_or_else(|| ApiError::BadRequest("channel image unavailable".to_string()))?;

    let upstream = state
        .image_http
        .get(avatar_url)
        .send()
        .await
        .map_err(|error| ApiError::ChannelImage(format!("request failed: {error}")))?
        .error_for_status()
        .map_err(|error| ApiError::ChannelImage(format!("response failed: {error}")))?;
    let content_type = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("image/"))
        .unwrap_or("image/jpeg")
        .to_string();
    if upstream
        .content_length()
        .is_some_and(|length| length > 2_000_000)
    {
        return Err(ApiError::ChannelImage("image is too large".to_string()));
    }
    let bytes = upstream
        .bytes()
        .await
        .map_err(|error| ApiError::ChannelImage(format!("body failed: {error}")))?;
    if bytes.len() > 2_000_000 {
        return Err(ApiError::ChannelImage("image is too large".to_string()));
    }

    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("image/jpeg")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CHANNEL_IMAGE),
    );
    Ok(response)
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
    let mode = params.mode.unwrap_or_default();
    if mode == SearchMode::Semantic && !state.semantic_search_enabled {
        return Err(ApiError::BadRequest(
            "semantic search is not enabled on this deployment".to_string(),
        ));
    }

    let per_page = params
        .per_page
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(1, MAX_PER_PAGE);
    let page = params.page.unwrap_or(1).max(1);
    let mut sort = params.sort.unwrap_or_default();
    validate_search_sort(sort, state.engagement_sorts_enabled)?;
    let low_signal_query = is_low_signal_query(query);
    if sort == SearchSort::Relevance && low_signal_query {
        sort = SearchSort::PublishedDesc;
    }
    let search_started = Instant::now();
    let video_scope = params.video_id.is_some();
    // Let the backend paginate video cards so long transcripts cannot crowd all
    // other videos out of the bounded result window. A video-scoped request is
    // the exception because it intentionally loads every matching moment.
    let use_backend_grouping = !video_scope;
    let search_page_size = if video_scope {
        250
    } else if use_backend_grouping {
        per_page
    } else {
        per_page.saturating_mul(8).min(MAX_PER_PAGE)
    };
    let requested_offset = (page - 1).saturating_mul(u32::from(per_page));
    let search_backend_page = if use_backend_grouping { page } else { 1 };
    let local_offset = if use_backend_grouping {
        0
    } else {
        requested_offset as usize
    };
    let backend_query = if low_signal_query { "*" } else { query };
    let mut search_query = TextSearchQuery::new(backend_query)
        .query_by("text,title")
        .highlight_fields("text,title")
        .per_page(search_page_size)
        .page(search_backend_page);
    if mode == SearchMode::Semantic {
        search_query = search_query
            .query_by(format!("text,title,{}", state.semantic_embedding_field))
            .hybrid(
                format!(
                    "{}:([], alpha: 0.3, k: 200)",
                    state.semantic_embedding_field
                ),
                false,
            );
    }
    search_query = match sort {
        SearchSort::PublishedDesc => search_query.sort_by("published_at_timestamp:desc"),
        SearchSort::PublishedAsc => search_query.sort_by("published_at_timestamp:asc"),
        SearchSort::RelevanceEngagement => search_query.sort_by("engagement_score:desc"),
        SearchSort::ViewsDesc => search_query.sort_by("view_count:desc"),
        SearchSort::LikesDesc => search_query.sort_by("like_count:desc"),
        SearchSort::CommentsDesc => search_query.sort_by("comment_count:desc"),
        SearchSort::Relevance => search_query,
    };
    if use_backend_grouping {
        search_query = search_query.group_by("video_id").group_limit(5);
    } else {
        search_query = search_query.without_grouping();
    }
    tracing::info!(
        query,
        per_page,
        page,
        ?sort,
        ?mode,
        search_page = search_backend_page,
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
        search_query = search_query.youtuber_handle(normalize_handle(youtuber));
    }
    if let Some(video_id) = params
        .video_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !video_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            return Err(ApiError::BadRequest("invalid video id".to_string()));
        }
        search_query = search_query.filter_by(format!("video_id:={video_id}"));
    }

    let first_search = async {
        let first_backend_started = Instant::now();
        let raw = state.search.search(&search_query).await?;
        Ok::<_, ApiError>((raw, first_backend_started.elapsed().as_millis() as u64))
    };
    let totals_query = search_query.clone().without_grouping().per_page(1).page(1);
    let ((first_raw, first_backend_search_ms), total_videos, ungrouped_totals) = tokio::try_join!(
        first_search,
        grouped_total_count(state.search.as_ref(), &search_query),
        async {
            if use_backend_grouping {
                state
                    .search
                    .search(&totals_query)
                    .await
                    .map(Some)
                    .map_err(ApiError::from)
            } else {
                Ok::<_, ApiError>(None)
            }
        },
    )?;
    let totals_raw = ungrouped_totals.as_ref().unwrap_or(&first_raw);
    let found = backend_total_moments(&first_raw).unwrap_or_else(|| backend_found(totals_raw));
    let out_of = backend_out_of(totals_raw);
    let processing_started = Instant::now();
    let mut page_result = collect_visible_videos(
        state.search.as_ref(),
        &search_query,
        first_raw,
        search_backend_page,
        local_offset,
        per_page,
        search_page_size,
        sort,
        query,
        state.preserve_backend_relevance_order,
        first_backend_search_ms,
        if video_scope {
            100
        } else {
            MAX_SEARCH_PAGE_FETCHES
        },
    )
    .await?;
    if use_backend_grouping {
        page_result.has_next_page =
            has_more_grouped_videos(total_videos, requested_offset, page_result.videos.len());
    }
    let result_processing_ms = processing_started.elapsed().as_millis() as u64;
    let videos = page_result.videos;
    let hits = videos
        .iter()
        .flat_map(|video| video.hits.iter().cloned())
        .collect::<Vec<_>>();
    let mut response = SearchResponse {
        query: query.to_string(),
        total_moments: found.max(hits.len() as u64),
        total_videos: total_videos.max(videos.len() as u64),
        total_found: found.max(hits.len() as u64),
        out_of,
        time_to_first_result_ms: first_backend_search_ms,
        page,
        per_page,
        sort,
        mode,
        has_next_page: page_result.has_next_page,
        hits,
        videos,
    };
    if mode == SearchMode::Semantic
        && let Some(base_url) = &state.semantic_lexical_api_url
    {
        match fetch_lexical_fallback(
            &state.image_http,
            base_url,
            query,
            params.youtuber.as_deref(),
            page,
            per_page,
            sort,
        )
        .await
        {
            Ok(lexical) => response = fuse_search_responses(lexical, response),
            Err(error) => tracing::warn!(%error, "semantic lexical fallback failed"),
        }
    }
    let response_body_bytes = serde_json::to_vec(&response).map_or(0, |body| body.len());
    tracing::info!(
        query,
        found,
        returned_videos = response.videos.len(),
        returned_hits = response.hits.len(),
        backend_search_ms = page_result.timings.backend_search_ms,
        first_backend_search_ms,
        result_processing_ms,
        response_body_bytes,
        "search response"
    );
    if let Some(axiom) = &state.axiom {
        axiom.log_search(SearchLogEvent {
            event: "search_query",
            backend: state.search_backend.clone(),
            query: query.to_string(),
            youtuber: params.youtuber.clone(),
            client_country: header_string(&headers, "cf-ipcountry"),
            cf_ray: header_string(&headers, "cf-ray"),
            user_agent: header_string(&headers, "user-agent"),
            page,
            per_page,
            backend_page: search_backend_page,
            backend_page_size: search_page_size,
            sort,
            total_found: response.total_found,
            out_of,
            returned_videos: response.videos.len(),
            returned_hits: response.hits.len(),
            has_next_page: response.has_next_page,
            duration_ms: search_started.elapsed().as_millis() as u64,
            backend_search_ms: page_result.timings.backend_search_ms,
            first_backend_search_ms,
            result_processing_ms,
            backend_search_calls: page_result.timings.backend_search_calls,
            backend_raw_hits: page_result.timings.backend_raw_hits,
            backend_grouped_hits: page_result.timings.backend_grouped_hits,
            response_body_bytes,
            cache_control: CACHE_SEARCH,
        });
    }

    Ok(cache_response(CACHE_SEARCH, Json(response)))
}

async fn fetch_lexical_fallback(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    youtuber: Option<&str>,
    page: u32,
    per_page: u8,
    sort: SearchSort,
) -> Result<SearchResponse, String> {
    let mut url = reqwest::Url::parse(&format!("{base_url}/api/search"))
        .map_err(|error| error.to_string())?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("q", query);
        pairs.append_pair("page", &page.to_string());
        pairs.append_pair("per_page", &per_page.to_string());
        pairs.append_pair("mode", "lexical");
        pairs.append_pair(
            "sort",
            match sort {
                SearchSort::Relevance => "relevance",
                SearchSort::RelevanceEngagement => "relevance_engagement",
                SearchSort::ViewsDesc => "views_desc",
                SearchSort::LikesDesc => "likes_desc",
                SearchSort::CommentsDesc => "comments_desc",
                SearchSort::PublishedDesc => "published_desc",
                SearchSort::PublishedAsc => "published_asc",
            },
        );
        if let Some(youtuber) = youtuber.filter(|value| !value.trim().is_empty()) {
            pairs.append_pair("youtuber", youtuber);
        }
    }
    client
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())
}

fn fuse_search_responses(lexical: SearchResponse, semantic: SearchResponse) -> SearchResponse {
    const RANK_CONSTANT: f64 = 60.0;
    let mut scores = HashMap::<String, f64>::new();
    let mut videos = HashMap::<String, SearchVideoResult>::new();
    for (weight, response) in [(0.7, &lexical), (0.3, &semantic)] {
        for (offset, video) in response.videos.iter().enumerate() {
            *scores.entry(video.video_id.clone()).or_default() +=
                weight / (RANK_CONSTANT + offset as f64 + 1.0);
            videos
                .entry(video.video_id.clone())
                .or_insert_with(|| video.clone());
        }
    }
    let mut ranked = videos.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        scores[&right.video_id]
            .total_cmp(&scores[&left.video_id])
            .then_with(|| left.video_id.cmp(&right.video_id))
    });
    ranked.truncate(usize::from(semantic.per_page));
    let hits = ranked
        .iter()
        .flat_map(|video| video.hits.iter().cloned())
        .collect::<Vec<_>>();

    SearchResponse {
        query: semantic.query,
        total_moments: lexical.total_moments.max(semantic.total_moments),
        total_videos: lexical.total_videos.max(semantic.total_videos),
        total_found: lexical.total_found.max(semantic.total_found),
        out_of: lexical.out_of.max(semantic.out_of),
        time_to_first_result_ms: lexical
            .time_to_first_result_ms
            .min(semantic.time_to_first_result_ms),
        page: semantic.page,
        per_page: semantic.per_page,
        sort: semantic.sort,
        mode: SearchMode::Semantic,
        has_next_page: lexical.has_next_page || semantic.has_next_page,
        hits,
        videos: ranked,
    }
}

async fn grouped_total_count(
    search: &dyn TranscriptSearchBackend,
    base_query: &TextSearchQuery,
) -> Result<u64, ApiError> {
    Ok(search.count_distinct(base_query, "video_id").await?)
}

async fn create_index_request(
    State(state): State<Arc<AppState>>,
    Json(input): Json<IndexRequestInput>,
) -> Result<Response, ApiError> {
    let handle = normalize_handle_checked(&input.handle)?;
    let requested_at = epoch_seconds();
    let note = input
        .note
        .map(|note| note.trim().to_string())
        .filter(|note| !note.is_empty());
    let backends = queue_target_backends(input.backends.as_deref())?;
    let mut requests = Vec::with_capacity(backends.len());
    for target_backend in backends {
        let request = IndexQueueItem {
            id: format!(
                "{}-{}-{}",
                requested_at,
                target_backend,
                handle.trim_start_matches('@').replace('-', "")
            ),
            handle: handle.clone(),
            note: note.clone(),
            requested_at,
            source_backend: state.search_backend.clone(),
            target_backend: Some(target_backend),
            status: "pending".to_string(),
        };
        requests.push(state.index_queue.enqueue(request).await?);
    }
    let request = requests
        .first()
        .cloned()
        .ok_or_else(|| ApiError::Queue("no target backends selected".to_string()))?;
    Ok(cache_response(
        CACHE_NO_STORE,
        (
            StatusCode::ACCEPTED,
            Json(IndexQueueResponse { request, requests }),
        ),
    ))
}

async fn list_index_requests(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<IndexQueueParams>,
) -> Result<Response, ApiError> {
    state.index_queue.authorize(&headers)?;
    let requests = state
        .index_queue
        .list(params.limit.unwrap_or(100).clamp(1, 1000))
        .await?;
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(IndexQueueListResponse { requests }),
    ))
}

async fn index_status(
    State(state): State<Arc<AppState>>,
    Query(params): Query<IndexStatusParams>,
) -> Result<Response, ApiError> {
    let handle = params
        .handle
        .as_deref()
        .map(normalize_handle_checked)
        .transpose()?;
    let activity = state.index_queue.activity().await?;
    if let Some(activity) = activity.filter(|activity| {
        handle
            .as_ref()
            .is_none_or(|handle| activity.handle == *handle)
    }) {
        let channel = state
            .search
            .list_youtubers()
            .await?
            .into_iter()
            .find(|channel| channel.handle.eq_ignore_ascii_case(&activity.handle));
        return Ok(cache_response(
            CACHE_NO_STORE,
            Json(serde_json::json!({
                "handle": activity.handle,
                "status": "running",
                "request_id": format!("active-{}", activity.started_at),
                "target_backend": activity.target_backend,
                "channel": channel,
            })),
        ));
    }
    let request = state
        .index_queue
        .list(usize::MAX)
        .await?
        .into_iter()
        .find(|request| {
            (matches!(request.status.as_str(), "running" | "queued")
                || (handle.is_some()
                    && matches!(request.status.as_str(), "pending" | "failed" | "rejected")))
                && handle
                    .as_ref()
                    .is_none_or(|handle| request.handle == *handle)
        });
    let channel = match request.as_ref() {
        Some(request) => state
            .search
            .list_youtubers()
            .await?
            .into_iter()
            .find(|channel| channel.handle.eq_ignore_ascii_case(&request.handle)),
        None => None,
    };
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(serde_json::json!({
            "handle": request.as_ref().map(|item| item.handle.as_str()).unwrap_or(""),
            "status": request.as_ref().map(|item| item.status.as_str()).unwrap_or("idle"),
            "request_id": request.as_ref().map(|item| item.id.clone()),
            "target_backend": request.and_then(|item| item.target_backend),
            "channel": channel,
        })),
    ))
}

async fn update_index_activity(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<IndexActivityInput>,
) -> Result<Response, ApiError> {
    state.index_queue.authorize(&headers)?;
    let activity = match input.handle {
        Some(handle) => Some(IndexActivity {
            handle: normalize_handle_checked(&handle)?,
            started_at: epoch_seconds(),
            target_backend: input
                .target_backend
                .unwrap_or_else(|| "typesense-fly".to_string()),
        }),
        None => None,
    };
    state.index_queue.set_activity(activity).await?;
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(serde_json::json!({ "ok": true })),
    ))
}

async fn ack_index_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.index_queue.authorize(&headers)?;
    state.index_queue.ack(&id).await?;
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(serde_json::json!({ "ok": true })),
    ))
}

async fn approve_index_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.index_queue.authorize(&headers)?;
    let request = state
        .index_queue
        .set_status_from(&id, "queued", &["pending", "rejected", "failed"])
        .await?;
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(serde_json::json!({ "request": request })),
    ))
}

async fn reject_index_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.index_queue.authorize(&headers)?;
    let request = state
        .index_queue
        .set_status_from(&id, "rejected", &["pending"])
        .await?;
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(serde_json::json!({ "request": request })),
    ))
}

#[derive(Debug, Deserialize)]
struct IndexRequestStatusInput {
    status: String,
}

async fn update_index_request_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<IndexRequestStatusInput>,
) -> Result<Response, ApiError> {
    state.index_queue.authorize(&headers)?;
    if !is_valid_index_request_status(&input.status) {
        return Err(ApiError::BadRequest(
            "invalid index request status".to_string(),
        ));
    }
    let request = state.index_queue.set_status(&id, &input.status).await?;
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(serde_json::json!({ "request": request })),
    ))
}

fn is_valid_index_request_status(status: &str) -> bool {
    matches!(
        status,
        "pending" | "queued" | "running" | "failed" | "rejected"
    )
}

fn is_searchable_channel(channel: &YoutuberDocument) -> bool {
    channel.indexed_video_count > 0 && channel.indexed_chunk_count > 0
}

fn queue_target_backends(input: Option<&[String]>) -> Result<Vec<String>, ApiError> {
    let requested = input
        .map(|backends| backends.iter().map(String::as_str).collect::<Vec<_>>())
        .unwrap_or_else(|| vec!["typesense-fly"]);
    let mut normalized = Vec::new();
    for backend in requested {
        let backend = backend.trim();
        if backend.is_empty() {
            continue;
        }
        let backend = match backend {
            "typesense-fly" | "typesense" => "typesense-fly",
            other => {
                return Err(ApiError::BadRequest(format!(
                    "unsupported target backend: {other}"
                )));
            }
        };
        if !normalized.iter().any(|existing| existing == backend) {
            normalized.push(backend.to_string());
        }
    }
    if normalized.is_empty() {
        return Err(ApiError::BadRequest(
            "at least one target backend is required".to_string(),
        ));
    }
    Ok(normalized)
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

impl IndexQueue {
    fn activity_path(&self) -> PathBuf {
        self.path.with_extension("activity.json")
    }

    async fn activity(&self) -> Result<Option<IndexActivity>, ApiError> {
        let path = self.activity_path();
        let activity = match tokio::fs::read_to_string(path).await {
            Ok(body) => serde_json::from_str::<IndexActivity>(&body).ok(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ApiError::Server(error)),
        };
        Ok(activity.filter(|activity| epoch_seconds().saturating_sub(activity.started_at) <= 900))
    }

    async fn set_activity(&self, activity: Option<IndexActivity>) -> Result<(), ApiError> {
        let path = self.activity_path();
        match activity {
            Some(activity) => {
                let body = serde_json::to_string(&activity)
                    .map_err(|error| ApiError::Queue(error.to_string()))?;
                tokio::fs::write(path, body).await?;
            }
            None => match tokio::fs::remove_file(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ApiError::Server(error)),
            },
        }
        Ok(())
    }
    async fn enqueue(&self, request: IndexQueueItem) -> Result<IndexQueueItem, ApiError> {
        if let Some(existing) = self.list(usize::MAX).await?.into_iter().find(|queued| {
            queued.handle == request.handle
                && matches!(queued.status.as_str(), "pending" | "queued" | "running")
                && queued.target_backend == request.target_backend
        }) {
            return Ok(existing);
        }

        self.append(&request).await?;
        Ok(request)
    }

    async fn append(&self, request: &IndexQueueItem) -> Result<(), ApiError> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let line =
            serde_json::to_string(request).map_err(|error| ApiError::Queue(error.to_string()))?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        use tokio::io::AsyncWriteExt;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(())
    }

    async fn list(&self, limit: usize) -> Result<Vec<IndexQueueItem>, ApiError> {
        let body = match tokio::fs::read_to_string(&self.path).await {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(ApiError::Server(error)),
        };
        Ok(body
            .lines()
            .filter_map(|line| serde_json::from_str::<IndexQueueItem>(line).ok())
            .take(limit)
            .collect())
    }

    async fn ack(&self, id: &str) -> Result<(), ApiError> {
        let requests = self
            .list(usize::MAX)
            .await?
            .into_iter()
            .filter(|request| request.id != id)
            .collect::<Vec<_>>();
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut body = String::new();
        for request in requests {
            let line = serde_json::to_string(&request)
                .map_err(|error| ApiError::Queue(error.to_string()))?;
            body.push_str(&line);
            body.push('\n');
        }
        tokio::fs::write(&self.path, body).await?;
        Ok(())
    }

    async fn set_status(&self, id: &str, status: &str) -> Result<IndexQueueItem, ApiError> {
        let mut requests = self.list(usize::MAX).await?;
        let request = requests
            .iter_mut()
            .find(|request| request.id == id)
            .ok_or_else(|| ApiError::BadRequest("unknown index request".to_string()))?;
        request.status = status.to_string();
        let updated = request.clone();
        let body = requests
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ApiError::Queue(error.to_string()))?
            .join("\n");
        tokio::fs::write(&self.path, format!("{body}\n")).await?;
        Ok(updated)
    }

    async fn set_status_from(
        &self,
        id: &str,
        status: &str,
        allowed_statuses: &[&str],
    ) -> Result<IndexQueueItem, ApiError> {
        let mut requests = self.list(usize::MAX).await?;
        let request = requests
            .iter_mut()
            .find(|request| request.id == id)
            .ok_or_else(|| ApiError::BadRequest("unknown index request".to_string()))?;
        if request.status == status {
            return Ok(request.clone());
        }
        if !allowed_statuses.contains(&request.status.as_str()) {
            return Err(ApiError::BadRequest(format!(
                "cannot change index request from {} to {status}",
                request.status
            )));
        }
        request.status = status.to_string();
        let updated = request.clone();
        let body = requests
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ApiError::Queue(error.to_string()))?
            .join("\n");
        tokio::fs::write(&self.path, format!("{body}\n")).await?;
        Ok(updated)
    }

    fn authorize(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        let Some(expected) = &self.token else {
            return Err(ApiError::Unauthorized);
        };
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        let explicit = headers
            .get("x-index-queue-token")
            .and_then(|value| value.to_str().ok());
        if bearer == Some(expected.as_str()) || explicit == Some(expected.as_str()) {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        }
    }
}

struct SearchPageResult {
    videos: Vec<SearchVideoResult>,
    has_next_page: bool,
    timings: SearchCollectionTimings,
}

#[derive(Clone, Copy, Debug, Default)]
struct SearchCollectionTimings {
    backend_search_calls: u32,
    backend_search_ms: u64,
    backend_raw_hits: usize,
    backend_grouped_hits: usize,
}

#[allow(clippy::too_many_arguments)]
async fn collect_visible_videos(
    search: &dyn TranscriptSearchBackend,
    base_query: &TextSearchQuery,
    first_raw: Value,
    first_search_backend_page: u32,
    local_offset: usize,
    per_page: u8,
    search_page_size: u8,
    sort: SearchSort,
    query: &str,
    preserve_backend_relevance_order: bool,
    first_backend_search_ms: u64,
    max_page_fetches: u32,
) -> Result<SearchPageResult, ApiError> {
    let required_videos = local_offset
        .saturating_add(usize::from(per_page))
        .saturating_add(1);
    let mut combined = Vec::new();
    let mut current_page = first_search_backend_page;
    let mut raw = first_raw;
    let mut timings = SearchCollectionTimings {
        backend_search_calls: 1,
        backend_search_ms: first_backend_search_ms,
        ..SearchCollectionTimings::default()
    };

    for fetch_index in 0..max_page_fetches {
        timings.backend_raw_hits = timings
            .backend_raw_hits
            .saturating_add(raw_hits_count(&raw));
        timings.backend_grouped_hits = timings
            .backend_grouped_hits
            .saturating_add(grouped_hits_count(&raw));
        let page_hits = parse_hits(&raw)?;
        let is_empty = page_hits.is_empty();
        combined.extend(page_hits);
        combined = merge_nearby_transcript_hits(dedupe_title_only_hits(combined));
        let videos = video_results_from_hits(
            &combined,
            sort,
            query,
            base_query.vector_query.is_some() || preserve_backend_relevance_order,
        );

        if videos.len() >= required_videos || is_empty {
            return Ok(page_result_from_videos(
                videos,
                local_offset,
                per_page,
                timings,
            ));
        }

        let fetched_raw_hits = raw
            .get("hits")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        if fetched_raw_hits < usize::from(search_page_size) {
            break;
        }

        if fetch_index + 1 == max_page_fetches {
            break;
        }

        current_page = current_page.saturating_add(1);
        let backend_started = Instant::now();
        raw = search
            .search(&base_query.clone().page(current_page))
            .await?;
        timings.backend_search_calls = timings.backend_search_calls.saturating_add(1);
        timings.backend_search_ms = timings
            .backend_search_ms
            .saturating_add(backend_started.elapsed().as_millis() as u64);
    }

    Ok(page_result_from_videos(
        video_results_from_hits(
            &combined,
            sort,
            query,
            base_query.vector_query.is_some() || preserve_backend_relevance_order,
        ),
        local_offset,
        per_page,
        timings,
    ))
}

fn page_result_from_videos(
    videos: Vec<SearchVideoResult>,
    local_offset: usize,
    per_page: u8,
    timings: SearchCollectionTimings,
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
        timings,
    }
}

fn has_more_grouped_videos(total_videos: u64, requested_offset: u32, returned: usize) -> bool {
    u64::from(requested_offset).saturating_add(returned as u64) < total_videos
}

fn backend_found(raw: &Value) -> u64 {
    raw.get("estimatedTotalHits")
        .or_else(|| raw.get("totalHits"))
        .or_else(|| raw.get("found"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

fn backend_total_moments(raw: &Value) -> Option<u64> {
    raw.get("total_moments").and_then(Value::as_u64)
}

fn backend_out_of(raw: &Value) -> u64 {
    raw.get("estimatedTotalHits")
        .or_else(|| raw.get("totalHits"))
        .or_else(|| raw.get("out_of"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
}

fn video_results_from_hits(
    hits: &[SearchHit],
    sort: SearchSort,
    query: &str,
    preserve_backend_order: bool,
) -> Vec<SearchVideoResult> {
    let terms = searchable_terms(query);
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
            if video
                .hits
                .iter()
                .any(|hit| hit.match_kind == MatchKind::Transcript)
            {
                video
                    .hits
                    .retain(|hit| hit.match_kind == MatchKind::Transcript);
            }
            video.match_count = video.hits.len();
            match sort {
                SearchSort::Relevance if preserve_backend_order => {}
                SearchSort::Relevance | SearchSort::RelevanceEngagement => {
                    video.hits.sort_by_cached_key(|hit| {
                        (Reverse(hit_relevance_score(hit, &terms)), hit.t_start_ms)
                    })
                }
                SearchSort::PublishedDesc
                | SearchSort::PublishedAsc
                | SearchSort::ViewsDesc
                | SearchSort::LikesDesc
                | SearchSort::CommentsDesc => {
                    video.hits.sort_by_key(|hit| hit.t_start_ms);
                }
            }
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

    if preserve_backend_order {
        return videos;
    }

    match sort {
        SearchSort::Relevance => videos.sort_by_cached_key(|video| {
            (
                Reverse(video_relevance_score(video, &terms)),
                Reverse(video.match_count),
                Reverse(video.published_at.clone()),
                video.title.clone(),
            )
        }),
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
        SearchSort::RelevanceEngagement
        | SearchSort::ViewsDesc
        | SearchSort::LikesDesc
        | SearchSort::CommentsDesc => {}
    }

    videos
}

fn searchable_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() >= 2 && !is_stopword(term))
        .collect()
}

fn is_stopword(term: &str) -> bool {
    matches!(
        term,
        "a" | "an"
            | "and"
            | "are"
            | "for"
            | "i"
            | "in"
            | "is"
            | "it"
            | "of"
            | "on"
            | "or"
            | "the"
            | "to"
    )
}

fn is_low_signal_query(query: &str) -> bool {
    let searchable_chars = query
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .count();
    searchable_chars <= 1
}

fn hit_relevance_score(hit: &SearchHit, terms: &[String]) -> usize {
    if terms.is_empty() {
        return usize::from(hit.match_kind == MatchKind::Title);
    }

    let title = hit.title.to_ascii_lowercase();
    let text = hit.text.to_ascii_lowercase();
    let title_highlight = hit
        .title_highlight
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let text_highlight = hit
        .text_highlight
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();

    terms
        .iter()
        .map(|term| {
            let mut score = 0;
            if title.contains(term) {
                score += 3;
            }
            if title_highlight.contains(term) {
                score += 2;
            }
            if text_highlight.contains(term) {
                score += 2;
            }
            if text.contains(term) {
                score += 1;
            }
            score
        })
        .sum()
}

// Backends retrieve the candidate moments. This deterministic pass favors
// videos that cover more meaningful query terms, especially in their title,
// while retaining backend ranking as a tie-breaker through stable input order.
fn video_relevance_score(video: &SearchVideoResult, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }

    let title = video.title.to_ascii_lowercase();
    let title_compact = alphanumeric_compact(&title);
    let joined_terms = terms.join("");
    let mut score = video
        .hits
        .iter()
        .map(|hit| hit_relevance_score(hit, terms))
        .max()
        .unwrap_or_default();

    for term in terms {
        if title.contains(term) {
            score += 8;
        }
        if video.hits.iter().any(|hit| {
            hit.title.to_ascii_lowercase().contains(term)
                || hit.text.to_ascii_lowercase().contains(term)
        }) {
            score += 3;
        }
    }
    if terms.len() > 1 && title_compact.contains(&joined_terms) {
        score += 12;
    }
    score
}

fn alphanumeric_compact(value: &str) -> String {
    value.chars().filter(char::is_ascii_alphanumeric).collect()
}

impl SearchVideoResult {
    fn from_hit(hit: &SearchHit) -> Self {
        Self {
            video_id: hit.video_id.clone(),
            title: hit.title.clone(),
            title_highlight: hit.title_highlight.clone(),
            youtuber_handle: hit.youtuber_handle.clone(),
            youtuber_name: hit.youtuber_name.clone(),
            thumbnail: hit.thumbnail.clone(),
            published_at: hit.published_at.clone(),
            view_count: hit.view_count,
            like_count: hit.like_count,
            comment_count: hit.comment_count,
            content_kind: hit.content_kind,
            match_count: 0,
            youtube_url: hit.youtube_url.clone(),
            embed_url: hit.embed_url.clone(),
            best_timestamp: hit.timestamp.clone(),
            hits: Vec::new(),
        }
    }
}

fn content_kind_for_duration(duration_ms: Option<u32>) -> ContentKind {
    match duration_ms {
        Some(duration_ms) if duration_ms <= 60_000 => ContentKind::Short,
        Some(_) => ContentKind::Video,
        None => ContentKind::Unknown,
    }
}

fn parse_hits(raw: &Value) -> Result<Vec<SearchHit>, ApiError> {
    if let Some(grouped_hits) = raw.get("grouped_hits").and_then(Value::as_array) {
        let hit_count = grouped_hits
            .iter()
            .filter_map(|group| group.get("hits").and_then(Value::as_array))
            .map(Vec::len)
            .sum();
        let mut parsed = Vec::with_capacity(hit_count);
        for group in grouped_hits {
            let Some(hits) = group.get("hits").and_then(Value::as_array) else {
                continue;
            };
            for hit in hits {
                let document = hit.get("document").unwrap_or(hit);
                parsed.push(SearchHit::from_search_backend_hit(document, hit)?);
            }
        }
        return Ok(parsed);
    }

    let Some(hits) = raw.get("hits").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut parsed = Vec::with_capacity(hits.len());
    for hit in hits {
        let document = hit.get("document").unwrap_or(hit);
        parsed.push(SearchHit::from_search_backend_hit(document, hit)?);
    }

    Ok(parsed)
}

fn raw_hits_count(raw: &Value) -> usize {
    raw.get("hits")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn grouped_hits_count(raw: &Value) -> usize {
    raw.get("grouped_hits")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn browser_origins_allow_only_configured_and_local_development() {
        let configured = vec!["https://search.pronsh.dev".to_string()];

        for allowed in [
            "https://search.pronsh.dev",
            "http://localhost:5173",
            "http://127.0.0.1:3000",
            "http://[::1]:8787",
        ] {
            assert!(browser_origin_allowed(
                &HeaderValue::from_str(allowed).unwrap(),
                &configured
            ));
        }
        for rejected in [
            "https://ytsearch-755.pages.dev",
            "https://search.pronsh.dev.evil.example",
            "https://evil.example",
            "null",
        ] {
            assert!(!browser_origin_allowed(
                &HeaderValue::from_str(rejected).unwrap(),
                &configured
            ));
        }
    }

    #[test]
    fn axiom_ingest_urls_preserve_base_paths_and_encode_dataset_names() {
        assert_eq!(
            axiom_ingest_url("https://api.axiom.co", "search events", false)
                .unwrap()
                .as_str(),
            "https://api.axiom.co/v1/datasets/search%20events/ingest"
        );
        assert_eq!(
            axiom_ingest_url("https://edge.example.com/custom/", "search/events", true)
                .unwrap()
                .as_str(),
            "https://edge.example.com/custom/v1/ingest/search%2Fevents"
        );
    }

    fn test_document(id: &str, video_id: &str, text: &str) -> Value {
        json!({
            "id": id,
            "video_id": video_id,
            "title": "Test video",
            "text": text,
            "youtuber_handle": "@test",
            "youtuber_name": "Test",
            "view_count": 1_000_000,
            "like_count": "100000",
            "comment_count": 10000.0,
            "t_start_ms": 1000,
            "t_end_ms": 2000
        })
    }

    fn test_hit(video_id: &str, title: &str, text: &str) -> SearchHit {
        let document = json!({
            "id": format!("{video_id}-1"),
            "video_id": video_id,
            "title": title,
            "text": text,
            "youtuber_handle": "@test",
            "youtuber_name": "Test",
            "t_start_ms": 1000,
            "t_end_ms": 2000,
            "published_at": "2026-01-01T00:00:00Z"
        });
        SearchHit::from_search_backend_hit(&document, &json!({})).expect("test hit")
    }

    #[test]
    fn only_proxies_canonical_youtube_channel_images() {
        assert!(is_valid_channel_id("UC6nSFpj9HTCZ5t-N3Rm3-HA"));
        assert!(!is_valid_channel_id("../etc/passwd"));
        assert!(is_allowed_youtube_image_url(
            "https://yt3.googleusercontent.com/channel-avatar=s800-c-k-c0x00ffffff-no-rj"
        ));
        assert!(is_allowed_youtube_image_url(
            "https://yt3.ggpht.com/channel-avatar=s800-c-k-c0x00ffffff-no-rj"
        ));
        assert!(!is_allowed_youtube_image_url(
            "https://example.com/avatar.png"
        ));
        assert!(!is_allowed_youtube_image_url(
            "http://yt3.ggpht.com/avatar.png"
        ));
    }

    #[test]
    fn parse_hits_flattens_typesense_grouped_hits() {
        let raw = json!({
            "grouped_hits": [
                {
                    "group_key": ["video-a"],
                    "hits": [
                        {
                            "document": test_document("a-1", "video-a", "first"),
                            "highlights": [{"field": "text", "snippet": "<mark>first</mark>"}]
                        }
                    ]
                },
                {
                    "group_key": ["video-b"],
                    "hits": [
                        {
                            "document": test_document("b-1", "video-b", "second"),
                            "highlights": [{"field": "text", "snippet": "<mark>second</mark>"}]
                        }
                    ]
                }
            ]
        });

        let hits = parse_hits(&raw).expect("grouped hits parse");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].video_id, "video-a");
        assert_eq!(hits[1].video_id, "video-b");
        assert_eq!(hits[0].view_count, 1_000_000);
        assert_eq!(hits[0].like_count, 100_000);
        assert_eq!(hits[0].comment_count, 10_000);
    }

    #[test]
    fn absent_optional_search_fields_are_omitted_from_json() {
        let mut hit = test_hit("video-a", "Result", "matching moment");
        hit.prev_text = None;
        hit.next_text = None;
        hit.text_highlight = None;
        hit.title_highlight = None;
        hit.thumbnail = None;
        hit.published_at = None;
        hit.timestamp = None;
        let hit_json = serde_json::to_value(&hit).expect("serialize hit");
        for field in [
            "prev_text",
            "next_text",
            "text_highlight",
            "title_highlight",
            "thumbnail",
            "published_at",
            "timestamp",
        ] {
            assert!(
                hit_json.get(field).is_none(),
                "unexpected null field {field}"
            );
        }

        let mut video = SearchVideoResult::from_hit(&hit);
        video.best_timestamp = None;
        let video_json = serde_json::to_value(video).expect("serialize video");
        for field in [
            "title_highlight",
            "thumbnail",
            "published_at",
            "best_timestamp",
        ] {
            assert!(
                video_json.get(field).is_none(),
                "unexpected null field {field}"
            );
        }
    }

    #[test]
    fn engagement_sorts_are_gated_to_the_experimental_backend() {
        for sort in [
            SearchSort::RelevanceEngagement,
            SearchSort::ViewsDesc,
            SearchSort::LikesDesc,
            SearchSort::CommentsDesc,
        ] {
            assert!(validate_search_sort(sort, true).is_ok());
            assert!(matches!(
                validate_search_sort(sort, false),
                Err(ApiError::BadRequest(_))
            ));
        }
        for sort in [
            SearchSort::Relevance,
            SearchSort::PublishedDesc,
            SearchSort::PublishedAsc,
        ] {
            assert!(validate_search_sort(sort, false).is_ok());
        }
    }

    #[test]
    fn engagement_results_preserve_backend_video_order_and_counts() {
        let mut popular = test_hit("popular", "Popular", "matching moment");
        popular.view_count = 9_000_000;
        popular.like_count = 700_000;
        popular.comment_count = 40_000;
        let mut niche = test_hit("niche", "Niche", "matching moment");
        niche.view_count = 1_000;
        niche.like_count = 100;
        niche.comment_count = 10;

        let videos = video_results_from_hits(
            &[popular, niche],
            SearchSort::RelevanceEngagement,
            "matching moment",
            false,
        );

        assert_eq!(videos[0].video_id, "popular");
        assert_eq!(videos[0].view_count, 9_000_000);
        assert_eq!(videos[0].like_count, 700_000);
        assert_eq!(videos[0].comment_count, 40_000);
    }

    #[test]
    fn one_character_queries_are_low_signal() {
        assert!(is_low_signal_query("I"));
        assert!(is_low_signal_query("?"));
        assert!(!is_low_signal_query("fable"));
    }

    #[tokio::test]
    async fn health_response_is_successful_and_not_cached() {
        let response = health().await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static(CACHE_NO_STORE))
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("application/json"))
        );
    }

    #[test]
    fn api_errors_use_stable_http_statuses_and_json_content_type() {
        let cases = [
            (ApiError::EmptyQuery, StatusCode::BAD_REQUEST),
            (ApiError::InvalidHandle, StatusCode::BAD_REQUEST),
            (ApiError::Unauthorized, StatusCode::UNAUTHORIZED),
            (ApiError::RateLimited(10), StatusCode::TOO_MANY_REQUESTS),
            (
                ApiError::BadRequest("bad input".to_string()),
                StatusCode::BAD_REQUEST,
            ),
        ];

        for (error, expected_status) in cases {
            let response = error.into_response();
            assert_eq!(response.status(), expected_status);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE),
                Some(&HeaderValue::from_static("application/json"))
            );
        }
    }

    #[test]
    fn search_page_slicing_reports_next_page_without_leaking_extra_result() {
        let videos: Vec<SearchVideoResult> = (0..4)
            .map(|index| {
                SearchVideoResult::from_hit(&test_hit(
                    &format!("video-{index}"),
                    "Result",
                    "matching moment",
                ))
            })
            .collect();

        let first = page_result_from_videos(videos.clone(), 0, 3, Default::default());
        assert_eq!(first.videos.len(), 3);
        assert!(first.has_next_page);

        let second = page_result_from_videos(videos, 3, 3, Default::default());
        assert_eq!(second.videos.len(), 1);
        assert!(!second.has_next_page);
    }

    #[test]
    fn grouped_pagination_uses_the_distinct_video_total() {
        assert!(has_more_grouped_videos(73, 0, 12));
        assert!(has_more_grouped_videos(73, 60, 12));
        assert!(!has_more_grouped_videos(73, 72, 1));
    }

    #[test]
    fn ungrouped_totals_preserve_the_matching_moment_count() {
        let totals = json!({ "found": 35_383, "out_of": 7_129_139 });
        assert_eq!(backend_found(&totals), 35_383);
        assert_eq!(backend_out_of(&totals), 7_129_139);
    }

    #[test]
    fn raw_grouped_totals_preserve_all_matching_moments() {
        let grouped = json!({
            "found": 15,
            "total_moments": 41,
            "grouped_hits": [{ "found": 5, "hits": [{}, {}, {}] }]
        });
        let ungrouped = json!({ "found": 99 });

        assert_eq!(backend_total_moments(&grouped), Some(41));
        assert_eq!(
            backend_total_moments(&grouped).unwrap_or_else(|| backend_found(&ungrouped)),
            41
        );
    }

    #[test]
    fn index_request_statuses_include_review_states() {
        assert!(is_valid_index_request_status("pending"));
        assert!(is_valid_index_request_status("queued"));
        assert!(is_valid_index_request_status("running"));
        assert!(is_valid_index_request_status("failed"));
        assert!(is_valid_index_request_status("rejected"));
        assert!(!is_valid_index_request_status("complete"));
    }

    #[tokio::test]
    async fn pending_requests_require_approval_before_becoming_queueable() {
        let queue = test_index_queue("approval");
        let request = test_index_request("approval-request", "pending");
        queue
            .append(&request)
            .await
            .expect("append pending request");

        let approved = queue
            .set_status_from(&request.id, "queued", &["pending", "rejected", "failed"])
            .await
            .expect("approve pending request");
        assert_eq!(approved.status, "queued");

        let approved_again = queue
            .set_status_from(&request.id, "queued", &["pending", "rejected", "failed"])
            .await
            .expect("approval is idempotent");
        assert_eq!(approved_again.status, "queued");

        let rejection = queue
            .set_status_from(&request.id, "rejected", &["pending"])
            .await;
        assert!(rejection.is_err());
        let _ = tokio::fs::remove_file(queue.path).await;
    }

    #[tokio::test]
    async fn enqueue_deduplicates_pending_requests() {
        let queue = test_index_queue("dedupe");
        let first = test_index_request("first-request", "pending");
        let duplicate = IndexQueueItem {
            id: "duplicate-request".to_string(),
            ..first.clone()
        };

        queue.enqueue(first.clone()).await.expect("enqueue first");
        let returned = queue.enqueue(duplicate).await.expect("dedupe request");
        assert_eq!(returned.id, first.id);
        assert_eq!(queue.list(usize::MAX).await.expect("list queue").len(), 1);
        let _ = tokio::fs::remove_file(queue.path).await;
    }

    fn test_index_queue(label: &str) -> IndexQueue {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        IndexQueue {
            path: std::env::temp_dir().join(format!(
                "ytsearch-{label}-{}-{nonce}.jsonl",
                std::process::id()
            )),
            token: None,
        }
    }

    fn test_index_request(id: &str, status: &str) -> IndexQueueItem {
        IndexQueueItem {
            id: id.to_string(),
            handle: "@test".to_string(),
            note: None,
            requested_at: 1,
            source_backend: "typesense".to_string(),
            target_backend: Some("typesense-fly".to_string()),
            status: status.to_string(),
        }
    }

    #[test]
    fn searchable_terms_ignore_short_terms_and_stopwords() {
        assert_eq!(searchable_terms("fable on high"), vec!["fable", "high"]);
        assert!(searchable_terms("I").is_empty());
    }

    #[test]
    fn relevance_score_prefers_specific_query_terms_over_stopword_noise() {
        let focused = SearchHit {
            text: "Use Fable on high reasoning".to_string(),
            text_highlight: Some(
                "Use <mark>Fable</mark> on <mark>high</mark> reasoning".to_string(),
            ),
            ..SearchHit::from_search_backend_hit(
                &test_document("focused", "video-a", "Use Fable on high reasoning"),
                &json!({}),
            )
            .expect("focused hit")
        };
        let noisy = SearchHit {
            text: "on and on and on".to_string(),
            text_highlight: Some("<mark>on</mark> and <mark>on</mark>".to_string()),
            ..SearchHit::from_search_backend_hit(
                &test_document("noisy", "video-a", "on and on and on"),
                &json!({}),
            )
            .expect("noisy hit")
        };
        let terms = searchable_terms("fable on high");

        assert!(hit_relevance_score(&focused, &terms) > hit_relevance_score(&noisy, &terms));
    }

    #[test]
    fn nearby_moment_coalescing_is_chronological_and_order_independent() {
        let hits = [20_000, 40_000, 0]
            .into_iter()
            .map(|start| {
                let mut hit = test_hit("video", "Video", "matching moment");
                hit.id = format!("moment-{start}");
                hit.t_start_ms = start;
                hit
            })
            .collect::<Vec<_>>();

        let merged = merge_nearby_transcript_hits(hits);
        let mut starts = merged.iter().map(|hit| hit.t_start_ms).collect::<Vec<_>>();
        starts.sort_unstable();
        assert_eq!(starts, vec![0, 40_000]);
    }

    #[test]
    fn video_results_keep_every_collected_moment() {
        let hits = (0..12)
            .map(|index| {
                let mut hit = test_hit("same-video", "Complete result", "matching moment");
                hit.id = format!("moment-{index}");
                hit.t_start_ms = index * 30_000;
                hit
            })
            .collect::<Vec<_>>();

        let videos =
            video_results_from_hits(&hits, SearchSort::Relevance, "matching moment", false);

        assert_eq!(videos[0].match_count, 12);
        assert_eq!(videos[0].hits.len(), 12);
    }

    #[test]
    fn relevance_reranker_promotes_batterygate_over_generic_battery_mentions() {
        let videos = video_results_from_hits(
            &[
                test_hit(
                    "generic",
                    "Why phones have batteries",
                    "The battery can age over time.",
                ),
                test_hit(
                    "batterygate",
                    "The BatteryGate incident explained",
                    "Apple slowed phones during the BatteryGate incident.",
                ),
            ],
            SearchSort::Relevance,
            "battery gate incident",
            false,
        );

        assert_eq!(
            videos.first().map(|video| video.video_id.as_str()),
            Some("batterygate")
        );
    }

    #[test]
    fn relevance_reranker_promotes_a_direct_linux_exploit_match() {
        let videos = video_results_from_hits(
            &[
                test_hit(
                    "generic-linux",
                    "A tour of Linux desktops",
                    "Linux has a broad software ecosystem.",
                ),
                test_hit(
                    "exploit",
                    "How the Linux exploit worked",
                    "This Linux exploit gained local privileges.",
                ),
            ],
            SearchSort::Relevance,
            "linux exploit",
            false,
        );

        assert_eq!(
            videos.first().map(|video| video.video_id.as_str()),
            Some("exploit")
        );
    }

    #[test]
    fn raw_relevance_policy_preserves_backend_video_and_moment_order() {
        let mut first_moment = test_hit(
            "backend-first",
            "A backend-ranked result",
            "A lower-scoring local substring match.",
        );
        first_moment.id = "backend-first-moment-a".to_string();
        first_moment.t_start_ms = 20_000;
        let mut second_moment = test_hit(
            "backend-first",
            "A backend-ranked result",
            "linux exploit appears here but was ranked second by the backend.",
        );
        second_moment.id = "backend-first-moment-b".to_string();
        second_moment.t_start_ms = 1_000;
        let locally_favored = test_hit(
            "local-first",
            "How the Linux exploit worked",
            "This Linux exploit gained local privileges.",
        );
        let hits = [first_moment, second_moment, locally_favored];

        let preserved =
            video_results_from_hits(&hits, SearchSort::Relevance, "linux exploit", true);
        let legacy = video_results_from_hits(&hits, SearchSort::Relevance, "linux exploit", false);

        assert_eq!(preserved[0].video_id, "backend-first");
        assert_eq!(preserved[0].hits[0].id, "backend-first-moment-a");
        assert_eq!(legacy[0].video_id, "local-first");
        assert_eq!(legacy[1].hits[0].id, "backend-first-moment-b");
    }

    #[test]
    fn semantic_grouping_preserves_title_hit_video_rank() {
        let mut ranked_title = test_hit("ranked", "C Programming Language", "title match");
        ranked_title.match_kind = MatchKind::Title;
        let videos = video_results_from_hits(
            &dedupe_title_only_hits(vec![
                ranked_title,
                test_hit("other", "Generic result", "semantic candidate"),
                test_hit("ranked", "C Programming Language", "arrays and pointers"),
            ]),
            SearchSort::Relevance,
            "c programming arrays",
            true,
        );

        assert_eq!(videos[0].video_id, "ranked");
        assert!(
            videos[0]
                .hits
                .iter()
                .all(|hit| hit.match_kind == MatchKind::Transcript)
        );
    }

    #[test]
    fn semantic_fusion_keeps_full_corpus_lexical_results() {
        let response = |mode, hits: Vec<SearchHit>| {
            let videos = video_results_from_hits(
                &hits,
                SearchSort::Relevance,
                "react server components",
                mode == SearchMode::Semantic,
            );
            SearchResponse {
                query: "react server components".to_string(),
                total_moments: videos.len() as u64,
                total_videos: videos.len() as u64,
                total_found: videos.len() as u64,
                out_of: videos.len() as u64,
                time_to_first_result_ms: 10,
                page: 1,
                per_page: 3,
                sort: SearchSort::Relevance,
                mode,
                has_next_page: false,
                hits,
                videos,
            }
        };
        let lexical = response(
            SearchMode::Lexical,
            vec![test_hit(
                "react",
                "React Server Components: A Comprehensive Breakdown",
                "How server components work.",
            )],
        );
        let semantic = response(
            SearchMode::Semantic,
            vec![test_hit(
                "unrelated",
                "Generic systems talk",
                "A loosely related software discussion.",
            )],
        );

        let fused = fuse_search_responses(lexical, semantic);

        assert_eq!(fused.videos[0].video_id, "react");
        assert!(
            fused
                .videos
                .iter()
                .any(|video| video.video_id == "unrelated")
        );
        assert_eq!(fused.mode, SearchMode::Semantic);
    }

    #[test]
    #[ignore = "manual performance comparison; run with --release --ignored --nocapture"]
    fn cached_relevance_keys_avoid_repeated_normalization() {
        const HITS: usize = 4_096;
        const ROUNDS: usize = 40;
        let terms = searchable_terms("rust memory safety compiler performance");
        let hits = (0..HITS)
            .map(|index| {
                test_hit(
                    &format!("video-{index}"),
                    if index % 3 == 0 {
                        "Rust compiler memory safety"
                    } else {
                        "A general software discussion"
                    },
                    if index % 5 == 0 {
                        "compiler performance and memory safety in Rust"
                    } else {
                        "software engineering conversation"
                    },
                )
            })
            .collect::<Vec<_>>();

        let comparison_started = Instant::now();
        let mut comparison_ids = Vec::new();
        for _ in 0..ROUNDS {
            let mut candidate = hits.clone();
            candidate.sort_by(|left, right| {
                hit_relevance_score(right, &terms)
                    .cmp(&hit_relevance_score(left, &terms))
                    .then_with(|| left.t_start_ms.cmp(&right.t_start_ms))
            });
            comparison_ids = candidate.into_iter().map(|hit| hit.id).collect();
        }
        let comparison_elapsed = comparison_started.elapsed();

        let cached_started = Instant::now();
        let mut cached_ids = Vec::new();
        for _ in 0..ROUNDS {
            let mut candidate = hits.clone();
            candidate.sort_by_cached_key(|hit| {
                (Reverse(hit_relevance_score(hit, &terms)), hit.t_start_ms)
            });
            cached_ids = candidate.into_iter().map(|hit| hit.id).collect();
        }
        let cached_elapsed = cached_started.elapsed();

        assert_eq!(comparison_ids, cached_ids);
        println!(
            "hits={HITS} rounds={ROUNDS} comparison_ms={:.2} cached_ms={:.2} speedup={:.2}",
            comparison_elapsed.as_secs_f64() * 1_000.0,
            cached_elapsed.as_secs_f64() * 1_000.0,
            comparison_elapsed.as_secs_f64() / cached_elapsed.as_secs_f64()
        );
    }
}

fn merge_nearby_transcript_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut keep = vec![true; hits.len()];
    let mut by_video = HashMap::<&str, Vec<usize>>::new();
    for (index, hit) in hits.iter().enumerate() {
        if hit.match_kind == MatchKind::Transcript {
            by_video
                .entry(hit.video_id.as_str())
                .or_default()
                .push(index);
        }
    }
    for indices in by_video.values_mut() {
        indices.sort_unstable_by_key(|index| (hits[*index].t_start_ms, *index));
        let mut cluster_start = None;
        for index in indices.iter().copied() {
            let start = hits[index].t_start_ms;
            if cluster_start.is_none_or(|cluster| {
                start.saturating_sub(cluster) > MERGE_TRANSCRIPT_HIT_WINDOW_MS
            }) {
                cluster_start = Some(start);
            } else {
                keep[index] = false;
            }
        }
    }

    hits.into_iter()
        .enumerate()
        .filter_map(|(index, hit)| keep[index].then_some(hit))
        .collect()
}

fn dedupe_title_only_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen_title_videos = std::collections::HashSet::new();
    let mut deduped = Vec::with_capacity(hits.len());

    for hit in hits {
        match hit.match_kind {
            MatchKind::Transcript => deduped.push(hit),
            MatchKind::Title => {
                if seen_title_videos.insert(hit.video_id.clone()) {
                    deduped.push(hit);
                }
            }
        }
    }

    deduped
}

impl SearchHit {
    fn from_search_backend_hit(document: &Value, hit: &Value) -> Result<Self, ApiError> {
        let id = string_field(document, "id")?;
        let video_id = string_field(document, "video_id")?;
        let t_start_ms = u32_field(document, "t_start_ms")?;
        let t_end_ms = u32_field(document, "t_end_ms")?;
        let content_kind = content_kind_for_duration(optional_u32_field(document, "duration"));
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
            view_count: optional_u64_field(document, "view_count").unwrap_or_default(),
            like_count: optional_u64_field(document, "like_count").unwrap_or_default(),
            comment_count: optional_u64_field(document, "comment_count").unwrap_or_default(),
            content_kind,
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
            hit.get("_formatted")
                .and_then(|formatted| formatted.get(field))
                .and_then(Value::as_str)
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

fn optional_u32_field(document: &Value, field: &'static str) -> Option<u32> {
    document.get(field).and_then(value_as_u32)
}

fn optional_u64_field(document: &Value, field: &'static str) -> Option<u64> {
    document.get(field).and_then(value_as_u64)
}

fn u32_field(document: &Value, field: &'static str) -> Result<u32, ApiError> {
    optional_u32_field(document, field).ok_or(ApiError::UnexpectedSearchShape)
}

fn value_as_u32(value: &Value) -> Option<u32> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .or_else(|| number.as_i64().and_then(|value| u32::try_from(value).ok()))
            .or_else(|| {
                number
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .and_then(|value| u32::try_from(value.round() as i64).ok())
            }),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_i64().and_then(|value| u64::try_from(value).ok()))
            .or_else(|| {
                number
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .map(|value| value.round() as u64)
            }),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
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

fn normalize_handle_checked(handle: &str) -> Result<String, ApiError> {
    let normalized = normalize_handle(handle);
    if normalized.len() <= 1
        || !normalized
            .strip_prefix('@')
            .unwrap_or_default()
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(ApiError::InvalidHandle);
    }
    Ok(normalized)
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

fn header_string(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn cache_response<T: IntoResponse>(cache_control: &'static str, response: T) -> Response {
    let mut response = response.into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    response
}

fn is_valid_channel_id(channel_id: &str) -> bool {
    channel_id.starts_with("UC")
        && (20..=30).contains(&channel_id.len())
        && channel_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn is_allowed_youtube_image_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("yt3.ggpht.com" | "yt3.googleusercontent.com")
        )
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn validate_search_sort(sort: SearchSort, engagement_sorts_enabled: bool) -> Result<(), ApiError> {
    if !engagement_sorts_enabled
        && matches!(
            sort,
            SearchSort::RelevanceEngagement
                | SearchSort::ViewsDesc
                | SearchSort::LikesDesc
                | SearchSort::CommentsDesc
        )
    {
        return Err(ApiError::BadRequest(
            "engagement sorting is unavailable on this search backend".to_string(),
        ));
    }
    Ok(())
}

fn collection_env(_backend: &str) -> String {
    env::var("TYPESENSE_COLLECTION").unwrap_or_else(|_| DEFAULT_COLLECTION.to_string())
}

fn search_backend(
    backend: &str,
    collection: String,
) -> Result<Arc<dyn TranscriptSearchBackend>, ApiError> {
    match backend {
        "typesense" => {
            let api_url = required_env("TYPESENSE_API_URL")?;
            let search_api_key = required_env("TYPESENSE_SEARCH_API_KEY")?;
            Ok(Arc::new(TypesenseSearchClient::new(
                TypesenseSearchConfig::new(api_url, search_api_key).with_collection(collection),
            )))
        }
        _ => Err(ApiError::InvalidSearchBackend(backend.to_string())),
    }
}

fn required_env(name: &'static str) -> Result<String, ApiError> {
    env::var(name).map_err(|_| ApiError::MissingEnv(name))
}

impl AxiomLogger {
    fn from_env() -> Option<Self> {
        let token = env::var("AXIOM_TOKEN")
            .or_else(|_| env::var("AXIOM_API_KEY"))
            .ok()?;
        let dataset = env::var("AXIOM_DATASET").ok()?;
        let (base_url, edge) = if let Ok(edge_url) = env::var("AXIOM_EDGE_URL") {
            (edge_url, true)
        } else if let Ok(edge) = env::var("AXIOM_EDGE") {
            (
                format!("https://{}", edge.trim().trim_end_matches('/')),
                true,
            )
        } else if let Ok(url) = env::var("AXIOM_URL") {
            (url, false)
        } else if let Ok(domain) = env::var("AXIOM_DOMAIN") {
            (
                format!(
                    "https://{}",
                    domain
                        .trim()
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .trim_end_matches('/')
                ),
                false,
            )
        } else {
            ("https://api.axiom.co".to_string(), false)
        };
        let ingest_url = match axiom_ingest_url(&base_url, &dataset, edge) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!(%error, "failed to build Axiom ingest URL");
                return None;
            }
        };
        let mut headers = reqwest::header::HeaderMap::new();
        let mut authorization =
            match reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(%error, "failed to initialize Axiom authorization");
                    return None;
                }
            };
        authorization.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        let client = match reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "failed to initialize Axiom logger");
                return None;
            }
        };

        Some(Self { client, ingest_url })
    }

    fn log_search(&self, event: SearchLogEvent) {
        let logger = self.clone();
        tokio::spawn(async move {
            if let Err(error) = logger.ingest(event).await {
                tracing::warn!(%error, "failed to ingest search query log to Axiom");
            }
        });
    }

    async fn ingest(&self, event: SearchLogEvent) -> Result<(), reqwest::Error> {
        self.client
            .post(self.ingest_url.clone())
            .json(&[event])
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map(|_| ())
    }
}

fn axiom_ingest_url(base_url: &str, dataset: &str, edge: bool) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url).map_err(|error| error.to_string())?;
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| "Axiom URL cannot be used as a base URL".to_string())?;
    segments.pop_if_empty();
    if edge {
        segments.extend(["v1", "ingest", dataset]);
    } else {
        segments.extend(["v1", "datasets", dataset, "ingest"]);
    }
    drop(segments);
    Ok(url)
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
            Self::EmptyQuery | Self::InvalidHandle | Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::MissingEnv(_)
            | Self::InvalidBindAddress(_)
            | Self::InvalidSearchBackend(_)
            | Self::Queue(_)
            | Self::Server(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::SearchBackend(_) | Self::UnexpectedSearchShape | Self::ChannelImage(_) => {
                StatusCode::BAD_GATEWAY
            }
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
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}
