use crate::{
    ADMIN_REQUEST_TIMEOUT, BackendReadiness, ChannelDeletionScope, ChannelDocument, ChunkStats,
    IndexOperation, IndexStoreError, RawGenerationManifest, RawGenerationUpload, RawPublicationJob,
    VideoIdPage, VideoReplacement,
};
use reqwest::{Client, RequestBuilder, StatusCode, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{num::NonZeroU16, path::Path, time::Duration};
use tokio::io::AsyncWriteExt;

const BACKEND: &str = "raw";
const ADMIN_KEY_HEADER: &str = "x-raw-admin-key";
const DEFAULT_PUBLICATION_TRANSFER_TIMEOUT: Duration = Duration::from_secs(45 * 60);

#[derive(Clone, Debug)]
pub struct RawIndexAdminConfig {
    base_url: String,
    api_key: String,
    timeout: Duration,
    publication_timeout: Duration,
}

impl RawIndexAdminConfig {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            timeout: ADMIN_REQUEST_TIMEOUT,
            publication_timeout: DEFAULT_PUBLICATION_TRANSFER_TIMEOUT,
        }
    }

    pub fn with_publication_timeout(mut self, timeout: Duration) -> Self {
        self.publication_timeout = timeout;
        self
    }
}

#[derive(Clone)]
pub struct RawIndexAdminClient {
    client: Client,
    base_url: String,
    api_key: String,
    publication_timeout: Duration,
}

#[derive(Serialize)]
struct ReplaceVideoRequest<'a> {
    publication_api_version: u16,
    idempotency_key: &'a str,
    chunks: &'a [youtube_ingest::TranscriptChunk],
}

#[derive(Serialize)]
struct UpsertChannelRequest<'a> {
    publication_api_version: u16,
    idempotency_key: &'a str,
    document: &'a ChannelDocument,
}

#[derive(Serialize)]
struct DeleteVideoRequest<'a> {
    publication_api_version: u16,
    idempotency_key: &'a str,
}

#[derive(Serialize)]
struct DeleteChannelRequest<'a> {
    publication_api_version: u16,
    idempotency_key: &'a str,
    cascade: bool,
}

#[derive(Deserialize)]
struct CountResponse {
    count: u64,
}

