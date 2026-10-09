mod axiom;
mod channel_images;
mod index_queue;
mod rate_limit;
mod websub;

use axiom::AxiomLogger;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use channel_images::{
    ChannelImageCache, is_allowed_youtube_image_url, is_valid_channel_id, resized_youtube_image_url,
};
use index_queue::{IndexActivity, IndexQueue, IndexQueueItem, new_index_request_id};
use rate_limit::RateLimiter;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
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
use websub::WebSubConfig;
use youtube_search_client::{
    ChannelDocument, DEFAULT_COLLECTION, SearchClient, SearchClientConfig, SearchError,
    TextSearchQuery,
};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_PER_PAGE: u8 = 20;
const MAX_PER_PAGE: u8 = 100;
const MERGE_TRANSCRIPT_HIT_WINDOW_MS: u32 = 20_000;
const VIDEO_SCOPE_PAGE_SIZE: u8 = 250;
const VIDEO_SCOPE_MAX_PAGE_FETCHES: u32 = 100;
const INITIAL_GROUP_LIMIT: u8 = 3;
const CACHE_YOUTUBERS: &str = "public, max-age=30, stale-while-revalidate=300";
const CACHE_CHANNEL_IMAGE: &str = "public, max-age=86400, stale-while-revalidate=604800";
const CACHE_NO_STORE: &str = "no-store";
const DEFAULT_SEARCH_RATE_LIMIT_PER_MINUTE: usize = 120;
const DEFAULT_INDEX_REQUEST_RATE_LIMIT_PER_MINUTE: usize = 6;
const DEFAULT_INDEX_QUEUE_MAX_ITEMS: usize = 10_000;
const MAX_INDEX_REQUEST_BODY_BYTES: usize = 8 * 1024;
const MAX_HANDLE_BYTES: usize = 64;
const MAX_NOTE_CHARS: usize = 500;

