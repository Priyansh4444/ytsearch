use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::sync::Semaphore;

const TRANSCRIPT_COLLECTION: &str = "youtube_transcript_chunks";
const CHANNEL_COLLECTION: &str = "youtube_youtubers";
const MAX_CACHE_TTL_SECONDS: u64 = 300;
const MAX_CONCURRENT_BACKEND_SEARCHES: usize = 4;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    pub q: String,
    #[serde(default)]
    pub query_by: Option<String>,
    #[serde(default)]
    pub query_by_weights: Option<String>,
    #[serde(default)]
    pub filter_by: Option<String>,
    #[serde(default)]
    pub group_by: Option<String>,
    #[serde(default)]
    pub group_limit: Option<u8>,
    #[serde(default)]
    pub per_page: Option<u8>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u8>,
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub text_match_type: Option<String>,
    #[serde(default)]
    pub prioritize_exact_match: Option<bool>,
    #[serde(default)]
    pub prioritize_token_position: Option<bool>,
    #[serde(default)]
    pub phrase_slop: Option<u8>,
    #[serde(default)]
    pub typo_tokens_threshold: Option<u32>,
    #[serde(default)]
    pub drop_tokens_threshold: Option<u32>,
    #[serde(default)]
    pub drop_tokens_mode: Option<String>,
    #[serde(default)]
    pub split_join_tokens: Option<String>,
    #[serde(default)]
    pub highlight_fields: Option<String>,
    #[serde(default)]
    pub include_fields: Option<String>,
    #[serde(default)]
    pub use_cache: Option<bool>,
    #[serde(default)]
    pub cache_ttl: Option<u64>,
}

#[derive(Debug)]
pub enum BackendError {
    InvalidInput(String),
    Internal(String),
}

pub trait SearchBackend: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn search(&self, params: &SearchParams) -> Result<Value, BackendError>;
    fn search_channels(&self, params: &SearchParams) -> Result<Value, BackendError>;
    fn documents(&self) -> u64;
    fn index_bytes(&self) -> u64;

    fn cache_generation(&self) -> u64 {
        0
    }
}

pub struct ServerConfig {
    pub bind: SocketAddr,
    pub api_key: Option<String>,
    pub raw_route: bool,
}

#[derive(Clone)]
struct AppState {
    backend: Arc<dyn SearchBackend>,
    metrics: Arc<Metrics>,
    started: Instant,
    api_key: Option<Arc<str>>,
    cache: Arc<Mutex<SearchCache>>,
    search_permits: Arc<Semaphore>,
}

#[derive(Default)]
struct Metrics {
    searches: AtomicU64,
    errors: AtomicU64,
    search_nanos: AtomicU64,
    cache_hits: AtomicU64,
}

#[derive(Default)]
struct SearchCache {
    entries: VecDeque<CacheEntry>,
}

struct CacheEntry {
    key: String,
    expires: Instant,
    value: Bytes,
}

#[derive(Serialize)]
struct MetricsResponse {
    backend: &'static str,
    documents: u64,
    index_bytes: u64,
    rss_bytes: u64,
    cpu_ticks: u64,
    searches: u64,
    errors: u64,
    search_nanos: u64,
    cache_hits: u64,
    uptime_seconds: u64,
}

pub async fn serve(config: ServerConfig, backend: Arc<dyn SearchBackend>) -> std::io::Result<()> {
    let bind = config.bind;
    let backend_name = backend.name();
    let app = router(&config, backend);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(bind = %bind, backend = backend_name, "search experiment ready");
    serve_router(listener, app).await
}

pub fn router(config: &ServerConfig, backend: Arc<dyn SearchBackend>) -> Router {
    let state = AppState {
        backend,
        metrics: Arc::new(Metrics::default()),
        started: Instant::now(),
        api_key: config.api_key.as_deref().map(Arc::from),
        cache: Arc::new(Mutex::new(SearchCache::default())),
        search_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_BACKEND_SEARCHES)),
    };
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/collections/{collection}/documents/search", get(search));
    if config.raw_route {
        app = app.route(
            "/raw/collections/{collection}/documents/search",
            get(search),
        );
    }
    app.with_state(state)
}

pub async fn serve_router(listener: tokio::net::TcpListener, app: Router) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "backend": state.backend.name(),
        "documents": state.backend.documents(),
    }))
}

