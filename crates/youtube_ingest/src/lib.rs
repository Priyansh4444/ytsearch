mod chunker;
mod error;
mod model;
mod ports;

pub use chunker::Chunker;
pub use error::{IngestError, Result};
pub use model::{
    DurationMs, TimestampMs, TranscriptChunk, TranscriptSegment, VideoMetadata, Youtuber,
};
pub use ports::{TranscriptSource, VideoMetadataSource};
