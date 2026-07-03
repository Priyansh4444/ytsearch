use crate::{Chunker, Result, TranscriptSource, VideoMetadataSource};

pub struct VideoIngestor<M, T> {
    metadata: M,
    transcripts: T,
    chunker: Chunker,
}

impl<M, T> VideoIngestor<M, T>
where
    M: VideoMetadataSource + Send + Sync,
    T: TranscriptSource + Send + Sync,
{
    pub fn new(metadata: M, transcripts: T) -> Self {
        Self {
            metadata,
            transcripts,
            chunker: Chunker::five_seconds(),
        }
    }

    pub fn with_chunker(mut self, chunker: Chunker) -> Self {
        self.chunker = chunker;
        self
    }

    pub async fn ingest_video(&self, video_id: &str) -> Result<Vec<crate::TranscriptChunk>> {
        let metadata = self.metadata.metadata_for_video(video_id).await?;
        let transcript = self.transcripts.transcript_for_video(video_id).await?;

        self.chunker.chunks(&metadata, &transcript)
    }
}
