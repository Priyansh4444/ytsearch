use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use youtube_client::{ListResource, Page, Query, YouTubeClient};
use youtube_ingest::{
    DurationMs, IngestError, TranscriptSegment, TranscriptSource, VideoMetadata,
    VideoMetadataSource, Youtuber,
};

#[derive(Clone, Debug)]
pub struct YouTubeMetadataSource {
    client: YouTubeClient,
}

impl YouTubeMetadataSource {
    pub fn new(client: YouTubeClient) -> Self {
        Self { client }
    }

    pub fn client(&self) -> &YouTubeClient {
        &self.client
    }
}

#[async_trait]
impl VideoMetadataSource for YouTubeMetadataSource {
    async fn metadata_for_video(&self, video_id: &str) -> youtube_ingest::Result<VideoMetadata> {
        let video_page = self
            .client
            .list(ListResource::Videos)
            .query(
                Query::new()
                    .part(["snippet", "contentDetails"])
                    .ids([video_id])
                    .fields("items(id,snippet(title,channelId,channelTitle,publishedAt,thumbnails),contentDetails(duration))"),
            )
            .page()
            .await
            .map_err(|error| metadata_error(video_id, format!("videos.list failed: {error}")))?;

        let video = first_item(&video_page)
            .ok_or_else(|| metadata_error(video_id, "videos.list returned no items"))?;

        let channel_id = required_string(video, "/snippet/channelId", video_id)?;
        let channel_title = optional_string(video, "/snippet/channelTitle");
        let published_at = optional_string(video, "/snippet/publishedAt");

        let channel_page = self
            .client
            .list(ListResource::Channels)
            .query(
                Query::new()
                    .part(["snippet", "brandingSettings"])
                    .ids([channel_id.as_str()])
                    .fields("items(id,snippet(title,customUrl),brandingSettings/channel/title)"),
            )
            .page()
            .await
            .map_err(|error| metadata_error(video_id, format!("channels.list failed: {error}")))?;

        let channel = first_item(&channel_page);
        let handle = channel
            .and_then(|item| optional_string(item, "/snippet/customUrl"))
            .unwrap_or_else(|| channel_id.clone());
        let name = channel
            .and_then(|item| optional_string(item, "/snippet/title"))
            .or(channel_title)
            .unwrap_or_else(|| channel_id.clone());

        Ok(VideoMetadata {
            video_id: required_string(video, "/id", video_id)?,
            title: required_string(video, "/snippet/title", video_id)?,
            youtuber: Youtuber {
                id: stable_youtuber_id(&handle, &channel_id),
                handle,
                name,
                channel_id,
            },
            duration_ms: optional_string(video, "/contentDetails/duration")
                .and_then(|duration| parse_youtube_duration_ms(&duration)),
            thumbnail: best_thumbnail(video.pointer("/snippet/thumbnails")),
            published_at: published_at.clone(),
            upload_date: published_at.as_deref().and_then(compact_upload_date),
        })
    }
}

#[derive(Default)]
pub struct FallbackMetadataSource {
    sources: Vec<Box<dyn VideoMetadataSource + Send + Sync>>,
}

impl FallbackMetadataSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<S>(&mut self, source: S)
    where
        S: VideoMetadataSource + Send + Sync + 'static,
    {
        self.sources.push(Box::new(source));
    }

    pub fn with_source<S>(mut self, source: S) -> Self
    where
        S: VideoMetadataSource + Send + Sync + 'static,
    {
        self.push(source);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }
}

#[async_trait]
impl VideoMetadataSource for FallbackMetadataSource {
    async fn metadata_for_video(&self, video_id: &str) -> youtube_ingest::Result<VideoMetadata> {
        let mut reasons = Vec::new();

        for source in &self.sources {
            match source.metadata_for_video(video_id).await {
                Ok(metadata) => return Ok(metadata),
                Err(error) => reasons.push(error.to_string()),
            }
        }

        let reason = if reasons.is_empty() {
            "no metadata sources configured".to_string()
        } else {
            reasons.join("; ")
        };

        Err(metadata_error(video_id, reason))
    }
}

#[derive(Default)]
pub struct FallbackTranscriptSource {
    sources: Vec<Box<dyn TranscriptSource + Send + Sync>>,
}

impl FallbackTranscriptSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<S>(&mut self, source: S)
    where
        S: TranscriptSource + Send + Sync + 'static,
    {
        self.sources.push(Box::new(source));
    }

    pub fn with_source<S>(mut self, source: S) -> Self
    where
        S: TranscriptSource + Send + Sync + 'static,
    {
        self.push(source);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }
}

#[async_trait]
impl TranscriptSource for FallbackTranscriptSource {
    async fn transcript_for_video(
        &self,
        video_id: &str,
    ) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
        let mut reasons = Vec::new();

        for source in &self.sources {
            match source.transcript_for_video(video_id).await {
                Ok(transcript) if transcript.is_empty() => {
                    reasons.push("transcript source returned no segments".to_string());
                }
                Ok(transcript) => return Ok(transcript),
                Err(error) => reasons.push(error.to_string()),
            }
        }

        let reason = if reasons.is_empty() {
            "no transcript sources configured".to_string()
        } else {
            reasons.join("; ")
        };

