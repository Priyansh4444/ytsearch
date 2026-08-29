use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU16,
};
use thiserror::Error;
use youtube_ingest::TranscriptChunk;

#[cfg(feature = "http-client")]
mod http;
#[cfg(feature = "http-client")]
pub use http::{RawIndexAdminClient, RawIndexAdminConfig};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChannelDocument {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub channel_id: String,
    pub uploads_playlist: Option<String>,
    pub public_video_count: u32,
    pub indexed_video_count: u32,
    pub indexed_chunk_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelDeletionScope {
    ChannelOnly,
    ChannelAndVideos,
}

impl ChannelDeletionScope {
    pub const fn from_cascade(cascade: bool) -> Self {
        if cascade {
            Self::ChannelAndVideos
        } else {
            Self::ChannelOnly
        }
    }

    pub const fn cascades_videos(self) -> bool {
        matches!(self, Self::ChannelAndVideos)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackendReadiness {
    pub backend: String,
    pub ready: bool,
    pub generation: Option<String>,
    pub last_sequence: Option<u64>,
    pub acknowledged_readers: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_api_version: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
}

pub const RAW_PUBLICATION_API_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RawGenerationCompatibility {
    pub generation_format: String,
    pub corpus_identity: String,
    pub base_chunks_blake3: String,
    pub base_channels_blake3: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawPublicationStatus {
    Prepared,
    ManifestAccepted,
    Activated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawGenerationFile {
    pub bytes: u64,
    pub blake3: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawGenerationManifest {
    pub schema_version: u8,
    pub documents: u32,
    pub videos: u32,
    pub channels: u32,
    pub terms: u32,
    pub blocks: u32,
    pub average_document_tokens: f32,
    pub max_view_count: u64,
    pub max_like_count: u64,
    pub max_comment_count: u64,
    pub files: BTreeMap<String, RawGenerationFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawPublicationJob {
    pub job_id: String,
    pub target_sequence: u64,
    pub snapshot_bytes: u64,
    pub snapshot_blake3: String,
    pub compatibility: RawGenerationCompatibility,
    pub status: RawPublicationStatus,
    pub completed_files: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawGenerationUpload {
    pub target_sequence: u64,
    pub compatibility: RawGenerationCompatibility,
    pub manifest: RawGenerationManifest,
}

pub const RAW_GENERATION_FILES: [&str; 11] = [
    "channels.json",
    "videos.json",
    "lexicon.bin",
    "postings.bin",
    "positions.bin",
    "positions.full.bin",
    "documents.blocks",
    "documents.meta",
    "fuzzy.bin",
    "bigrams.text.bin",
    "bigrams.title.bin",
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum IndexOperationState {
    Accepted,
    Durable,
    Published,
    Replicated,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexOperation {
    pub id: String,
    pub sequence: Option<u64>,
    pub state: IndexOperationState,
    pub acknowledged_readers: u8,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChunkStats {
    pub indexed_videos: u64,
    pub indexed_chunks: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VideoIdPage {
    pub video_ids: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Error)]
pub enum IndexStoreError {
    #[error("invalid index administration input: {0}")]
    InvalidInput(String),
    #[error("index administration conflict: {0}")]
    Conflict(String),
    #[error("{backend} index administration is unavailable: {message}")]
    Unavailable {
        backend: &'static str,
        message: String,
    },
    #[error("{backend} index administration failed: {message}")]
    Internal {
        backend: &'static str,
        message: String,
    },
}

impl IndexStoreError {
    pub fn unavailable(backend: &'static str, source: impl ToString) -> Self {
        Self::Unavailable {
            backend,
            message: source.to_string(),
        }
    }

    pub fn internal(backend: &'static str, source: impl ToString) -> Self {
        Self::Internal {
            backend,
            message: source.to_string(),
        }
    }
}

/// A validated, allocation-free view of one complete video replacement.
#[derive(Clone, Copy, Debug)]
pub struct VideoReplacement<'a> {
    video_id: &'a str,
    chunks: &'a [TranscriptChunk],
}

impl<'a> VideoReplacement<'a> {
    pub fn new(video_id: &'a str, chunks: &'a [TranscriptChunk]) -> Result<Self, IndexStoreError> {
        if video_id.is_empty() {
            return Err(IndexStoreError::InvalidInput(
                "video_id must not be empty".to_string(),
            ));
        }
        if chunks.is_empty() {
            return Err(IndexStoreError::InvalidInput(
                "a replacement must contain at least one chunk".to_string(),
            ));
        }
        if chunks.iter().any(|chunk| chunk.video_id() != video_id) {
            return Err(IndexStoreError::InvalidInput(format!(
                "every replacement chunk must belong to video {video_id}"
            )));
        }

        Ok(Self { video_id, chunks })
    }

    pub fn video_id(self) -> &'a str {
        self.video_id
    }

    pub fn chunks(self) -> &'a [TranscriptChunk] {
        self.chunks
    }
}

/// Backend-neutral control plane used by indexers and repair jobs.
///
/// Mutations must be idempotent for a stable `idempotency_key`. Returning an
/// operation does not imply replica visibility; callers must inspect `state`
/// and `acknowledged_readers` when they require read-after-write guarantees.
/// The write contract lives outside the latency-sensitive query crates so its
/// dependencies and state cannot enter the search hot path.
#[async_trait]
pub trait SearchIndexAdmin: Send + Sync {
    async fn readiness(&self) -> Result<BackendReadiness, IndexStoreError>;

    async fn publish(&self) -> Result<BackendReadiness, IndexStoreError> {
        self.readiness().await
    }

    async fn channel(&self, _channel_id: &str) -> Result<Option<ChannelDocument>, IndexStoreError> {
        Err(IndexStoreError::Unavailable {
            backend: "index",
            message: "channel lookup is not implemented by this backend".to_string(),
        })
    }

    async fn replace_video(
        &self,
        replacement: VideoReplacement<'_>,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError>;

    async fn delete_video(
        &self,
        video_id: &str,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError>;

    async fn upsert_channel(
        &self,
        channel: &ChannelDocument,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError>;

    async fn delete_channel(
        &self,
        channel_id: &str,
        scope: ChannelDeletionScope,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError>;

    async fn operation(&self, operation_id: &str) -> Result<IndexOperation, IndexStoreError>;

    async fn video_chunk_count(&self, video_id: &str) -> Result<u64, IndexStoreError>;

    async fn indexed_video_ids(
        &self,
        channel_id: &str,
        cursor: Option<&str>,
        limit: NonZeroU16,
    ) -> Result<VideoIdPage, IndexStoreError>;

    async fn channel_stats(&self, channel_id: &str) -> Result<ChunkStats, IndexStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    fn chunk(video_id: &str) -> TranscriptChunk {
        serde_json::from_value(serde_json::json!({
            "id": format!("{video_id}-0"),
            "youtuber_id": "channel",
            "youtuber_handle": "@channel",
            "youtuber_name": "Channel",
            "channel_id": "UC123",
            "video_id": video_id,
            "title": "Title",
            "text": "Transcript",
            "sequence": 0,
            "t_start_ms": 0,
            "t_end_ms": 1,
            "uploader_id": "channel"
        }))
        .unwrap()
    }

    #[test]
    fn compact_control_values_stay_inline() {
        assert_eq!(size_of::<IndexOperationState>(), 1);
        assert_eq!(size_of::<ChunkStats>(), 16);
    }

    #[test]
    fn replacement_borrows_validated_chunks_without_copying() {
        let chunks = [chunk("video")];
        let replacement = VideoReplacement::new("video", &chunks).unwrap();

        assert_eq!(replacement.video_id(), "video");
        assert!(std::ptr::eq(replacement.chunks(), &chunks));
    }

    #[test]
    fn replacement_rejects_empty_and_mixed_inputs() {
        assert!(matches!(
            VideoReplacement::new("video", &[]),
            Err(IndexStoreError::InvalidInput(_))
        ));
        let chunks = [chunk("other")];
        assert!(matches!(
            VideoReplacement::new("video", &chunks),
            Err(IndexStoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn errors_preserve_backend_context() {
        let error = IndexStoreError::unavailable("raw", "generation missing");
        assert_eq!(
            error.to_string(),
            "raw index administration is unavailable: generation missing"
        );
    }
}