#[derive(Clone)]
struct AppState {
    search: SearchClient,
    backend_label: String,
    engagement_sorts_enabled: bool,
    relevance_ordering: RelevanceOrdering,
    axiom: Option<AxiomLogger>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    search_rate_limit_per_minute: usize,
    index_request_rate_limit_per_minute: usize,
    index_queue: IndexQueue,
    image_http: reqwest::Client,
    channel_images: ChannelImageCache,
    websub: Option<WebSubConfig>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelevanceOrdering {
    Backend,
    ApiHeuristic,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    youtuber: Option<String>,
    video_id: Option<String>,
    per_page: Option<u8>,
    page: Option<u32>,
    sort: Option<SearchSort>,
}

#[derive(Debug, Serialize)]
struct YoutubersResponse {
    channels: Vec<ChannelDocument>,
}

#[derive(Debug, Deserialize)]
struct IndexRequestInput {
    handle: String,
    note: Option<String>,
    backends: Option<Vec<String>>,
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

#[derive(Debug, Deserialize)]
struct WebSubVerifyParams {
    #[serde(rename = "hub.mode")]
    mode: String,
    #[serde(rename = "hub.topic")]
    topic: String,
    #[serde(rename = "hub.challenge")]
    challenge: String,
}

#[derive(Debug, Serialize)]
struct IndexQueueListResponse {
    requests: Vec<IndexQueueItem>,
}

#[derive(Debug, Deserialize)]
struct IndexRequestStatusInput {
    status: String,
}

#[derive(Debug, Serialize)]
struct SearchResponse<'a> {
    query: &'a str,
    total_moments: u64,
    total_videos: u64,
    totals_approximate: bool,
    total_found: u64,
    out_of: u64,
    time_to_first_result_ms: u64,
    page: u32,
    per_page: u8,
    sort: SearchSort,
    mode: &'static str,
    has_next_page: bool,
    /// Always empty; moments live in `videos[].hits`. Frontends built before
    /// 2026-10-05 reject a response without this array, so it stays until the
    /// Worker serving the current frontend has been deployed and old tabs have
    /// aged out.
    hits: [SearchHit; 0],
    videos: &'a [SearchVideoResult],
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

#[derive(Clone, Debug, Serialize)]
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
    #[serde(skip)]
    group_match_count: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum MatchKind {
    Transcript,
    Title,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
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
    #[error("index request queue is at capacity")]
    QueueFull,
    #[error("channel image failed: {0}")]
    ChannelImage(String),
    #[error("search backend failed: {0}")]
    SearchBackend(#[from] SearchError),
    #[error("search response had an unexpected shape")]
    UnexpectedSearchShape,
    #[error("server failed: {0}")]
    Server(#[from] std::io::Error),
}

#[tokio::main]
async fn main() -> Result<(), ApiError> {
    let _ = dotenvy::dotenv();
    init_tracing();

    let collection =
        env::var("RAW_SEARCH_COLLECTION").unwrap_or_else(|_| DEFAULT_COLLECTION.to_string());
    let bind_addr = env::var("YTSEARCH_API_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let bind_addr: SocketAddr = bind_addr
        .parse()
        .map_err(|error: AddrParseError| ApiError::InvalidBindAddress(error.to_string()))?;

    let search_client = raw_search_client(collection)?;
    let allowed_origins = allowed_browser_origins();
    let (axiom, axiom_worker) = AxiomLogger::from_env()
        .map(|(logger, worker)| (Some(logger), Some(worker)))
        .unwrap_or((None, None));
    let state = Arc::new(AppState {
        search: search_client,
        backend_label: env::var("YTSEARCH_BACKEND_LABEL").unwrap_or_else(|_| "raw".to_string()),
        engagement_sorts_enabled: env_bool("YTSEARCH_ENGAGEMENT_SORTS", false),
        relevance_ordering: if env_bool("YTSEARCH_PRESERVE_BACKEND_RELEVANCE_ORDER", false) {
            RelevanceOrdering::Backend
        } else {
            RelevanceOrdering::ApiHeuristic
        },
        axiom,
        rate_limiter: Arc::new(Mutex::new(RateLimiter::default())),
        search_rate_limit_per_minute: env_usize(
            "YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE",
            DEFAULT_SEARCH_RATE_LIMIT_PER_MINUTE,
        ),
        index_request_rate_limit_per_minute: env_usize(
            "YTSEARCH_INDEX_REQUEST_RATE_LIMIT_PER_MINUTE",
            DEFAULT_INDEX_REQUEST_RATE_LIMIT_PER_MINUTE,
        ),
        index_queue: IndexQueue::new(
            env::var("YTSEARCH_INDEX_QUEUE_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data/index-requests.jsonl")),
            env::var("YTSEARCH_INDEX_QUEUE_TOKEN").ok(),
            env_usize(
                "YTSEARCH_INDEX_QUEUE_MAX_ITEMS",
                DEFAULT_INDEX_QUEUE_MAX_ITEMS,
            ),
        ),
        image_http: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            // The first URL is checked against YouTube's image hosts; every
            // redirect must be too, or a redirect could reach any host.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 3 {
                    attempt.error("too many image redirects")
                } else if is_allowed_youtube_image_url(attempt.url().as_str()) {
                    attempt.follow()
                } else {
                    attempt.error("image redirect to a disallowed host")
                }
            }))
            .build()
            .map_err(|error| ApiError::ChannelImage(format!("client setup failed: {error}")))?,
        channel_images: ChannelImageCache::default(),
        websub: WebSubConfig::from_env(),
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
            post(create_index_request)
                .layer(DefaultBodyLimit::max(MAX_INDEX_REQUEST_BODY_BYTES))
                .get(list_index_requests),
        )
        .route(
            "/api/websub/youtube",
            get(websub_verify)
                .post(websub_notify)
                .layer(DefaultBodyLimit::max(websub::MAX_NOTIFICATION_BYTES)),
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
        .with_state(state.clone());

    tracing::info!(%bind_addr, "starting ytsearch API");
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    if let Some(config) = state.websub.clone() {
        tokio::spawn(renew_websub_subscriptions(state.search.clone(), config));
    }
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    if let Some(worker) = axiom_worker {
        worker.shutdown().await;
    }
    result?;

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
    no_store_json(json!({ "ok": true }))
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
    state.channel_images.replace(&channels).await;
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
        .channel_images
        .get_or_refresh(&channel_id, &state.search)
        .await?;
    let avatar_url = resized_youtube_image_url(&avatar_url, 96);

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
        .filter(|value| {
            value
                .to_str()
                .is_ok_and(|value| value.starts_with("image/"))
        })
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("image/jpeg"));
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