impl RawIndexAdminClient {
    pub fn new(config: RawIndexAdminConfig) -> Result<Self, IndexStoreError> {
        if config.api_key.is_empty() {
            return Err(IndexStoreError::InvalidInput(
                "raw admin API key must not be empty".to_string(),
            ));
        }
        let base_url = config.base_url.trim_end_matches('/').to_string();
        let parsed = reqwest::Url::parse(&base_url).map_err(|error| {
            IndexStoreError::InvalidInput(format!("invalid raw admin URL: {error}"))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(IndexStoreError::InvalidInput(
                "raw admin URL must use http or https".to_string(),
            ));
        }
        let client = Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(map_http)?;
        Ok(Self {
            client,
            base_url,
            api_key: config.api_key,
            publication_timeout: config.publication_timeout,
        })
    }

    fn request(&self, request: RequestBuilder) -> RequestBuilder {
        request.header(ADMIN_KEY_HEADER, &self.api_key)
    }

    async fn execute<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, IndexStoreError> {
        let response = self.request(request).send().await.map_err(map_http)?;
        if !response.status().is_success() {
            return Err(error_response(response).await);
        }
        response.json().await.map_err(map_http)
    }

    async fn execute_job(
        &self,
        request: RequestBuilder,
    ) -> Result<RawPublicationJob, IndexStoreError> {
        let job = self.execute(request).await?;
        validate_publication_job_response(&job)?;
        Ok(job)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Prepare exports the reader's whole mutation store, which grows with
    /// every indexed video, so it gets the transfer timeout. Under the admin
    /// timeout, a 964 MB export failed two daily publications in production.
    pub async fn prepare_publication(&self) -> Result<RawPublicationJob, IndexStoreError> {
        self.execute_job(
            self.client
                .post(self.endpoint("/admin/publications/prepare"))
                .timeout(self.publication_timeout),
        )
        .await
    }

    pub async fn publication_status(
        &self,
        job_id: &str,
    ) -> Result<RawPublicationJob, IndexStoreError> {
        validate_publication_job_id(job_id)?;
        self.execute_job(
            self.client
                .get(self.endpoint(&format!("/admin/publications/{job_id}"))),
        )
        .await
    }

    pub async fn download_publication_snapshot(
        &self,
        job: &RawPublicationJob,
        destination: &Path,
    ) -> Result<(), IndexStoreError> {
        validate_publication_job_id(&job.job_id)?;
        let mut response = self
            .request(
                self.client
                    .get(self.endpoint(&format!("/admin/publications/{}/snapshot", job.job_id))),
            )
            .timeout(self.publication_timeout)
            .send()
            .await
            .map_err(map_http)?;
        if !response.status().is_success() {
            return Err(error_response(response).await);
        }
        if response.content_length() != Some(job.snapshot_bytes) {
            return Err(IndexStoreError::InvalidInput(
                "raw publication snapshot Content-Length does not match the prepared job"
                    .to_string(),
            ));
        }
        let expected_etag = format!("\"{}\"", job.snapshot_blake3);
        if response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            != Some(expected_etag.as_str())
        {
            return Err(IndexStoreError::InvalidInput(
                "raw publication snapshot ETag does not match the prepared job".to_string(),
            ));
        }

        let temporary = destination.with_extension("download");
        if temporary.exists() {
            tokio::fs::remove_file(&temporary).await.map_err(map_io)?;
        }
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
            .map_err(map_io)?;
        let mut received = 0_u64;
        let mut hasher = blake3::Hasher::new();
        let result = async {
            while let Some(chunk) = response.chunk().await.map_err(map_http)? {
                received = received.checked_add(chunk.len() as u64).ok_or_else(|| {
                    IndexStoreError::InvalidInput(
                        "raw publication snapshot length overflows".to_string(),
                    )
                })?;
                if received > job.snapshot_bytes {
                    return Err(IndexStoreError::InvalidInput(
                        "raw publication snapshot exceeds its prepared length".to_string(),
                    ));
                }
                hasher.update(&chunk);
                output.write_all(&chunk).await.map_err(map_io)?;
            }
            if received != job.snapshot_bytes
                || hasher.finalize().to_hex().as_str() != job.snapshot_blake3
            {
                return Err(IndexStoreError::InvalidInput(
                    "raw publication snapshot failed length or BLAKE3 validation".to_string(),
                ));
            }
            output.flush().await.map_err(map_io)?;
            output.sync_all().await.map_err(map_io)?;
            Ok::<(), IndexStoreError>(())
        }
        .await;
        drop(output);
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
        tokio::fs::rename(&temporary, destination)
            .await
            .map_err(map_io)
    }

    pub async fn accept_publication_manifest(
        &self,
        job: &RawPublicationJob,
        manifest: RawGenerationManifest,
    ) -> Result<RawPublicationJob, IndexStoreError> {
        validate_publication_job_id(&job.job_id)?;
        self.execute_job(
            self.client
                .put(self.endpoint(&format!("/admin/publications/{}/manifest", job.job_id)))
                .json(&RawGenerationUpload {
                    target_sequence: job.target_sequence,
                    compatibility: job.compatibility.clone(),
                    manifest,
                }),
        )
        .await
    }

    pub async fn upload_publication_file(
        &self,
        job_id: &str,
        file_name: &str,
        path: &Path,
    ) -> Result<RawPublicationJob, IndexStoreError> {
        validate_publication_job_id(job_id)?;
        validate_generation_file_name(file_name)?;
        let file = tokio::fs::File::open(path).await.map_err(map_io)?;
        let bytes = file.metadata().await.map_err(map_io)?.len();
        self.execute_job(
            self.client
                .put(self.endpoint(&format!("/admin/publications/{job_id}/files/{file_name}")))
                .header(header::CONTENT_LENGTH, bytes)
                .body(reqwest::Body::from(file))
                .timeout(self.publication_timeout),
        )
        .await
    }

    pub async fn activate_publication(
        &self,
        job_id: &str,
    ) -> Result<RawPublicationJob, IndexStoreError> {
        validate_publication_job_id(job_id)?;
        self.execute_job(
            self.client
                .post(self.endpoint(&format!("/admin/publications/{job_id}/activate"))),
        )
        .await
    }
}

fn validate_publication_job_id(value: &str) -> Result<(), IndexStoreError> {
    let valid = value.len() == 37
        && value.as_bytes()[..20].iter().all(u8::is_ascii_digit)
        && value.as_bytes()[20] == b'-'
        && is_lower_hex(&value.as_bytes()[21..]);
    if !valid {
        return Err(IndexStoreError::InvalidInput(
            "raw publication job id is invalid".to_string(),
        ));
    }
    Ok(())
}

fn validate_generation_file_name(value: &str) -> Result<(), IndexStoreError> {
    if !crate::RAW_GENERATION_FILES.contains(&value) {
        return Err(IndexStoreError::InvalidInput(
            "raw generation file name is not allowlisted".to_string(),
        ));
    }
    Ok(())
}

fn validate_publication_job_response(job: &RawPublicationJob) -> Result<(), IndexStoreError> {
    validate_publication_job_id(&job.job_id)?;
    if job.target_sequence == 0 || job.snapshot_bytes == 0 {
        return Err(IndexStoreError::InvalidInput(
            "raw publication job has an invalid sequence or snapshot length".to_string(),
        ));
    }
    validate_blake3("snapshot", &job.snapshot_blake3)?;
    validate_blake3("corpus", &job.compatibility.corpus_identity)?;
    validate_blake3("base chunks", &job.compatibility.base_chunks_blake3)?;
    validate_blake3("base channels", &job.compatibility.base_channels_blake3)?;
    if job.compatibility.generation_format.is_empty()
        || !job
            .completed_files
            .iter()
            .all(|name| crate::RAW_GENERATION_FILES.contains(&name.as_str()))
    {
        return Err(IndexStoreError::InvalidInput(
            "raw publication job contains invalid compatibility or completed files".to_string(),
        ));
    }
    Ok(())
}

fn validate_blake3(label: &str, value: &str) -> Result<(), IndexStoreError> {
    if value.len() != 64 || !is_lower_hex(value.as_bytes()) {
        return Err(IndexStoreError::InvalidInput(format!(
            "raw publication {label} BLAKE3 digest is invalid"
        )));
    }
    Ok(())
}

fn is_lower_hex(value: &[u8]) -> bool {
    value
        .iter()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn map_io(error: std::io::Error) -> IndexStoreError {
    IndexStoreError::unavailable(BACKEND, error)
}

/// Raw's write and control plane, used by indexers and repair jobs.
///
/// Mutations must be idempotent for a stable `idempotency_key`. Returning an
/// operation does not imply replica visibility; callers must inspect `state`
/// and `acknowledged_readers` when they require read-after-write guarantees.
/// The write contract lives outside the latency-sensitive query crates so its
/// dependencies and state cannot enter the search hot path.
impl RawIndexAdminClient {
    pub async fn readiness(&self) -> Result<BackendReadiness, IndexStoreError> {
        self.execute(self.client.get(self.endpoint("/admin/readiness")))
            .await
    }

    pub async fn publish(&self) -> Result<BackendReadiness, IndexStoreError> {
        let _: serde_json::Value = self
            .execute(self.client.post(self.endpoint("/admin/publish")))
            .await?;
        self.readiness().await
    }

    pub async fn channel(
        &self,
        channel_id: &str,
    ) -> Result<Option<ChannelDocument>, IndexStoreError> {
        self.execute(
            self.client
                .get(self.endpoint(&format!("/admin/channels/{channel_id}"))),
        )
        .await
    }

    pub async fn replace_video(
        &self,
        replacement: VideoReplacement<'_>,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .put(self.endpoint(&format!("/admin/videos/{}", replacement.video_id())))
                .json(&ReplaceVideoRequest {
                    publication_api_version: crate::RAW_PUBLICATION_API_VERSION,
                    idempotency_key,
                    chunks: replacement.chunks(),
                }),
        )
        .await
    }

    pub async fn delete_video(
        &self,
        video_id: &str,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .delete(self.endpoint(&format!("/admin/videos/{video_id}")))
                .query(&DeleteVideoRequest {
                    publication_api_version: crate::RAW_PUBLICATION_API_VERSION,
                    idempotency_key,
                }),
        )
        .await
    }

    pub async fn upsert_channel(
        &self,
        channel: &ChannelDocument,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .put(self.endpoint(&format!("/admin/channels/{}", channel.id)))
                .json(&UpsertChannelRequest {
                    publication_api_version: crate::RAW_PUBLICATION_API_VERSION,
                    idempotency_key,
                    document: channel,
                }),
        )
        .await
    }

