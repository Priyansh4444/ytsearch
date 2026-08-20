use crate::{
    MutationStore, Publisher, ReloadableEngine, channel_ids_equal,
    remote_publication::{GenerationUploadRequest, RemotePublication},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, num::NonZeroU16, sync::Arc};
use tokio_util::io::ReaderStream;
use youtube_index_store::{
    BackendReadiness, ChannelDocument, IndexStoreError, VideoIdPage, VideoReplacement,
};
use youtube_ingest::TranscriptChunk;

const ADMIN_KEY_HEADER: &str = "x-raw-admin-key";
const MAX_MUTATION_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_PUBLICATION_MANIFEST_BYTES: usize = 64 * 1024;
const DEFAULT_PAGE_SIZE: u16 = 1_000;
const MAX_PAGE_SIZE: u16 = 10_000;

#[derive(Clone)]
pub struct AdminState {
    engine: Arc<ReloadableEngine>,
    store: Arc<MutationStore>,
    remote_publication: RemotePublication,
    api_key: Arc<str>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceVideoRequest {
    idempotency_key: String,
    chunks: Vec<TranscriptChunk>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteRequest {
    idempotency_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteChannelRequest {
    idempotency_key: String,
    #[serde(default)]
    cascade: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpsertChannelRequest {
    idempotency_key: String,
    document: ChannelDocument,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    cursor: Option<String>,
    limit: Option<u16>,
}

impl AdminState {
    pub fn new(
        engine: Arc<ReloadableEngine>,
        store: Arc<MutationStore>,
        publisher: Arc<Publisher>,
        api_key: String,
    ) -> Result<Self, IndexStoreError> {
        if api_key.len() < 24 {
            return Err(IndexStoreError::InvalidInput(
                "RAW_SEARCH_ADMIN_API_KEY must contain at least 24 bytes".to_string(),
            ));
        }
        let remote_publication =
            RemotePublication::new(Arc::clone(&engine), Arc::clone(&store), publisher)?;
        Ok(Self {
            engine,
            store,
            remote_publication,
            api_key: Arc::from(api_key),
        })
    }
}

pub fn router(state: AdminState) -> Router {
    let mutations = Router::new()
        .route("/admin/readiness", get(readiness))
        .route("/admin/publish", post(publish))
        .route("/admin/operations/{operation_id}", get(operation))
        .route(
            "/admin/videos/{video_id}",
            put(replace_video).delete(delete_video),
        )
        .route(
            "/admin/videos/{video_id}/chunk-count",
            get(video_chunk_count),
        )
        .route(
            "/admin/channels/{channel_id}",
            get(channel).put(upsert_channel).delete(delete_channel),
        )
        .route(
            "/admin/channels/{channel_id}/video-ids",
            get(indexed_video_ids),
        )
        .route("/admin/channels/{channel_id}/stats", get(channel_stats))
        .layer(DefaultBodyLimit::max(MAX_MUTATION_BODY_BYTES));
    let remote_publication = Router::new()
        .route("/admin/publications/prepare", post(prepare_publication))
        .route("/admin/publications/{job_id}", get(publication_status))
        .route(
            "/admin/publications/{job_id}/snapshot",
            get(publication_snapshot),
        )
        .route(
            "/admin/publications/{job_id}/manifest",
            put(publication_manifest).layer(DefaultBodyLimit::max(MAX_PUBLICATION_MANIFEST_BYTES)),
        )
        .route(
            "/admin/publications/{job_id}/files/{file_name}",
            put(publication_file).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/admin/publications/{job_id}/activate",
            post(activate_publication),
        );
    Router::new()
        .merge(mutations)
        .merge(remote_publication)
        .with_state(state)
}

async fn readiness(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Json<BackendReadiness>, AdminError> {
    authorize(&state, &headers)?;
    Ok(Json(readiness_value(&state)?))
}

async fn publish(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<Response, AdminError> {
    authorize(&state, &headers)?;
    Err(AdminError::Store(IndexStoreError::Conflict(
        "local publication is disabled; use /admin/publications/prepare with the remote builder"
            .to_string(),
    )))
}

async fn prepare_publication(
    State(state): State<AdminState>,
    headers: HeaderMap,
) -> Result<
    (
        StatusCode,
        Json<crate::remote_publication::RemoteJobResponse>,
    ),
    AdminError,
> {
    authorize(&state, &headers)?;
    Ok((
        StatusCode::CREATED,
        Json(state.remote_publication.prepare().await?),
    ))
}

async fn publication_status(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<crate::remote_publication::RemoteJobResponse>, AdminError> {
    authorize(&state, &headers)?;
    Ok(Json(state.remote_publication.status(&job_id).await?))
}

async fn publication_snapshot(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AdminError> {
    authorize(&state, &headers)?;
    let snapshot = state.remote_publication.open_snapshot(&job_id).await?;
    let mut response = Response::new(Body::from_stream(ReaderStream::with_capacity(
        snapshot.file,
        256 * 1024,
    )));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.sqlite3"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&snapshot.bytes.to_string())
            .map_err(|error| AdminError::Store(IndexStoreError::internal("raw", error)))?,
    );
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", snapshot.blake3))
            .map_err(|error| AdminError::Store(IndexStoreError::internal("raw", error)))?,
    );
    Ok(response)
}

async fn publication_manifest(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<GenerationUploadRequest>,
) -> Result<Json<crate::remote_publication::RemoteJobResponse>, AdminError> {
    authorize(&state, &headers)?;
    Ok(Json(
        state
            .remote_publication
            .accept_manifest(&job_id, request)
            .await?,
    ))
}

async fn publication_file(
    State(state): State<AdminState>,
    Path((job_id, file_name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<crate::remote_publication::RemoteJobResponse>, AdminError> {
    authorize(&state, &headers)?;
    let content_length = headers
        .get(header::CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .map_err(|_| {
                    IndexStoreError::InvalidInput("invalid Content-Length header".to_string())
                })?
                .parse::<u64>()
                .map_err(|_| {
                    IndexStoreError::InvalidInput("invalid Content-Length header".to_string())
                })
        })
        .transpose()?;
    Ok(Json(
        state
            .remote_publication
            .upload_file(&job_id, &file_name, content_length, body)
            .await?,
    ))
}

async fn activate_publication(
    State(state): State<AdminState>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<crate::remote_publication::RemoteJobResponse>, AdminError> {
    authorize(&state, &headers)?;
    Ok(Json(state.remote_publication.activate(&job_id).await?))
}

async fn operation(
    State(state): State<AdminState>,
    Path(operation_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<youtube_index_store::IndexOperation>, AdminError> {
    authorize(&state, &headers)?;
    Ok(Json(state.store.operation(&operation_id)?))
}

async fn replace_video(
    State(state): State<AdminState>,
    Path(video_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ReplaceVideoRequest>,
) -> Result<(StatusCode, Json<youtube_index_store::IndexOperation>), AdminError> {
    authorize(&state, &headers)?;
    let store = Arc::clone(&state.store);
    let operation = tokio::task::spawn_blocking(move || {
        let replacement = VideoReplacement::new(&video_id, &request.chunks)?;
        store.replace_video(replacement, &request.idempotency_key)
    })
    .await
    .map_err(AdminError::task)??;
    Ok((StatusCode::ACCEPTED, Json(operation)))
}

async fn delete_video(
    State(state): State<AdminState>,
    Path(video_id): Path<String>,
    headers: HeaderMap,
    Query(request): Query<DeleteRequest>,
) -> Result<(StatusCode, Json<youtube_index_store::IndexOperation>), AdminError> {
    authorize(&state, &headers)?;
    let store = Arc::clone(&state.store);
    let operation = tokio::task::spawn_blocking(move || {
        store.delete_video(&video_id, &request.idempotency_key)
    })
    .await
    .map_err(AdminError::task)??;
    Ok((StatusCode::ACCEPTED, Json(operation)))
}

async fn upsert_channel(
    State(state): State<AdminState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UpsertChannelRequest>,
) -> Result<(StatusCode, Json<youtube_index_store::IndexOperation>), AdminError> {
    authorize(&state, &headers)?;
    if request.document.id != channel_id {
        return Err(AdminError::Store(IndexStoreError::InvalidInput(
            "channel document id must match the URL".to_string(),
        )));
    }
    let store = Arc::clone(&state.store);
    let operation = tokio::task::spawn_blocking(move || {
        store.upsert_channel(&request.document, &request.idempotency_key)
    })
    .await
    .map_err(AdminError::task)??;
    Ok((StatusCode::ACCEPTED, Json(operation)))
}

async fn delete_channel(
    State(state): State<AdminState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    Query(request): Query<DeleteChannelRequest>,
) -> Result<(StatusCode, Json<youtube_index_store::IndexOperation>), AdminError> {
    authorize(&state, &headers)?;
    let store = Arc::clone(&state.store);
    let operation = tokio::task::spawn_blocking(move || {
        store.delete_channel(&channel_id, request.cascade, &request.idempotency_key)
    })
    .await
    .map_err(AdminError::task)??;
    Ok((StatusCode::ACCEPTED, Json(operation)))
}

async fn video_chunk_count(
    State(state): State<AdminState>,
    Path(video_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, AdminError> {
    authorize(&state, &headers)?;
    let count = state.engine.active()?.video_chunk_count(&video_id);
    let count = state
        .store
        .video_mutation_summary(&video_id)?
        .map_or(count, |mutation| mutation.chunk_count.unwrap_or(0));
    Ok(Json(json!({ "count": count })))
}

async fn indexed_video_ids(
    State(state): State<AdminState>,
    Path(channel_id): Path<String>,
    Query(query): Query<PageQuery>,
    headers: HeaderMap,
) -> Result<Json<VideoIdPage>, AdminError> {
    authorize(&state, &headers)?;
    let limit = query.limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let limit = NonZeroU16::new(limit)
        .filter(|limit| limit.get() <= MAX_PAGE_SIZE)
        .ok_or_else(|| {
            AdminError::Store(IndexStoreError::InvalidInput(format!(
                "limit must be between 1 and {MAX_PAGE_SIZE}"
            )))
        })?;
    let video_ids = merged_video_counts(&state, &channel_id)?
        .into_keys()
        .collect();
    Ok(Json(paginate(video_ids, query.cursor.as_deref(), limit)))
}

async fn channel_stats(
    State(state): State<AdminState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<youtube_index_store::ChunkStats>, AdminError> {
    authorize(&state, &headers)?;
    let stats = merged_video_counts(&state, &channel_id)?
        .into_values()
        .fold(
            youtube_index_store::ChunkStats::default(),
            |mut stats, chunks| {
                stats.indexed_videos = stats.indexed_videos.saturating_add(1);
                stats.indexed_chunks = stats.indexed_chunks.saturating_add(chunks);
                stats
            },
        );
    Ok(Json(stats))
}

async fn channel(
    State(state): State<AdminState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Option<ChannelDocument>>, AdminError> {
    authorize(&state, &headers)?;
    if let Some(mutation) = state.store.channel_mutation(&channel_id)? {
        return Ok(Json(mutation.document));
    }
    Ok(Json(state.engine.active()?.channel_document(&channel_id)?))
}

fn merged_video_counts(
    state: &AdminState,
    channel_id: &str,
) -> Result<BTreeMap<String, u64>, AdminError> {
    let channel_deleted = state
        .store
        .channel_mutation(channel_id)?
        .is_some_and(|mutation| mutation.document.is_none() && mutation.cascade);
    let mut counts = if channel_deleted {
        BTreeMap::new()
    } else {
        state
            .engine
            .active()?
            .video_chunk_counts(channel_id)
            .into_iter()
            .collect()
    };
    for mutation in state.store.video_mutation_summaries()? {
        counts.remove(&mutation.video_id);
        if mutation
            .youtuber_id
            .as_deref()
            .is_some_and(|candidate| channel_ids_equal(candidate, channel_id))
            && let Some(chunk_count) = mutation.chunk_count
        {
            counts.insert(mutation.video_id, chunk_count);
        }
    }
    Ok(counts)
}

fn readiness_value(state: &AdminState) -> Result<BackendReadiness, AdminError> {
    Ok(BackendReadiness {
        backend: "raw".to_string(),
        ready: true,
        generation: Some(format!("{:020}", state.engine.sequence())),
        last_sequence: Some(state.store.last_sequence()?),
        acknowledged_readers: 1,
    })
}

fn authorize(state: &AdminState, headers: &HeaderMap) -> Result<(), AdminError> {
    let supplied = headers
        .get(ADMIN_KEY_HEADER)
        .and_then(|value| value.to_str().ok());
    if supplied.is_some_and(|supplied| constant_time_eq(supplied, state.api_key.as_ref())) {
        Ok(())
    } else {
        Err(AdminError::Unauthorized)
    }
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn paginate(mut video_ids: Vec<String>, cursor: Option<&str>, limit: NonZeroU16) -> VideoIdPage {
    video_ids.sort_unstable();
    let start = cursor.map_or(0, |cursor| {
        video_ids.partition_point(|video_id| video_id.as_str() <= cursor)
    });
    let end = start
        .saturating_add(usize::from(limit.get()))
        .min(video_ids.len());
    let next_cursor = (end < video_ids.len()).then(|| video_ids[end - 1].clone());
    VideoIdPage {
        video_ids: video_ids
            .into_iter()
            .skip(start)
            .take(end - start)
            .collect(),
        next_cursor,
    }
}

#[derive(Debug)]
enum AdminError {
    Store(IndexStoreError),
    Backend(youtube_search_service::BackendError),
    Unauthorized,
}

impl AdminError {
    fn task(error: tokio::task::JoinError) -> Self {
        Self::Store(IndexStoreError::internal("raw", error))
    }
}

impl From<IndexStoreError> for AdminError {
    fn from(error: IndexStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<youtube_search_service::BackendError> for AdminError {
    fn from(error: youtube_search_service::BackendError) -> Self {
        Self::Backend(error)
    }
}

impl From<crate::Error> for AdminError {
    fn from(error: crate::Error) -> Self {
        Self::Store(IndexStoreError::internal("raw", error))
    }
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "invalid raw admin key".to_string(),
            ),
            Self::Store(IndexStoreError::InvalidInput(message)) => {
                (StatusCode::BAD_REQUEST, message)
            }
            Self::Store(IndexStoreError::Conflict(message)) => (StatusCode::CONFLICT, message),
            Self::Store(IndexStoreError::Unavailable { message, .. }) => {
                (StatusCode::SERVICE_UNAVAILABLE, message)
            }
            Self::Store(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
            Self::Backend(youtube_search_service::BackendError::InvalidInput(message)) => {
                (StatusCode::BAD_REQUEST, message)
            }
            Self::Backend(youtube_search_service::BackendError::Internal(message)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, message)
            }
        };
        (status, Json(json!({ "message": message }))).into_response()
    }
}

#[cfg(test)]
mod tests;
