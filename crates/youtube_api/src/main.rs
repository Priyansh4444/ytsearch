use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env,
    net::{AddrParseError, SocketAddr},
    sync::Arc,
};
use thiserror::Error;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use youtube_typesense::{
    DEFAULT_COLLECTION, TextSearchQuery, TypesenseSearchClient, TypesenseSearchConfig,
};

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_PER_PAGE: u8 = 20;
const MAX_PER_PAGE: u8 = 100;

#[derive(Clone)]
struct AppState {
    search: TypesenseSearchClient,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    youtuber: Option<String>,
    per_page: Option<u8>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    total_found: u64,
    out_of: u64,
    hits: Vec<SearchHit>,
}

#[derive(Debug, Serialize)]
struct SearchHit {
    id: String,
    video_id: String,
    title: String,
    text: String,
    prev_text: Option<String>,
    next_text: Option<String>,
    text_highlight: Option<String>,
    prev_text_highlight: Option<String>,
    next_text_highlight: Option<String>,
    title_highlight: Option<String>,
    youtuber_handle: String,
    youtuber_name: String,
    thumbnail: Option<String>,
    published_at: Option<String>,
    t_start_ms: u32,
    t_end_ms: u32,
    timestamp: String,
    youtube_url: String,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("missing required environment variable {0}")]
    MissingEnv(&'static str),
    #[error("invalid bind address: {0}")]
    InvalidBindAddress(String),
    #[error("query cannot be empty")]
    EmptyQuery,
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "youtube_api=info,tower_http=info".into()),
        )
        .init();

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
    });
    let app = Router::new()
        .route("/api/health", get(health))
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

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn search_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, ApiError> {
    let query = params.q.trim();
    if query.is_empty() {
        return Err(ApiError::EmptyQuery);
    }

    let per_page = params
        .per_page
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(1, MAX_PER_PAGE);
    let mut typesense_query = TextSearchQuery::new(query)
        .query_by("text,prev_text,next_text,title,youtuber_name,youtuber_handle")
        .highlight_fields("text,prev_text,next_text,title")
        .per_page(per_page)
        .without_grouping();
    tracing::info!(
        query,
        per_page,
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

    let raw = state.search.search(&typesense_query).await?;
    let hits = parse_hits(&raw)?;
    let found = raw.get("found").and_then(Value::as_u64).unwrap_or_default();
    tracing::info!(query, found, returned_hits = hits.len(), "search response");

    Ok(Json(SearchResponse {
        query: query.to_string(),
        total_found: raw
            .get("found")
            .and_then(Value::as_u64)
            .unwrap_or(hits.len() as u64),
        out_of: raw
            .get("out_of")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        hits,
    }))
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

impl SearchHit {
    fn from_typesense_hit(document: &Value, hit: &Value) -> Result<Self, ApiError> {
        let id = string_field(document, "id")?;
        let video_id = string_field(document, "video_id")?;
        let t_start_ms = u32_field(document, "t_start_ms")?;
        let t_end_ms = u32_field(document, "t_end_ms")?;
        let timestamp = format_timestamp(t_start_ms);
        let youtube_url = format!(
            "https://www.youtube.com/watch?v={video_id}&t={}s",
            t_start_ms / 1000
        );

        Ok(Self {
            id,
            video_id,
            title: string_field(document, "title")?,
            text: string_field(document, "text")?,
            prev_text: optional_string_field(document, "prev_text"),
            next_text: optional_string_field(document, "next_text"),
            text_highlight: highlight_for_field(hit, "text"),
            prev_text_highlight: highlight_for_field(hit, "prev_text"),
            next_text_highlight: highlight_for_field(hit, "next_text"),
            title_highlight: highlight_for_field(hit, "title"),
            youtuber_handle: string_field(document, "youtuber_handle")?,
            youtuber_name: string_field(document, "youtuber_name")?,
            thumbnail: optional_string_field(document, "thumbnail"),
            published_at: optional_string_field(document, "published_at"),
            t_start_ms,
            t_end_ms,
            timestamp,
            youtube_url,
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
    if handle.starts_with('@') {
        handle.to_string()
    } else {
        format!("@{handle}")
    }
}

fn required_env(name: &'static str) -> Result<String, ApiError> {
    env::var(name).map_err(|_| ApiError::MissingEnv(name))
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::warn!(%error, "failed to install Ctrl+C handler");
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::EmptyQuery => StatusCode::BAD_REQUEST,
            Self::MissingEnv(_) | Self::InvalidBindAddress(_) | Self::Server(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Self::Typesense(_) => StatusCode::BAD_GATEWAY,
            Self::UnexpectedSearchShape => StatusCode::BAD_GATEWAY,
        };

        let body = Json(serde_json::json!({ "error": self.to_string() }));
        (status, body).into_response()
    }
}