async fn metrics(State(state): State<AppState>) -> Json<MetricsResponse> {
    let (rss_bytes, cpu_ticks) = process_usage();
    Json(MetricsResponse {
        backend: state.backend.name(),
        documents: state.backend.documents(),
        index_bytes: state.backend.index_bytes(),
        rss_bytes,
        cpu_ticks,
        searches: state.metrics.searches.load(Ordering::Relaxed),
        errors: state.metrics.errors.load(Ordering::Relaxed),
        search_nanos: state.metrics.search_nanos.load(Ordering::Relaxed),
        cache_hits: state.metrics.cache_hits.load(Ordering::Relaxed),
        uptime_seconds: state.started.elapsed().as_secs(),
    })
}

async fn search(
    State(state): State<AppState>,
    Path(collection): Path<String>,
    Query(params): Query<SearchParams>,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    let unauthorized = state.api_key.as_deref().is_some_and(|expected| {
        headers
            .get("x-raw-search-key")
            .and_then(|value| value.to_str().ok())
            != Some(expected)
    });
    if unauthorized {
        return Err(ServerError::Unauthorized);
    }

    let started = Instant::now();
    state.metrics.searches.fetch_add(1, Ordering::Relaxed);
    if let Err(error) = validate_cache_params(&params) {
        state.metrics.errors.fetch_add(1, Ordering::Relaxed);
        record_elapsed(&state, started);
        return Err(ServerError::Backend(error));
    }
    let cache_key = params.use_cache.unwrap_or(false).then(|| {
        format!(
            "{}:{collection}:{}",
            state.backend.cache_generation(),
            serde_json::to_string(&params).unwrap_or_default()
        )
    });
    if let Some(value) = cache_key
        .as_deref()
        .and_then(|key| state.cache.lock().ok().and_then(|mut cache| cache.get(key)))
    {
        state.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
        record_elapsed(&state, started);
        return Ok(json_bytes_response(value));
    }

    let channel_search = match collection.as_str() {
        TRANSCRIPT_COLLECTION => false,
        CHANNEL_COLLECTION => true,
        _ => return Err(ServerError::NotFound(collection)),
    };
    let permit = Arc::clone(&state.search_permits)
        .acquire_owned()
        .await
        .map_err(|error| {
            ServerError::Backend(BackendError::Internal(format!(
                "search concurrency limiter closed: {error}"
            )))
        })?;
    let backend = Arc::clone(&state.backend);
    let backend_params = params.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if channel_search {
            backend.search_channels(&backend_params)
        } else {
            backend.search(&backend_params)
        }
    })
    .await
    .map_err(|error| {
        ServerError::Backend(BackendError::Internal(format!(
            "search task terminated: {error}"
        )))
    })?;
    record_elapsed(&state, started);
    match result {
        Ok(value) => {
            let body = Bytes::from(serde_json::to_vec(&value).map_err(|error| {
                ServerError::Backend(BackendError::Internal(error.to_string()))
            })?);
            if let Some(key) = cache_key {
                let ttl = params.cache_ttl.unwrap_or(60);
                if let Ok(mut cache) = state.cache.lock() {
                    cache.insert(key, body.clone(), ttl);
                }
            }
            Ok(json_bytes_response(body))
        }
        Err(error) => {
            state.metrics.errors.fetch_add(1, Ordering::Relaxed);
            Err(ServerError::Backend(error))
        }
    }
}

fn validate_cache_params(params: &SearchParams) -> Result<(), BackendError> {
    let Some(ttl) = params.cache_ttl else {
        return Ok(());
    };
    if params.use_cache != Some(true) {
        return Err(BackendError::InvalidInput(
            "cache_ttl requires use_cache=true".to_string(),
        ));
    }
    if !(1..=MAX_CACHE_TTL_SECONDS).contains(&ttl) {
        return Err(BackendError::InvalidInput(format!(
            "cache_ttl must be between 1 and {MAX_CACHE_TTL_SECONDS} seconds"
        )));
    }
    Ok(())
}

fn record_elapsed(state: &AppState, started: Instant) {
    state
        .metrics
        .search_nanos
        .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

impl SearchCache {
    fn get(&mut self, key: &str) -> Option<Bytes> {
        let now = Instant::now();
        self.entries.retain(|entry| entry.expires > now);
        self.entries
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| entry.value.clone())
    }

    fn insert(&mut self, key: String, value: Bytes, ttl_seconds: u64) {
        const CAPACITY: usize = 128;
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(CacheEntry {
            key,
            expires: Instant::now() + std::time::Duration::from_secs(ttl_seconds),
            value,
        });
    }
}