    Ok(cache_response(
        CACHE_CHANNEL_IMAGE,
        ([(header::CONTENT_TYPE, content_type)], bytes),
    ))
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<SearchParams>,
) -> Result<Response, ApiError> {
    rate_limit(
        &state,
        &headers,
        "search",
        state.search_rate_limit_per_minute,
    )
    .await?;
    let query = params.q.trim();
    if query.is_empty() {
        return Err(ApiError::EmptyQuery);
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
    // A video-scoped request loads every matching moment of one video. Any
    // other request lets the backend paginate video cards, so long transcripts
    // cannot crowd other videos out of the result window.
    let video_scope = params.video_id.is_some();
    let requested_offset = (page - 1).saturating_mul(u32::from(per_page));
    let (search_backend_page, search_page_size) = if video_scope {
        (1, VIDEO_SCOPE_PAGE_SIZE)
    } else {
        (page, per_page.saturating_add(1))
    };
    let backend_query = if low_signal_query { "*" } else { query };
    let mut search_query = TextSearchQuery::new(backend_query)
        .query_by("text,title")
        .highlight_fields("text,title");
    search_query = if video_scope {
        search_query
            .per_page(search_page_size)
            .page(search_backend_page)
            .without_grouping()
    } else {
        // Cards render three moments and load the rest on demand; Raw's group
        // `found` preserves the true count.
        search_query
            .window(requested_offset, search_page_size)
            .group_limit(INITIAL_GROUP_LIMIT)
    };
    search_query = match sort {
        SearchSort::PublishedDesc => search_query.sort_by("published_at_timestamp:desc"),
        SearchSort::PublishedAsc => search_query.sort_by("published_at_timestamp:asc"),
        SearchSort::RelevanceEngagement => search_query.sort_by("engagement_score:desc"),
        SearchSort::ViewsDesc => search_query.sort_by("view_count:desc"),
        SearchSort::LikesDesc => search_query.sort_by("like_count:desc"),
        SearchSort::CommentsDesc => search_query.sort_by("comment_count:desc"),
        SearchSort::Relevance => search_query,
    };
    tracing::info!(
        query,
        per_page,
        page,
        ?sort,
        search_page = search_backend_page,
        search_page_size,
        youtuber = params.youtuber.as_deref().unwrap_or(""),
        "search request"
    );

    if let Some(youtuber) = non_empty(params.youtuber.as_deref()) {
        search_query = search_query.youtuber_handle(normalize_handle(youtuber));
    }
    if let Some(video_id) = non_empty(params.video_id.as_deref()) {
        if !video_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        {
            return Err(ApiError::BadRequest("invalid video id".to_string()));
        }
        search_query = search_query.filter_by(format!("video_id:={video_id}"));
    }

    let first_backend_started = Instant::now();
    let mut first_raw = state.search.search(&search_query).await?;
    let first_backend_search_ms = first_backend_started.elapsed().as_millis() as u64;
    let backend_found = raw_u64(&first_raw, "found").unwrap_or_default();
    let backend_total_moments = raw_u64(&first_raw, "total_moments");
    let out_of = raw_u64(&first_raw, "out_of").unwrap_or_default();
    let total_videos = if video_scope {
        u64::from(backend_found > 0)
    } else {
        backend_found
    };
    let grouped_has_next_page =
        !video_scope && truncate_grouped_lookahead(&mut first_raw, per_page);
    let processing_started = Instant::now();
    let mut timings = SearchCollectionTimings {
        backend_search_calls: 1,
        backend_search_ms: first_backend_search_ms,
        ..SearchCollectionTimings::default()
    };
    let (local_offset, max_page_fetches) = if video_scope {
        (requested_offset as usize, VIDEO_SCOPE_MAX_PAGE_FETCHES)
    } else {
        (0, 1)
    };
    let hits = collect_hits(
        &state.search,
        &search_query,
        first_raw,
        search_page_size,
        max_page_fetches,
        local_offset + usize::from(per_page) + 1,
        &mut timings,
    )
    .await?;
    let videos = video_results_from_hits_with_policy(hits, sort, query, state.relevance_ordering);
    let (videos, mut has_next_page) = page_of_videos(videos, local_offset, per_page);
    if !video_scope {
        has_next_page = grouped_has_next_page;
    }
    let result_processing_ms = processing_started.elapsed().as_millis() as u64;
    let returned_hits = videos.iter().map(|video| video.hits.len()).sum::<usize>();
    let total_moments = backend_total_moments
        .unwrap_or(if video_scope {
            backend_found
        } else {
            returned_hits as u64
        })
        .max(returned_hits as u64);
    let response = SearchResponse {
        query,
        total_moments,
        total_videos: total_videos.max(videos.len() as u64),
        totals_approximate: !video_scope && backend_total_moments.is_none(),
        total_found: total_moments,
        out_of,
        time_to_first_result_ms: first_backend_search_ms,
        page,
        per_page,
        sort,
        mode: "lexical",
        has_next_page,
        hits: [],
        videos: &videos,
    };
    let body = serde_json::to_vec(&response).map_err(std::io::Error::from)?;
    tracing::info!(
        query,
        found = backend_found,
        returned_videos = response.videos.len(),
        returned_hits,
        backend_search_ms = timings.backend_search_ms,
        first_backend_search_ms,
        result_processing_ms,
        response_body_bytes = body.len(),
        "search response"
    );
    if let Some(axiom) = &state.axiom {
        axiom.log_search(SearchLogEvent {
            event: "search_query",
            backend: state.backend_label.clone(),
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
            returned_hits,
            has_next_page,
            duration_ms: search_started.elapsed().as_millis() as u64,
            backend_search_ms: timings.backend_search_ms,
            first_backend_search_ms,
            result_processing_ms,
            backend_search_calls: timings.backend_search_calls,
            backend_raw_hits: timings.backend_raw_hits,
            backend_grouped_hits: timings.backend_grouped_hits,
            response_body_bytes: body.len(),
            cache_control: CACHE_NO_STORE,
        });
    }

    Ok(cache_response(
        CACHE_NO_STORE,
        ([(header::CONTENT_TYPE, "application/json")], body),
    ))
}

async fn create_index_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(input): Json<IndexRequestInput>,
) -> Result<Response, ApiError> {
    rate_limit(
        &state,
        &headers,
        "index-request",
        state.index_request_rate_limit_per_minute,
    )
    .await?;
    let handle = normalize_handle_checked(&input.handle)?;
    let note = normalize_index_request_note(input.note)?;
    validate_target_backends(input.backends.as_deref())?;
    let request = state
        .index_queue
        .enqueue(IndexQueueItem {
            id: new_index_request_id("raw", &handle),
            handle,
            note,
            requested_at: epoch_seconds(),
            source_backend: "raw".to_string(),
            target_backend: Some("raw".to_string()),
            status: "pending".to_string(),
        })
        .await?;
    Ok(cache_response(
        CACHE_NO_STORE,
        (
            StatusCode::ACCEPTED,
            Json(IndexQueueResponse {
                requests: vec![request.clone()],
                request,
            }),
        ),
    ))
}

/// Hub verification: confirm a subscription only for an indexed creator, and an
/// unsubscription only for a creator no longer indexed, so a third party
/// cannot cancel the subscriptions this API maintains.
async fn websub_verify(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WebSubVerifyParams>,
) -> Result<Response, ApiError> {
    if state.websub.is_none() || params.challenge.is_empty() || params.challenge.len() > 512 {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let Some(channel_id) = websub::channel_id_from_topic(&params.topic) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let indexed = indexed_channel(&state, channel_id).await?.is_some();
    let confirmed = match params.mode.as_str() {
        "subscribe" => indexed,
        "unsubscribe" => !indexed,
        _ => false,
    };
    if !confirmed {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    Ok(cache_response(
        CACHE_NO_STORE,
        (
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            params.challenge,
        ),
    ))
}

/// A verified upload from an indexed creator queues one update request for
/// that creator; the queue collapses repeats until the worker runs it. Every
/// other push is acknowledged and ignored, as WebSub requires.
async fn websub_notify(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let Some(config) = &state.websub else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let accepted = || cache_response(CACHE_NO_STORE, StatusCode::ACCEPTED);
    let signature = header_string(&headers, "x-hub-signature");
    if !websub::signature_valid(&config.secret, &body, signature.as_deref()) {
        tracing::warn!("ignored WebSub notification with an invalid signature");
        return Ok(accepted());
    }
    let Some(notification) = std::str::from_utf8(&body)
        .ok()
        .and_then(websub::parse_notification)
    else {
        return Ok(accepted());
    };
    let Some(channel) = indexed_channel(&state, &notification.channel_id).await? else {
        return Ok(accepted());
    };
    let handle = normalize_handle(&channel.handle);
    let request = state
        .index_queue
        .enqueue(IndexQueueItem {
            id: format!("upload-{}", new_index_request_id("raw", &handle)),
            handle,
            note: Some(format!("YouTube upload {}", notification.video_id)),
            requested_at: epoch_seconds(),
            source_backend: "raw".to_string(),
            target_backend: Some("raw".to_string()),
            status: "queued".to_string(),
        })
        .await?;
    tracing::info!(
        request_id = %request.id,
        handle = %request.handle,
        video_id = %notification.video_id,
        "queued creator update from a YouTube upload notification"
    );
    Ok(accepted())
}

async fn indexed_channel(
    state: &AppState,
    channel_id: &str,
) -> Result<Option<ChannelDocument>, ApiError> {
    Ok(state
        .search
        .list_youtubers()
        .await?
        .into_iter()
        .find(|channel| channel.channel_id == channel_id))
}

/// Renews every indexed channel's subscription once a day, sooner after a
/// failed channel listing.
async fn renew_websub_subscriptions(search: SearchClient, config: WebSubConfig) {
    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(http) => http,
        Err(error) => {
            tracing::error!(%error, "WebSub renewal disabled: client setup failed");
            return;
        }
    };
    loop {
        let wait = match search.list_youtubers().await {
            Ok(channels) => {
                let (mut renewed, mut failed) = (0_usize, 0_usize);
                for channel in channels
                    .iter()
                    .filter(|channel| is_valid_channel_id(&channel.channel_id))
                {
                    let response = http
                        .post(&config.hub_url)
                        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                        .body(websub::subscribe_form(&config, &channel.channel_id))
                        .send()
                        .await;
                    match response {
                        Ok(response) if response.status().is_success() => renewed += 1,
                        Ok(response) => {
                            failed += 1;
                            tracing::warn!(channel_id = %channel.channel_id, status = %response.status(), "WebSub subscription refused");
                        }
                        Err(error) => {
                            failed += 1;
                            tracing::warn!(channel_id = %channel.channel_id, %error, "WebSub subscription failed");
                        }
                    }
                }
                tracing::info!(renewed, failed, "renewed WebSub subscriptions");
                websub::RENEW_INTERVAL
            }
            Err(error) => {
                tracing::warn!(%error, "WebSub renewal could not list channels");
                Duration::from_secs(60 * 60)
            }
        };
        tokio::time::sleep(wait).await;
    }
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
    Ok(no_store_json(IndexQueueListResponse { requests }))
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
    let wanted = |candidate: &str| handle.as_deref().is_none_or(|handle| candidate == handle);
    let (status_handle, status, request_id, target_backend) = if let Some(activity) = state
        .index_queue
        .activity()
        .await?
        .filter(|activity| wanted(&activity.handle))
    {
        (
            activity.handle,
            "running".to_string(),
            Some(format!("active-{}", activity.started_at)),
            Some(activity.target_backend),
        )
    } else if let Some(request) =
        state
            .index_queue
            .list(usize::MAX)
            .await?
            .into_iter()
            .find(|request| {
                (matches!(request.status.as_str(), "running" | "queued")
                    || (handle.is_some()
                        && matches!(request.status.as_str(), "pending" | "failed" | "rejected")))
                    && wanted(&request.handle)
            })
    {
        (
            request.handle,
            request.status,
            Some(request.id),
            request.target_backend,
        )
    } else {
        (String::new(), "idle".to_string(), None, None)
    };
    let channel = if status_handle.is_empty() {
        None
    } else {
        state
            .search
            .list_youtubers()
            .await?
            .into_iter()
            .find(|channel| channel.handle.eq_ignore_ascii_case(&status_handle))
    };
    Ok(no_store_json(json!({
        "handle": status_handle,
        "status": status,
        "request_id": request_id,
        "target_backend": target_backend,
        "channel": channel,
    })))
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
            target_backend: input.target_backend.unwrap_or_else(|| "raw".to_string()),
        }),
        None => None,
    };
    state.index_queue.set_activity(activity).await?;
    Ok(no_store_json(json!({ "ok": true })))
}