    pub async fn delete_channel(
        &self,
        channel_id: &str,
        scope: ChannelDeletionScope,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .delete(self.endpoint(&format!("/admin/channels/{channel_id}")))
                .query(&DeleteChannelRequest {
                    publication_api_version: crate::RAW_PUBLICATION_API_VERSION,
                    idempotency_key,
                    cascade: scope.cascades_videos(),
                }),
        )
        .await
    }

    pub async fn operation(&self, operation_id: &str) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .get(self.endpoint(&format!("/admin/operations/{operation_id}"))),
        )
        .await
    }

    pub async fn video_chunk_count(&self, video_id: &str) -> Result<u64, IndexStoreError> {
        let response: CountResponse = self
            .execute(
                self.client
                    .get(self.endpoint(&format!("/admin/videos/{video_id}/chunk-count"))),
            )
            .await?;
        Ok(response.count)
    }

    pub async fn indexed_video_ids(
        &self,
        channel_id: &str,
        cursor: Option<&str>,
        limit: NonZeroU16,
    ) -> Result<VideoIdPage, IndexStoreError> {
        let request = self
            .client
            .get(self.endpoint(&format!("/admin/channels/{channel_id}/video-ids")))
            .query(&[("limit", limit.get())]);
        let request = if let Some(cursor) = cursor {
            request.query(&[("cursor", cursor)])
        } else {
            request
        };
        self.execute(request).await
    }

    pub async fn channel_stats(&self, channel_id: &str) -> Result<ChunkStats, IndexStoreError> {
        self.execute(
            self.client
                .get(self.endpoint(&format!("/admin/channels/{channel_id}/stats"))),
        )
        .await
    }
}

fn map_http(error: reqwest::Error) -> IndexStoreError {
    if error.is_timeout() || error.is_connect() {
        IndexStoreError::unavailable(BACKEND, error)
    } else {
        IndexStoreError::internal(BACKEND, error)
    }
}

async fn error_response(response: reqwest::Response) -> IndexStoreError {
    let status = response.status();
    let message = response
        .text()
        .await
        .unwrap_or_else(|_| format!("raw admin returned {status}"));
    map_status(status, message)
}

fn map_status(status: StatusCode, body: String) -> IndexStoreError {
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value.get("message")?.as_str().map(ToOwned::to_owned))
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| {
            if body.trim().is_empty() {
                format!("raw admin returned {status}")
            } else {
                body
            }
        });
    match status {
        StatusCode::BAD_REQUEST
        | StatusCode::UNAUTHORIZED
        | StatusCode::FORBIDDEN
        | StatusCode::NOT_FOUND => IndexStoreError::InvalidInput(message),
        StatusCode::CONFLICT => IndexStoreError::Conflict(message),
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::SERVICE_UNAVAILABLE => IndexStoreError::unavailable(BACKEND, message),
        _ => IndexStoreError::internal(BACKEND, message),
    }
}

#[cfg(test)]
mod tests;
