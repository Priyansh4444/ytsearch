use crate::{Result, TranscriptChunk, TranscriptSegment, VideoMetadata};
use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptChunkPage {
    pub chunks: Vec<TranscriptChunk>,
    pub offset: usize,
    pub total: usize,
}

#[async_trait]
pub trait VideoMetadataSource {
    async fn metadata_for_video(&self, video_id: &str) -> Result<VideoMetadata>;
}

#[async_trait]
pub trait TranscriptSource {
    async fn transcript_for_video(&self, video_id: &str) -> Result<Vec<TranscriptSegment>>;
}

#[async_trait]
pub trait ChunkSink {
    async fn upsert_chunks(&self, chunks: &[TranscriptChunk]) -> Result<()>;
}

#[async_trait]
pub trait TranscriptChunkSource {
    async fn chunks_page_by_youtuber_id(
        &self,
        youtuber_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<TranscriptChunkPage>;
}

#[async_trait]
pub trait EmbeddingSource {
    async fn embed_chunks(&self, chunks: &mut [TranscriptChunk]) -> Result<()>;
}