async fn ack_index_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    state.index_queue.authorize(&headers)?;
    state.index_queue.ack(&id).await?;
    Ok(no_store_json(json!({ "ok": true })))
}

async fn approve_index_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    transition_index_request(
        &state,
        &headers,
        &id,
        "queued",
        &["pending", "rejected", "failed"],
    )
    .await
}

async fn reject_index_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    transition_index_request(&state, &headers, &id, "rejected", &["pending"]).await
}

async fn transition_index_request(
    state: &AppState,
    headers: &HeaderMap,
    id: &str,
    status: &str,
    allowed_statuses: &[&str],
) -> Result<Response, ApiError> {
    state.index_queue.authorize(headers)?;
    let request = state
        .index_queue
        .set_status_from(id, status, allowed_statuses)
        .await?;
    Ok(no_store_json(json!({ "request": request })))
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
    Ok(no_store_json(json!({ "request": request })))
}

fn is_valid_index_request_status(status: &str) -> bool {
    matches!(
        status,
        "pending" | "queued" | "running" | "failed" | "rejected"
    )
}

fn is_searchable_channel(channel: &ChannelDocument) -> bool {
    channel.indexed_video_count > 0 && channel.indexed_chunk_count > 0
}

/// `raw` is the only backend. Blank entries are ignored, but at least one
/// backend must remain.
fn validate_target_backends(input: Option<&[String]>) -> Result<(), ApiError> {
    let Some(backends) = input else {
        return Ok(());
    };
    let mut any = false;
    for backend in backends.iter().map(|backend| backend.trim()) {
        match backend {
            "" => {}
            "raw" => any = true,
            other => {
                return Err(ApiError::BadRequest(format!(
                    "unsupported target backend: {other}"
                )));
            }
        }
    }
    if !any {
        return Err(ApiError::BadRequest(
            "at least one target backend is required".to_string(),
        ));
    }
    Ok(())
}

