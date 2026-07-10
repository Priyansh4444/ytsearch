#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("video metadata unavailable for {video_id}: {reason}")]
    MetadataUnavailable { video_id: String, reason: String },
    #[error("transcript unavailable for {video_id}: {reason}")]
    TranscriptUnavailable { video_id: String, reason: String },
    #[error("video {video_id} has no usable transcript segments")]
    EmptyTranscript { video_id: String },
    #[error("search sink failed: {0}")]
    Sink(String),
    #[error("transcript chunk source failed: {0}")]
    ChunkSource(String),
}

pub type Result<T> = std::result::Result<T, IngestError>;
