use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU16;
use thiserror::Error;
use youtube_ingest::TranscriptChunk;

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackendReadiness {
    pub backend: String,
    pub ready: bool,
    pub generation: Option<String>,
    pub last_sequence: Option<u64>,
    pub acknowledged_readers: u8,
}

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
        cascade: bool,
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
    use std::{collections::BTreeMap, mem::size_of};

    fn chunk(video_id: &str) -> TranscriptChunk {
        TranscriptChunk {
            id: format!("{video_id}-0"),
            youtuber_id: "channel".to_string(),
            youtuber_handle: "@channel".to_string(),
            youtuber_name: "Channel".to_string(),
            channel_id: "UC123".to_string(),
            video_id: video_id.to_string(),
            title: "Title".to_string(),
            text: "Transcript".to_string(),
            prev_text: None,
            next_text: None,
            sequence: 0,
            chapter: None,
            chapter_index: None,
            duration: None,
            t_start_ms: 0,
            t_end_ms: 1,
            thumbnail: None,
            upload_date: None,
            published_at: None,
            published_at_timestamp: None,
            uploader_id: "channel".to_string(),
            embedding: None,
            extra: BTreeMap::new(),
        }
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