fn normalize_index_request_note(note: Option<String>) -> Result<Option<String>, ApiError> {
    let note = note
        .map(|note| note.trim().to_string())
        .filter(|note| !note.is_empty());
    if note
        .as_ref()
        .is_some_and(|note| note.chars().count() > MAX_NOTE_CHARS)
    {
        return Err(ApiError::BadRequest(format!(
            "index request note cannot exceed {MAX_NOTE_CHARS} characters"
        )));
    }
    Ok(note)
}

#[derive(Clone, Copy, Debug, Default)]
struct SearchCollectionTimings {
    backend_search_calls: u32,
    backend_search_ms: u64,
    backend_raw_hits: usize,
    backend_grouped_hits: usize,
}

/// Parses `raw` and then fetches following backend pages until one comes
/// back short, `max_page_fetches` pages have been read, or the hits span
/// `required_videos` videos. Nearby moments are coalesced after every page,
/// so later pages merge into the kept moments.
async fn collect_hits(
    search: &SearchClient,
    query: &TextSearchQuery,
    mut raw: Value,
    page_size: u8,
    max_page_fetches: u32,
    required_videos: usize,
    timings: &mut SearchCollectionTimings,
) -> Result<Vec<SearchHit>, ApiError> {
    let mut combined = Vec::new();
    for page in 1..=max_page_fetches {
        let raw_hits = array_len(&raw, "hits");
        timings.backend_raw_hits += raw_hits;
        timings.backend_grouped_hits += array_len(&raw, "grouped_hits");
        let page_hits = parse_hits(&raw)?;
        let last_page = page_hits.is_empty() || raw_hits < usize::from(page_size);
        combined.extend(page_hits);
        combined = merge_nearby_transcript_hits(dedupe_title_only_hits(combined));
        if last_page
            || page == max_page_fetches
            || has_at_least_distinct_videos(&combined, required_videos)
        {
            break;
        }

        let backend_started = Instant::now();
        raw = search.search(&query.clone().page(page + 1)).await?;
        timings.backend_search_calls += 1;
        timings.backend_search_ms += backend_started.elapsed().as_millis() as u64;
    }
    Ok(combined)
}

