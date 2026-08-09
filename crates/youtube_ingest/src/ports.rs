use crate::{Result, TranscriptSegment, VideoMetadata};
use async_trait::async_trait;

#[async_trait]
pub trait VideoMetadataSource {
    async fn metadata_for_video(&self, video_id: &str) -> Result<VideoMetadata>;
}

#[async_trait]
pub trait TranscriptSource {
    async fn transcript_for_video(&self, video_id: &str) -> Result<Vec<TranscriptSegment>>;
}