        Err(IngestError::TranscriptUnavailable {
            video_id: video_id.to_string(),
            reason,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryTranscriptSource {
    transcripts: BTreeMap<String, Vec<TranscriptSegment>>,
}

impl InMemoryTranscriptSource {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        mut self,
        video_id: impl Into<String>,
        transcript: Vec<TranscriptSegment>,
    ) -> Self {
        self.transcripts.insert(video_id.into(), transcript);
        self
    }
}

#[async_trait]
impl TranscriptSource for InMemoryTranscriptSource {
    async fn transcript_for_video(
        &self,
        video_id: &str,
    ) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
        self.transcripts
            .get(video_id)
            .cloned()
            .ok_or_else(|| IngestError::TranscriptUnavailable {
                video_id: video_id.to_string(),
                reason: "in-memory transcript not found".to_string(),
            })
    }
}

fn first_item(page: &Page<Value>) -> Option<&Value> {
    page.items.first()
}

fn required_string(value: &Value, pointer: &str, video_id: &str) -> youtube_ingest::Result<String> {
    optional_string(value, pointer).ok_or_else(|| {
        metadata_error(
            video_id,
            format!("expected string field at json pointer {pointer}"),
        )
    })
}

fn optional_string(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer)?.as_str().map(ToOwned::to_owned)
}

fn metadata_error(video_id: &str, reason: impl Into<String>) -> IngestError {
    IngestError::MetadataUnavailable {
        video_id: video_id.to_string(),
        reason: reason.into(),
    }
}

fn best_thumbnail(thumbnails: Option<&Value>) -> Option<String> {
    let thumbnails = thumbnails?.as_object()?;
    for quality in ["maxres", "standard", "high", "medium", "default"] {
        if let Some(url) = thumbnails
            .get(quality)
            .and_then(|thumbnail| thumbnail.get("url"))
            .and_then(Value::as_str)
        {
            return Some(url.to_string());
        }
    }

    thumbnails
        .values()
        .find_map(|thumbnail| thumbnail.get("url").and_then(Value::as_str))
        .map(ToOwned::to_owned)
}

fn stable_youtuber_id(handle: &str, channel_id: &str) -> String {
    let source = handle.strip_prefix('@').unwrap_or(handle);
    let source = if source.is_empty() {
        channel_id
    } else {
        source
    };
    let mut id = String::with_capacity(source.len());
    let mut previous_was_separator = false;

    for byte in source.bytes() {
        if byte.is_ascii_alphanumeric() {
            id.push(char::from(byte.to_ascii_lowercase()));
            previous_was_separator = false;
        } else if !previous_was_separator && !id.is_empty() {
            id.push('-');
            previous_was_separator = true;
        }
    }

    if id.ends_with('-') {
        id.pop();
    }
    id
}

fn compact_upload_date(published_at: &str) -> Option<String> {
    let date = published_at.get(..10)?;
    let bytes = date.as_bytes();
    let is_date = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 4 && *index != 7)
            .all(|(_, byte)| byte.is_ascii_digit());

    if !is_date {
        return None;
    }

    let mut compact = String::with_capacity(8);
    compact.push_str(&date[..4]);
    compact.push_str(&date[5..7]);
    compact.push_str(&date[8..10]);
    Some(compact)
}

fn parse_youtube_duration_ms(duration: &str) -> Option<DurationMs> {
    let value = duration.strip_prefix("PT")?;
    let mut amount: u32 = 0;
    let mut has_amount = false;
    let mut millis: DurationMs = 0;

    for byte in value.bytes() {
        if byte.is_ascii_digit() {
            has_amount = true;
            amount = amount
                .checked_mul(10)?
                .checked_add(u32::from(byte - b'0'))?;
            continue;
        }

        if !has_amount {
            return None;
        }

        let unit_millis = match byte {
            b'H' => 60 * 60 * 1000,
            b'M' => 60 * 1000,
            b'S' => 1000,
            _ => return None,
        };
        millis = millis.checked_add(amount.checked_mul(unit_millis)?)?;
        amount = 0;
        has_amount = false;
    }

    (!has_amount).then_some(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_youtube_duration() {
        assert_eq!(parse_youtube_duration_ms("PT1H2M3S"), Some(3_723_000));
        assert_eq!(parse_youtube_duration_ms("PT4M5S"), Some(245_000));
        assert_eq!(parse_youtube_duration_ms("PT30S"), Some(30_000));
    }

    #[test]
    fn derives_upload_date_from_published_at() {
        assert_eq!(
            compact_upload_date("2026-01-02T03:04:05Z").as_deref(),
            Some("20260102")
        );
    }

    #[test]
    fn normalizes_youtuber_id_from_handle() {
        assert_eq!(stable_youtuber_id("@T3 Dot GG", "fallback"), "t3-dot-gg");
        assert_eq!(stable_youtuber_id("", "UC123"), "uc123");
    }

    #[tokio::test]
    async fn in_memory_transcript_source_returns_clear_missing_error() {
        let source = InMemoryTranscriptSource::new();
        let error = source.transcript_for_video("missing").await.unwrap_err();

        assert!(matches!(error, IngestError::TranscriptUnavailable { .. }));
    }
}