fn has_at_least_distinct_videos(hits: &[SearchHit], required: usize) -> bool {
    let mut video_ids = HashSet::with_capacity(required.min(hits.len()));
    hits.iter()
        .any(|hit| video_ids.insert(hit.video_id.as_str()) && video_ids.len() >= required)
}

/// Returns one page of videos and whether any video follows it.
fn page_of_videos(
    videos: Vec<SearchVideoResult>,
    offset: usize,
    per_page: u8,
) -> (Vec<SearchVideoResult>, bool) {
    let per_page = usize::from(per_page);
    let has_next_page = videos.len() > offset.saturating_add(per_page);
    let page = videos.into_iter().skip(offset).take(per_page).collect();
    (page, has_next_page)
}

fn raw_u64(raw: &Value, field: &str) -> Option<u64> {
    raw.get(field).and_then(Value::as_u64)
}

fn array_len(raw: &Value, field: &str) -> usize {
    raw.get(field).and_then(Value::as_array).map_or(0, Vec::len)
}

fn video_results_from_hits_with_policy(
    hits: Vec<SearchHit>,
    sort: SearchSort,
    query: &str,
    relevance_ordering: RelevanceOrdering,
) -> Vec<SearchVideoResult> {
    let terms = searchable_terms(query);
    let joined_terms = terms.concat();
    let mut videos = Vec::<SearchVideoResult>::new();
    let mut by_video = HashMap::<String, usize>::new();

    for hit in hits {
        let index = *by_video.entry(hit.video_id.clone()).or_insert_with(|| {
            videos.push(SearchVideoResult::from_hit(&hit));
            videos.len() - 1
        });
        videos[index].hits.push(hit);
    }

    for video in &mut videos {
        if video
            .hits
            .iter()
            .any(|hit| hit.match_kind == MatchKind::Transcript)
        {
            video
                .hits
                .retain(|hit| hit.match_kind == MatchKind::Transcript);
        }
        video.match_count = video.match_count.max(video.hits.len());
        match sort {
            SearchSort::Relevance if relevance_ordering == RelevanceOrdering::Backend => {}
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
    }

    match sort {
        SearchSort::Relevance if relevance_ordering == RelevanceOrdering::ApiHeuristic => {
            videos.sort_by_cached_key(|video| {
                (
                    Reverse(video_relevance_score(video, &terms, &joined_terms)),
                    Reverse(video.match_count),
                    Reverse(video.published_at.clone()),
                    video.title.clone(),
                )
            });
        }
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
fn video_relevance_score(video: &SearchVideoResult, terms: &[String], joined_terms: &str) -> usize {
    if terms.is_empty() {
        return 0;
    }

    let title = video.title.to_ascii_lowercase();
    let title_compact = alphanumeric_compact(&title);
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
    if terms.len() > 1 && title_compact.contains(joined_terms) {
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
            match_count: hit.group_match_count.unwrap_or(0),
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
    let Some(groups) = raw.get("grouped_hits").and_then(Value::as_array) else {
        return raw
            .get("hits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|hit| SearchHit::from_backend_hit(hit, None))
            .collect();
    };
    let mut parsed = Vec::new();
    for group in groups {
        let group_match_count =
            raw_u64(group, "found").and_then(|found| usize::try_from(found).ok());
        for hit in group
            .get("hits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            parsed.push(SearchHit::from_backend_hit(hit, group_match_count)?);
        }
    }
    Ok(parsed)
}

fn truncate_grouped_lookahead(raw: &mut Value, per_page: u8) -> bool {
    let Some(groups) = raw.get_mut("grouped_hits").and_then(Value::as_array_mut) else {
        return false;
    };
    let per_page = usize::from(per_page);
    let has_next_page = groups.len() > per_page;
    groups.truncate(per_page);
    has_next_page
}

impl SearchHit {
    /// Parses one Raw hit: `{ "document": {...}, "highlights": [...] }`.
    fn from_backend_hit(hit: &Value, group_match_count: Option<usize>) -> Result<Self, ApiError> {
        let document = hit.get("document").unwrap_or(hit);
        let video_id = string_field(document, "video_id")?;
        let t_start_ms = u32_field(document, "t_start_ms")?;
        let text_highlight = highlight_for_field(hit, "text");
        let title_highlight = highlight_for_field(hit, "title");
        let match_kind = if title_highlight.is_some() && text_highlight.is_none() {
            MatchKind::Title
        } else {
            MatchKind::Transcript
        };
        let (timestamp, youtube_url, embed_url) = match match_kind {
            MatchKind::Transcript => {
                let seconds = t_start_ms / 1000;
                (
                    Some(format_timestamp(t_start_ms)),
                    format!("https://www.youtube.com/watch?v={video_id}&t={seconds}s"),
                    format!(
                        "https://www.youtube-nocookie.com/embed/{video_id}?start={seconds}&rel=0"
                    ),
                )
            }
            MatchKind::Title => (
                None,
                format!("https://www.youtube.com/watch?v={video_id}"),
                format!("https://www.youtube-nocookie.com/embed/{video_id}?rel=0"),
            ),
        };

        Ok(Self {
            id: string_field(document, "id")?,
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
            content_kind: content_kind_for_duration(optional_u32_field(document, "duration")),
            t_start_ms,
            t_end_ms: u32_field(document, "t_end_ms")?,
            timestamp,
            youtube_url,
            embed_url,
            video_id,
            group_match_count,
        })
    }
}

fn highlight_for_field(hit: &Value, field: &str) -> Option<String> {
    hit.get("highlights")?
        .as_array()?
        .iter()
        .find_map(|highlight| {
            if highlight.get("field")?.as_str()? != field {
                return None;
            }
            highlight.get("snippet")?.as_str().map(ToOwned::to_owned)
        })
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
        let mut best_ranked_index = None;
        for index in indices.iter().copied() {
            let start = hits[index].t_start_ms;
            if cluster_start.is_none_or(|cluster| {
                start.saturating_sub(cluster) > MERGE_TRANSCRIPT_HIT_WINDOW_MS
            }) {
                cluster_start = Some(start);
                best_ranked_index = Some(index);
            } else {
                let best = best_ranked_index.unwrap_or(index);
                if index < best {
                    keep[best] = false;
                    best_ranked_index = Some(index);
                } else {
                    keep[index] = false;
                }
            }
        }
    }

    hits.into_iter()
        .enumerate()
        .filter_map(|(index, hit)| keep[index].then_some(hit))
        .collect()
}

fn dedupe_title_only_hits(hits: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen_title_videos = HashSet::new();
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
        || normalized.len() > MAX_HANDLE_BYTES
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

async fn rate_limit(
    state: &AppState,
    headers: &HeaderMap,
    scope: &str,
    per_minute: usize,
) -> Result<(), ApiError> {
    if per_minute > 0 {
        state.rate_limiter.lock().await.allow(
            format!("{scope}:{}", client_key(headers)),
            per_minute,
            Duration::from_secs(60),
        )?;
    }
    Ok(())
}

fn client_key(headers: &HeaderMap) -> &str {
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
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
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

fn no_store_json(value: impl Serialize) -> Response {
    cache_response(CACHE_NO_STORE, Json(value))
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

fn raw_search_client(collection: String) -> Result<SearchClient, ApiError> {
    let api_url = required_env("RAW_SEARCH_API_URL")?;
    let search_api_key = required_env("RAW_SEARCH_API_KEY")?;
    Ok(SearchClient::new(
        SearchClientConfig::new(api_url, search_api_key).with_collection(collection),
    )?)
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
            Self::EmptyQuery | Self::InvalidHandle | Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::QueueFull => StatusCode::SERVICE_UNAVAILABLE,
            Self::MissingEnv(_)
            | Self::InvalidBindAddress(_)
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
        let mut response = no_store_json(json!({ "error": self.to_string() }));
        *response.status_mut() = status;
        if let Some(seconds) = retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        response
    }
}

#[cfg(test)]
mod tests;
