use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_CHUNK_SECONDS: u64 = 5;

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
}

pub type Result<T> = std::result::Result<T, IngestError>;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct Youtuber {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub channel_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct VideoMetadata {
    pub video_id: String,
    pub title: String,
    pub youtuber: Youtuber,
    pub duration_ms: Option<u64>,
    pub thumbnail: Option<String>,
    pub published_at: Option<String>,
    pub upload_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct TranscriptSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct TranscriptChunk {
    pub id: String,
    pub youtuber_id: String,
    pub youtuber_handle: String,
    pub youtuber_name: String,
    pub channel_id: String,
    pub video_id: String,
    pub title: String,
    pub text: String,
    pub prev_text: Option<String>,
    pub next_text: Option<String>,
    pub sequence: u32,
    pub chapter: Option<String>,
    pub chapter_index: Option<u32>,
    pub duration: Option<u64>,
    pub t_start_ms: u64,
    pub t_end_ms: u64,
    pub thumbnail: Option<String>,
    pub upload_date: Option<String>,
    pub published_at: Option<String>,
    pub uploader_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunker {
    window_ms: u64,
}

impl Chunker {
    pub fn five_seconds() -> Self {
        Self::new(DEFAULT_CHUNK_SECONDS)
    }

    pub fn new(window_seconds: u64) -> Self {
        let safe_seconds = window_seconds.max(1);
        Self {
            window_ms: safe_seconds * 1000,
        }
    }

    pub fn chunks(
        &self,
        metadata: &VideoMetadata,
        transcript: &[TranscriptSegment],
    ) -> Result<Vec<TranscriptChunk>> {
        let clean_segments = clean_segments(transcript);
        if clean_segments.is_empty() {
            return Err(IngestError::EmptyTranscript {
                video_id: metadata.video_id.clone(),
            });
        }

        let grouped_text = self.group_segments(&clean_segments);
        let mut chunks = Vec::with_capacity(grouped_text.len());

        for (sequence, group) in grouped_text.iter().enumerate() {
            let prev_text = sequence
                .checked_sub(1)
                .and_then(|index| grouped_text.get(index))
                .map(|prev| prev.text.clone());
            let next_text = grouped_text.get(sequence + 1).map(|next| next.text.clone());

            chunks.push(TranscriptChunk {
                id: format!("{}_{}", metadata.video_id, sequence),
                youtuber_id: metadata.youtuber.id.clone(),
                youtuber_handle: metadata.youtuber.handle.clone(),
                youtuber_name: metadata.youtuber.name.clone(),
                channel_id: metadata.youtuber.channel_id.clone(),
                video_id: metadata.video_id.clone(),
                title: metadata.title.clone(),
                text: group.text.clone(),
                prev_text,
                next_text,
                sequence: sequence as u32,
                chapter: None,
                chapter_index: None,
                duration: metadata.duration_ms,
                t_start_ms: group.start_ms,
                t_end_ms: group.end_ms,
                thumbnail: metadata.thumbnail.clone(),
                upload_date: metadata.upload_date.clone(),
                published_at: metadata.published_at.clone(),
                uploader_id: metadata.youtuber.handle.clone(),
                embedding: None,
                extra: BTreeMap::new(),
            });
        }

        Ok(chunks)
    }

    fn group_segments(&self, segments: &[TranscriptSegment]) -> Vec<GroupedText> {
        let mut groups = Vec::new();
        let mut current: Option<GroupedText> = None;

        for segment in segments {
            let Some(group) = &mut current else {
                current = Some(GroupedText::from_segment(segment));
                continue;
            };

            let segment_fits_window =
                segment.end_ms.saturating_sub(group.start_ms) <= self.window_ms;
            if segment_fits_window {
                group.push(segment);
                continue;
            }

            groups.push(group.clone());
            current = Some(GroupedText::from_segment(segment));
        }

        if let Some(group) = current {
            groups.push(group);
        }

        groups
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GroupedText {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

impl GroupedText {
    fn from_segment(segment: &TranscriptSegment) -> Self {
        Self {
            start_ms: segment.start_ms,
            end_ms: segment.end_ms,
            text: segment.text.clone(),
        }
    }

    fn push(&mut self, segment: &TranscriptSegment) {
        self.end_ms = self.end_ms.max(segment.end_ms);
        self.text.push(' ');
        self.text.push_str(&segment.text);
    }
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
pub trait EmbeddingSource {
    async fn embed_chunks(&self, chunks: &mut [TranscriptChunk]) -> Result<()>;
}

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

    pub async fn ingest_video(&self, video_id: &str) -> Result<Vec<TranscriptChunk>> {
        let metadata = self.metadata.metadata_for_video(video_id).await?;
        let transcript = self.transcripts.transcript_for_video(video_id).await?;
        self.chunker.chunks(&metadata, &transcript)
    }
}

fn clean_segments(transcript: &[TranscriptSegment]) -> Vec<TranscriptSegment> {
    transcript
        .iter()
        .filter_map(|segment| {
            let text = clean_text(&segment.text);
            let has_text = !text.is_empty();
            let has_time_range = segment.end_ms > segment.start_ms;

            if has_text && has_time_range {
                Some(TranscriptSegment {
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms,
                    text,
                })
            } else {
                None
            }
        })
        .collect()
}

fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> VideoMetadata {
        VideoMetadata {
            video_id: "abc123".to_string(),
            title: "A useful video".to_string(),
            youtuber: Youtuber {
                id: "t3dotgg".to_string(),
                handle: "@t3dotgg".to_string(),
                name: "Theo - t3.gg".to_string(),
                channel_id: "channel-1".to_string(),
            },
            duration_ms: Some(12_000),
            thumbnail: Some("https://example.com/thumb.jpg".to_string()),
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            upload_date: Some("20260101".to_string()),
        }
    }

    #[test]
    fn builds_five_second_chunks_with_context() {
        let transcript = vec![
            TranscriptSegment {
                start_ms: 0,
                end_ms: 2_000,
                text: "hello".to_string(),
            },
            TranscriptSegment {
                start_ms: 2_000,
                end_ms: 4_900,
                text: "world".to_string(),
            },
            TranscriptSegment {
                start_ms: 5_000,
                end_ms: 8_000,
                text: "second chunk".to_string(),
            },
        ];

        let chunks = Chunker::five_seconds()
            .chunks(&metadata(), &transcript)
            .unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].id, "abc123_0");
        assert_eq!(chunks[0].text, "hello world");
        assert_eq!(chunks[0].next_text.as_deref(), Some("second chunk"));
        assert_eq!(chunks[1].prev_text.as_deref(), Some("hello world"));
        assert_eq!(chunks[1].t_start_ms, 5_000);
    }

    #[test]
    fn rejects_empty_transcripts_clearly() {
        let transcript = vec![TranscriptSegment {
            start_ms: 0,
            end_ms: 1_000,
            text: "   ".to_string(),
        }];

        let error = Chunker::five_seconds()
            .chunks(&metadata(), &transcript)
            .unwrap_err();

        assert!(matches!(error, IngestError::EmptyTranscript { .. }));
        assert_eq!(
            error.to_string(),
            "video abc123 has no usable transcript segments"
        );
    }
}
