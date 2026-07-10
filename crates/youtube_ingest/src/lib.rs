mod chunker;
mod error;
mod ingestor;
mod model;
mod ports;

pub use chunker::{Chunker, DEFAULT_CHUNK_SECONDS};
pub use error::{IngestError, Result};
pub use ingestor::VideoIngestor;
pub use model::{
    DurationMs, TimestampMs, TranscriptChunk, TranscriptSegment, VideoMetadata, Youtuber,
};
pub use ports::{
    ChunkSink, EmbeddingSource, TranscriptChunkPage, TranscriptChunkSource, TranscriptSource,
    VideoMetadataSource,
};
