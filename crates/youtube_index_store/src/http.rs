use crate::{
    BackendReadiness, ChannelDocument, ChunkStats, IndexOperation, IndexStoreError,
    RawGenerationManifest, RawGenerationUpload, RawPublicationJob, SearchIndexAdmin, VideoIdPage,
    VideoReplacement,
};
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder, StatusCode, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{num::NonZeroU16, path::Path, time::Duration};
use tokio::io::AsyncWriteExt;

const BACKEND: &str = "raw";
const ADMIN_KEY_HEADER: &str = "x-raw-admin-key";
const DEFAULT_PUBLICATION_TRANSFER_TIMEOUT: Duration = Duration::from_secs(45 * 60);

#[derive(Clone, Debug)]
pub struct RawIndexAdminConfig {
    pub base_url: String,
    pub api_key: String,
    pub timeout: Duration,
    pub publication_timeout: Duration,
}

impl RawIndexAdminConfig {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            timeout: Duration::from_secs(30),
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
    idempotency_key: &'a str,
    chunks: &'a [youtube_ingest::TranscriptChunk],
}

#[derive(Serialize)]
struct UpsertChannelRequest<'a> {
    idempotency_key: &'a str,
    document: &'a ChannelDocument,
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
        let status = response.status();
        if status.is_success() {
            return response.json().await.map_err(map_http);
        }
        let message = response
            .text()
            .await
            .unwrap_or_else(|_| format!("raw admin returned {status}"));
        Err(map_status(status, message))
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    pub async fn prepare_publication(&self) -> Result<RawPublicationJob, IndexStoreError> {
        self.execute(
            self.client
                .post(self.endpoint("/admin/publications/prepare")),
        )
        .await
    }

    pub async fn publication_status(
        &self,
        job_id: &str,
    ) -> Result<RawPublicationJob, IndexStoreError> {
        validate_publication_component(job_id)?;
        self.execute(
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
        validate_publication_component(&job.job_id)?;
        let mut response = self
            .request(
                self.client
                    .get(self.endpoint(&format!("/admin/publications/{}/snapshot", job.job_id))),
            )
            .timeout(self.publication_timeout)
            .send()
            .await
            .map_err(map_http)?;
        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| format!("raw admin returned {status}"));
            return Err(map_status(status, message));
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
        validate_publication_component(&job.job_id)?;
        self.execute(
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
        validate_publication_component(job_id)?;
        validate_publication_component(file_name)?;
        let file = tokio::fs::File::open(path).await.map_err(map_io)?;
        let bytes = file.metadata().await.map_err(map_io)?.len();
        let response = self
            .client
            .put(self.endpoint(&format!("/admin/publications/{job_id}/files/{file_name}")))
            .header(header::CONTENT_LENGTH, bytes)
            .body(reqwest::Body::from(file))
            .timeout(self.publication_timeout);
        self.execute(response).await
    }

    pub async fn activate_publication(
        &self,
        job_id: &str,
    ) -> Result<RawPublicationJob, IndexStoreError> {
        validate_publication_component(job_id)?;
        self.execute(
            self.client
                .post(self.endpoint(&format!("/admin/publications/{job_id}/activate"))),
        )
        .await
    }
}

fn validate_publication_component(value: &str) -> Result<(), IndexStoreError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(IndexStoreError::InvalidInput(
            "raw publication URL component is invalid".to_string(),
        ));
    }
    Ok(())
}

fn map_io(error: std::io::Error) -> IndexStoreError {
    IndexStoreError::unavailable(BACKEND, error)
}

#[async_trait]
impl SearchIndexAdmin for RawIndexAdminClient {
    async fn readiness(&self) -> Result<BackendReadiness, IndexStoreError> {
        self.execute(self.client.get(self.endpoint("/admin/readiness")))
            .await
    }

    async fn publish(&self) -> Result<BackendReadiness, IndexStoreError> {
        let _: serde_json::Value = self
            .execute(self.client.post(self.endpoint("/admin/publish")))
            .await?;
        self.readiness().await
    }

    async fn channel(&self, channel_id: &str) -> Result<Option<ChannelDocument>, IndexStoreError> {
        self.execute(
            self.client
                .get(self.endpoint(&format!("/admin/channels/{channel_id}"))),
        )
        .await
    }

    async fn replace_video(
        &self,
        replacement: VideoReplacement<'_>,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .put(self.endpoint(&format!("/admin/videos/{}", replacement.video_id())))
                .json(&ReplaceVideoRequest {
                    idempotency_key,
                    chunks: replacement.chunks(),
                }),
        )
        .await
    }

    async fn delete_video(
        &self,
        video_id: &str,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .delete(self.endpoint(&format!("/admin/videos/{video_id}")))
                .query(&[("idempotency_key", idempotency_key)]),
        )
        .await
    }

    async fn upsert_channel(
        &self,
        channel: &ChannelDocument,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .put(self.endpoint(&format!("/admin/channels/{}", channel.id)))
                .json(&UpsertChannelRequest {
                    idempotency_key,
                    document: channel,
                }),
        )
        .await
    }

    async fn delete_channel(
        &self,
        channel_id: &str,
        cascade: bool,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .delete(self.endpoint(&format!("/admin/channels/{channel_id}")))
                .query(&[
                    ("idempotency_key", idempotency_key),
                    ("cascade", if cascade { "true" } else { "false" }),
                ]),
        )
        .await
    }

    async fn operation(&self, operation_id: &str) -> Result<IndexOperation, IndexStoreError> {
        self.execute(
            self.client
                .get(self.endpoint(&format!("/admin/operations/{operation_id}"))),
        )
        .await
    }

    async fn video_chunk_count(&self, video_id: &str) -> Result<u64, IndexStoreError> {
        let response: CountResponse = self
            .execute(
                self.client
                    .get(self.endpoint(&format!("/admin/videos/{video_id}/chunk-count"))),
            )
            .await?;
        Ok(response.count)
    }

    async fn indexed_video_ids(
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

    async fn channel_stats(&self, channel_id: &str) -> Result<ChunkStats, IndexStoreError> {
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

fn map_status(status: StatusCode, body: String) -> IndexStoreError {
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value.get("message")?.as_str().map(ToOwned::to_owned))
        .unwrap_or(body);
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
