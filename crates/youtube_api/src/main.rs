use axiom_rs::Client as AxiomClient;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    env,
    net::{AddrParseError, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::sync::Mutex;
use tower_http::{compression::CompressionLayer, cors::CorsLayer, trace::TraceLayer};
use youtube_meilisearch::{MeilisearchSearchClient, MeilisearchSearchConfig};
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
const MAX_HITS_PER_VIDEO: usize = 5;
const CACHE_YOUTUBERS: &str = "public, max-age=30, stale-while-revalidate=300";
const CACHE_SEARCH: &str = "public, max-age=30, stale-while-revalidate=300";
const CACHE_CHANNEL_IMAGE: &str = "public, max-age=86400, stale-while-revalidate=604800";
const CACHE_NO_STORE: &str = "no-store";
const DEFAULT_SEARCH_RATE_LIMIT_PER_MINUTE: usize = 120;

#[derive(Clone)]
struct AppState {
    search: Arc<dyn TranscriptSearchBackend>,
    search_backend: String,
    axiom: Option<AxiomLogger>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    search_rate_limit_per_minute: usize,
    index_queue: IndexQueue,
    image_http: reqwest::Client,
}

#[derive(Clone, Debug)]
struct IndexQueue {
    path: PathBuf,
    token: Option<String>,
}

#[derive(Clone, Debug)]
struct AxiomLogger {
    client: AxiomClient,
    dataset: String,
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

#[derive(Debug, Serialize)]
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
    content_kind: ContentKind,
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
    title_highlight: Option<String>,
    youtuber_handle: String,
    youtuber_name: String,
    thumbnail: Option<String>,
    published_at: Option<String>,
    content_kind: ContentKind,
    match_count: usize,
    youtube_url: String,
    embed_url: String,
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
    let state = Arc::new(AppState {
        search: search_client,
        search_backend: backend,
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
        .route("/api/index-requests/{id}", delete(ack_index_request))
        .route(
            "/api/index-requests/{id}/status",
            axum::routing::patch(update_index_request_status),
        )
        .layer(CorsLayer::permissive())
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

    let per_page = params
        .per_page
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(1, MAX_PER_PAGE);
    let page = params.page.unwrap_or(1).max(1);
    let mut sort = params.sort.unwrap_or_default();
    let low_signal_query = is_low_signal_query(query);
    if sort == SearchSort::Relevance && low_signal_query {
        sort = SearchSort::PublishedDesc;
    }
    let search_started = Instant::now();
    // Fetch raw transcript hits so a single video can return multiple matching
    // moments. The API groups those hits into video cards after retrieval.
    let use_backend_grouping = false;
    let search_page_size = if use_backend_grouping {
        per_page.saturating_add(1).min(MAX_PER_PAGE)
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
    let backend_query = if low_signal_query {
        if state.search_backend.eq_ignore_ascii_case("meilisearch") {
            ""
        } else {
            "*"
        }
    } else {
        query
    };
    let mut search_query = TextSearchQuery::new(backend_query)
        .query_by("text,title")
        .highlight_fields("text,title")
        .per_page(search_page_size)
        .page(search_backend_page);
    search_query = match sort {
        SearchSort::PublishedDesc if state.search_backend.eq_ignore_ascii_case("meilisearch") => {
            search_query.sort_by("published_at:desc")
        }
        SearchSort::PublishedAsc if state.search_backend.eq_ignore_ascii_case("meilisearch") => {
            search_query.sort_by("published_at:asc")
        }
        SearchSort::PublishedDesc => search_query.sort_by("published_at_timestamp:desc"),
        SearchSort::PublishedAsc => search_query.sort_by("published_at_timestamp:asc"),
        SearchSort::Relevance => search_query,
    };
    if use_backend_grouping {
        search_query = search_query.group_by("video_id").group_limit(1);
    } else {
        search_query = search_query.without_grouping();
    }
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

    if let Some(youtuber) = params
        .youtuber
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        search_query = search_query.youtuber_handle(normalize_handle(youtuber));
    }

    let first_search = async {
        let first_backend_started = Instant::now();
        let raw = state.search.search(&search_query).await?;
        Ok::<_, ApiError>((raw, first_backend_started.elapsed().as_millis() as u64))
    };
    let ((first_raw, first_backend_search_ms), total_videos) = tokio::try_join!(
        first_search,
        grouped_total_count(state.search.as_ref(), &search_query)
    )?;
    let found = first_raw
        .get("estimatedTotalHits")
        .or_else(|| first_raw.get("totalHits"))
        .or_else(|| first_raw.get("found"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let out_of = first_raw
        .get("estimatedTotalHits")
        .or_else(|| first_raw.get("totalHits"))
        .or_else(|| first_raw.get("out_of"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let processing_started = Instant::now();
    let page_result = collect_visible_videos(
        state.search.as_ref(),
        &search_query,
        first_raw,
        search_backend_page,
        local_offset,
        per_page,
        search_page_size,
        sort,
        query,
        first_backend_search_ms,
    )
    .await?;
    let result_processing_ms = processing_started.elapsed().as_millis() as u64;
    let videos = page_result.videos;
    let hits = videos
        .iter()
        .flat_map(|video| video.hits.iter().cloned())
        .collect::<Vec<_>>();
    let response = SearchResponse {
        query: query.to_string(),
        total_moments: found.max(hits.len() as u64),
        total_videos: total_videos.max(videos.len() as u64),
        total_found: found.max(hits.len() as u64),
        out_of,
        time_to_first_result_ms: first_backend_search_ms,
        page,
        per_page,
        sort,
        has_next_page: page_result.has_next_page,
        hits,
        videos,
    };
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
            status: "queued".to_string(),
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
    let request = state
        .index_queue
        .list(usize::MAX)
        .await?
        .into_iter()
        .find(|request| {
            matches!(request.status.as_str(), "running" | "queued")
                && handle
                    .as_ref()
                    .is_none_or(|handle| request.handle == *handle)
        });
    Ok(cache_response(
        CACHE_NO_STORE,
        Json(serde_json::json!({
            "handle": request.as_ref().map(|item| item.handle.as_str()).unwrap_or(""),
            "status": request.as_ref().map(|item| item.status.as_str()).unwrap_or("idle"),
            "request_id": request.as_ref().map(|item| item.id.clone()),
            "target_backend": request.and_then(|item| item.target_backend),
        })),
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
    if !matches!(input.status.as_str(), "queued" | "running") {
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
            "meilisearch" | "meili" => "meilisearch",
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
    async fn enqueue(&self, request: IndexQueueItem) -> Result<IndexQueueItem, ApiError> {
        if let Some(existing) = self.list(usize::MAX).await?.into_iter().find(|queued| {
            queued.handle == request.handle
                && queued.status == "queued"
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
    first_backend_search_ms: u64,
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

    for fetch_index in 0..MAX_SEARCH_PAGE_FETCHES {
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
        let videos = video_results_from_hits(&combined, sort, query);

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

        if fetch_index + 1 == MAX_SEARCH_PAGE_FETCHES {
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
        video_results_from_hits(&combined, sort, query),
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

fn video_results_from_hits(
    hits: &[SearchHit],
    sort: SearchSort,
    query: &str,
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
            video.match_count = video.hits.len();
            match sort {
                SearchSort::Relevance => video.hits.sort_by(|left, right| {
                    hit_relevance_score(right, &terms)
                        .cmp(&hit_relevance_score(left, &terms))
                        .then_with(|| left.t_start_ms.cmp(&right.t_start_ms))
                }),
                SearchSort::PublishedDesc | SearchSort::PublishedAsc => {
                    video.hits.sort_by_key(|hit| hit.t_start_ms);
                }
            }
            video.hits.truncate(MAX_HITS_PER_VIDEO);
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
        SearchSort::Relevance => videos.sort_by(|left, right| {
            video_relevance_score(right, &terms)
                .cmp(&video_relevance_score(left, &terms))
                .then_with(|| right.match_count.cmp(&left.match_count))
                .then_with(|| right.published_at.cmp(&left.published_at))
                .then_with(|| left.title.cmp(&right.title))
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
mod tests {
    use super::*;
    use serde_json::json;

    fn test_document(id: &str, video_id: &str, text: &str) -> Value {
        json!({
            "id": id,
            "video_id": video_id,
            "title": "Test video",
            "text": text,
            "youtuber_handle": "@test",
            "youtuber_name": "Test",
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
    }

    #[test]
    fn one_character_queries_are_low_signal() {
        assert!(is_low_signal_query("I"));
        assert!(is_low_signal_query("?"));
        assert!(!is_low_signal_query("fable"));
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
        );

        assert_eq!(
            videos.first().map(|video| video.video_id.as_str()),
            Some("exploit")
        );
    }
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

fn collection_env(backend: &str) -> String {
    match backend {
        "typesense" => env::var("TYPESENSE_COLLECTION"),
        _ => env::var("MEILI_TRANSCRIPTS_INDEX").or_else(|_| env::var("TYPESENSE_COLLECTION")),
    }
    .unwrap_or_else(|_| DEFAULT_COLLECTION.to_string())
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
        "meilisearch" | "meili" => {
            let api_url = required_env("MEILI_HTTP_ADDR")?;
            let search_api_key = required_env("MEILI_SEARCH_API_KEY")?;
            Ok(Arc::new(MeilisearchSearchClient::new(
                MeilisearchSearchConfig::new(api_url, search_api_key).with_collection(collection),
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
        let mut builder = AxiomClient::builder().with_token(token);

        if let Ok(edge_url) = env::var("AXIOM_EDGE_URL") {
            builder = builder.with_edge_url(edge_url);
        } else if let Ok(edge) = env::var("AXIOM_EDGE") {
            builder = builder.with_edge(edge);
        } else if let Ok(url) = env::var("AXIOM_URL") {
            builder = builder.with_url(url);
        } else if let Ok(domain) = env::var("AXIOM_DOMAIN") {
            builder = builder.with_url(format!(
                "https://{}",
                domain
                    .trim()
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
            ));
        }

        let client = match builder.build() {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!(%error, "failed to initialize Axiom logger");
                return None;
            }
        };

        Some(Self { client, dataset })
    }

    fn log_search(&self, event: SearchLogEvent) {
        let logger = self.clone();
        tokio::spawn(async move {
            if let Err(error) = logger.ingest(event).await {
                tracing::warn!(%error, "failed to ingest search query log to Axiom");
            }
        });
    }

    async fn ingest(&self, event: SearchLogEvent) -> Result<(), axiom_rs::Error> {
        self.client
            .ingest(self.dataset.as_str(), [event])
            .await
            .map(|_| ())
    }
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
        if let Some(seconds) = retry_after {
            if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}