fn json_bytes_response(body: Bytes) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

enum ServerError {
    Backend(BackendError),
    NotFound(String),
    Unauthorized,
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Backend(BackendError::InvalidInput(error)) => (StatusCode::BAD_REQUEST, error),
            Self::Backend(BackendError::Internal(error)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, error)
            }
            Self::NotFound(collection) => (
                StatusCode::NOT_FOUND,
                format!("unknown collection: {collection}"),
            ),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "invalid search key".to_string()),
        };
        (status, Json(json!({ "message": message }))).into_response()
    }
}

fn process_usage() -> (u64, u64) {
    let rss_pages = fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|stat| stat.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or_default();
    let cpu_ticks = fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| {
            let close = stat.rfind(')')?;
            let fields = stat
                .get(close + 2..)?
                .split_whitespace()
                .collect::<Vec<_>>();
            let user = fields.get(11)?.parse::<u64>().ok()?;
            let system = fields.get(12)?.parse::<u64>().ok()?;
            Some(user.saturating_add(system))
        })
        .unwrap_or_default();
    (rss_pages.saturating_mul(4_096), cpu_ticks)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Duration};

    struct SlowBackend;

    impl SearchBackend for SlowBackend {
        fn name(&self) -> &'static str {
            "slow-test"
        }

        fn search(&self, _params: &SearchParams) -> Result<Value, BackendError> {
            thread::sleep(Duration::from_millis(200));
            Ok(json!({ "found": 1 }))
        }

        fn search_channels(&self, params: &SearchParams) -> Result<Value, BackendError> {
            self.search(params)
        }

        fn documents(&self) -> u64 {
            1
        }

        fn index_bytes(&self) -> u64 {
            1
        }
    }

    fn test_state(backend: Arc<dyn SearchBackend>) -> AppState {
        AppState {
            backend,
            metrics: Arc::new(Metrics::default()),
            started: Instant::now(),
            api_key: None,
            cache: Arc::new(Mutex::new(SearchCache::default())),
            search_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_BACKEND_SEARCHES)),
        }
    }

    #[test]
    fn unknown_search_parameters_fail_closed() {
        for query in ["unsupported=true", "facet_by=video_id", "infix=always"] {
            let uri = format!("/?q=blackhole&{query}").parse().expect("URI");
            assert!(Query::<SearchParams>::try_from_uri(&uri).is_err());
        }
    }

    #[test]
    fn invalid_backend_input_is_a_bad_request() {
        let response = ServerError::Backend(BackendError::InvalidInput("bad query".to_string()))
            .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn cache_is_bounded_and_expires_by_contract() {
        let mut cache = SearchCache::default();
        for index in 0..140 {
            cache.insert(index.to_string(), Bytes::from_static(b"{}"), 60);
        }
        assert_eq!(cache.entries.len(), 128);
        assert!(cache.get("0").is_none());
        assert!(cache.get("139").is_some());
    }

    #[test]
    fn cache_parameters_fail_closed_instead_of_clamping() {
        let mut params = SearchParams {
            q: "blackhole".to_string(),
            cache_ttl: Some(60),
            ..SearchParams::default()
        };
        assert!(validate_cache_params(&params).is_err());
        params.use_cache = Some(true);
        assert!(validate_cache_params(&params).is_ok());
        for ttl in [0, MAX_CACHE_TTL_SECONDS + 1] {
            params.cache_ttl = Some(ttl);
            assert!(validate_cache_params(&params).is_err());
        }
    }

    #[tokio::test]
    async fn blocking_backend_search_does_not_starve_request_runtime() {
        let state = test_state(Arc::new(SlowBackend));
        let started = Instant::now();
        let search_task = tokio::spawn(search(
            State(state.clone()),
            Path(TRANSCRIPT_COLLECTION.to_string()),
            Query(SearchParams {
                q: "blackhole".to_string(),
                ..SearchParams::default()
            }),
            HeaderMap::new(),
        ));
        tokio::task::yield_now().await;

        let health = health(State(state)).await;
        assert_eq!(health.0["ok"], true);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "health handler was starved by synchronous search work"
        );
        assert!(search_task.await.unwrap().is_ok());
    }
}
