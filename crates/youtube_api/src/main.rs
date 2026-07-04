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
const MERGE_TRANSCRIPT_HIT_WINDOW_MS: u32 = 20_000;
const MAX_SEARCH_PAGE_FETCHES: u32 = 5;

#[derive(Clone)]
struct AppState {
    search: TypesenseSearchClient,
}

#[derive(Debug, Deserialize)]
struct SearchParams {
    q: String,
    youtuber: Option<String>,
    per_page: Option<u8>,
    page: Option<u32>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    total_found: u64,
    out_of: u64,
    page: u32,
    per_page: u8,
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
    let page = params.page.unwrap_or(1).max(1);
    let search_page_size = per_page.saturating_mul(4).min(MAX_PER_PAGE);
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
    let hits = collect_visible_hits(
        &state.search,
        &typesense_query,
        first_raw,
        typesense_page,
        local_offset,
        per_page,
        search_page_size,
    )
    .await?;
    tracing::info!(query, found, returned_hits = hits.len(), "search response");

    Ok(Json(SearchResponse {
        query: query.to_string(),
        total_found: found.max(hits.len() as u64),
        out_of,
        page,
        per_page,
        hits,
    }))
}

async fn collect_visible_hits(
    search: &TypesenseSearchClient,
    base_query: &TextSearchQuery,
    first_raw: Value,
    first_typesense_page: u32,
    local_offset: usize,
    per_page: u8,
    search_page_size: u8,
) -> Result<Vec<SearchHit>, ApiError> {
    let required_hits = local_offset.saturating_add(usize::from(per_page));
    let mut combined = Vec::new();
    let mut current_page = first_typesense_page;
    let mut raw = first_raw;

    for fetch_index in 0..MAX_SEARCH_PAGE_FETCHES {
        let page_hits = parse_hits(&raw)?;
        let is_empty = page_hits.is_empty();
        combined.extend(page_hits);
        combined = merge_nearby_transcript_hits(dedupe_title_only_hits(combined));

        if combined.len() >= required_hits || is_empty {
            break;
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

    Ok(combined
        .into_iter()
        .skip(local_offset)
        .take(usize::from(per_page))
        .collect())
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
