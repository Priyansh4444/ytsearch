use async_trait::async_trait;
use serde_json::Value;
use youtube_client::{ListResource, Page, Query, YouTubeClient};
use youtube_ingest::{IngestError, VideoMetadata, VideoMetadataSource, Youtuber};
use youtube_time::parse_duration_ms as parse_youtube_duration_ms;

#[derive(Clone, Debug)]
pub struct YouTubeMetadataSource {
    client: YouTubeClient,
}

impl YouTubeMetadataSource {
    pub fn new(client: YouTubeClient) -> Self {
        Self { client }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    enum MockMetadataResponse {
        Success(Box<VideoMetadata>),
        Failure(&'static str),
    }

    struct MockMetadataSource {
        name: &'static str,
        response: MockMetadataResponse,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl MockMetadataSource {
        fn new(
            name: &'static str,
            response: MockMetadataResponse,
            calls: Arc<Mutex<Vec<String>>>,
        ) -> Self {
            Self {
                name,
                response,
                calls,
            }
        }
    }

    #[async_trait]
    impl VideoMetadataSource for MockMetadataSource {
        async fn metadata_for_video(
            &self,
            video_id: &str,
        ) -> youtube_ingest::Result<VideoMetadata> {
            self.calls
                .lock()
                .expect("mock call log lock")
                .push(self.name.to_string());
            match &self.response {
                MockMetadataResponse::Success(metadata) => Ok((**metadata).clone()),
                MockMetadataResponse::Failure(reason) => Err(IngestError::MetadataUnavailable {
                    video_id: video_id.to_string(),
                    reason: (*reason).to_string(),
                }),
            }
        }
    }

    fn metadata(video_id: &str) -> VideoMetadata {
        VideoMetadata {
            video_id: video_id.to_string(),
            title: "Recovered metadata".to_string(),
            youtuber: Youtuber {
                id: "test".to_string(),
                handle: "@test".to_string(),
                name: "Test".to_string(),
                channel_id: "UCtest".to_string(),
            },
            duration_ms: Some(42_000),
            thumbnail: None,
            published_at: Some("2026-07-21T00:00:00Z".to_string()),
            upload_date: Some("20260721".to_string()),
        }
    }

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
    async fn metadata_fallback_uses_sources_in_order_and_stops_after_success() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let expected = metadata("video-1");
        let source = FallbackMetadataSource::new()
            .with_source(MockMetadataSource::new(
                "primary",
                MockMetadataResponse::Failure("primary unavailable"),
                Arc::clone(&calls),
            ))
            .with_source(MockMetadataSource::new(
                "fallback",
                MockMetadataResponse::Success(Box::new(expected.clone())),
                Arc::clone(&calls),
            ))
            .with_source(MockMetadataSource::new(
                "should-not-run",
                MockMetadataResponse::Failure("unexpected call"),
                Arc::clone(&calls),
            ));

        let actual = source.metadata_for_video("video-1").await.unwrap();

        assert_eq!(actual, expected);
        assert_eq!(
            *calls.lock().expect("mock call log lock"),
            ["primary", "fallback"]
        );
    }

    #[tokio::test]
    async fn metadata_fallback_preserves_every_failure_reason() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let source = FallbackMetadataSource::new()
            .with_source(MockMetadataSource::new(
                "yt-dlp",
                MockMetadataResponse::Failure("captions metadata missing"),
                Arc::clone(&calls),
            ))
            .with_source(MockMetadataSource::new(
                "data-api",
                MockMetadataResponse::Failure("quota exhausted"),
                Arc::clone(&calls),
            ));

        let error = source.metadata_for_video("video-2").await.unwrap_err();
        let message = error.to_string();

        assert!(message.contains("captions metadata missing"));
        assert!(message.contains("quota exhausted"));
        assert_eq!(
            *calls.lock().expect("mock call log lock"),
            ["yt-dlp", "data-api"]
        );
    }

    #[tokio::test]
    async fn empty_metadata_fallback_returns_an_actionable_error() {
        let error = FallbackMetadataSource::new()
            .metadata_for_video("video-3")
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "video metadata unavailable for video-3: no metadata sources configured"
        );
    }
}
