//! `yt-dlp` backed transcript source.
//!
//! This crate is intentionally separate from YouTube metadata and ingestion so the
//! caption extraction implementation can be swapped without touching the rest of
//! the pipeline. It implements [`youtube_ingest::TranscriptSource`] and returns
//! timestamped transcript segments parsed from public `json3` or `vtt` captions.

use async_trait::async_trait;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json::Value;
use std::{env, process::Stdio};
use tokio::{
    process::Command,
    time::{Duration, sleep, timeout},
};
use youtube_ingest::{DurationMs, IngestError, TimestampMs, TranscriptSegment, TranscriptSource};

const YOUTUBE_WATCH_URL_PREFIX: &str = "https://www.youtube.com/watch?v=";
const DEFAULT_YT_DLP_BINARY: &str = "yt-dlp";
const YT_DLP_BINARY_ENV: &str = "YT_DLP_BINARY";
const YT_DLP_COOKIES_FILE_ENV: &str = "YT_DLP_COOKIES_FILE";
const YT_DLP_EXTRACTOR_ARGS_ENV: &str = "YT_DLP_EXTRACTOR_ARGS";
const YT_DLP_USER_AGENT_ENV: &str = "YT_DLP_USER_AGENT";
const CAPTION_LANGUAGES: [&str; 4] = ["en", "en-orig", "a.en", "en-US"];
const CAPTION_EXTENSIONS: [&str; 2] = ["json3", "vtt"];
const CAPTION_FETCH_ATTEMPTS: u8 = 3;
const ERROR_BODY_PREVIEW_BYTES: usize = 240;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const YT_DLP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug)]
pub struct YtDlpTranscriptSource {
    http: HttpClient,
    binary: String,
}

impl YtDlpTranscriptSource {
    pub fn new() -> Self {
        Self::with_binary(
            env::var(YT_DLP_BINARY_ENV).unwrap_or_else(|_| DEFAULT_YT_DLP_BINARY.to_string()),
        )
    }

    pub fn with_binary(binary: impl Into<String>) -> Self {
        Self {
            http: timeout_http_client(),
            binary: binary.into(),
        }
    }

    pub fn with_http_client(http: HttpClient, binary: impl Into<String>) -> Self {
        Self {
            http,
            binary: binary.into(),
        }
    }

    async fn video_json(&self, video_id: &str) -> youtube_ingest::Result<Value> {
        let url = video_url(video_id);
        let mut command = Command::new(&self.binary);
        command
            .arg("--skip-download")
            .arg("--dump-json")
            .arg("--no-warnings");

        if let Ok(cookies_file) = env::var(YT_DLP_COOKIES_FILE_ENV) {
            if !cookies_file.trim().is_empty() {
                command.arg("--cookies").arg(cookies_file);
            }
        }
        if let Ok(extractor_args) = env::var(YT_DLP_EXTRACTOR_ARGS_ENV) {
            if !extractor_args.trim().is_empty() {
                command.arg("--extractor-args").arg(extractor_args);
            }
        }
        if let Ok(user_agent) = env::var(YT_DLP_USER_AGENT_ENV) {
            if !user_agent.trim().is_empty() {
                command.arg("--user-agent").arg(user_agent);
            }
        }

        let output = timeout(
            YT_DLP_TIMEOUT,
            command
                .arg(url)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| transcript_error(video_id, "yt-dlp timed out"))?
        .map_err(|error| transcript_error(video_id, format!("failed to run yt-dlp: {error}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(transcript_error(
                video_id,
                format!("yt-dlp exited with {}: {}", output.status, stderr.trim()),
            ));
        }

        serde_json::from_slice(&output.stdout).map_err(|error| {
            transcript_error(video_id, format!("yt-dlp json parse failed: {error}"))
        })
    }

    async fn fetch_caption(
        &self,
        video_id: &str,
        track: &CaptionTrack,
    ) -> youtube_ingest::Result<String> {
        let mut last_error = None;

        for attempt in 1..=CAPTION_FETCH_ATTEMPTS {
            match self.fetch_caption_once(video_id, track).await {
                Ok(body) => return Ok(body),
                Err(error) => {
                    last_error = Some(error);
                    if attempt < CAPTION_FETCH_ATTEMPTS {
                        sleep(Duration::from_millis(u64::from(attempt) * 500)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| transcript_error(video_id, "caption request failed")))
    }

    async fn fetch_caption_once(
        &self,
        video_id: &str,
        track: &CaptionTrack,
    ) -> youtube_ingest::Result<String> {
        let response = self.http.get(&track.url).send().await.map_err(|error| {
            transcript_error(video_id, format!("caption request failed: {error}"))
        })?;

        let status = response.status();
        let body = response.text().await.map_err(|error| {
            transcript_error(video_id, format!("caption body read failed: {error}"))
        })?;

        if !status.is_success() {
            return Err(transcript_error(
                video_id,
                format!(
                    "caption request returned {status}: {}",
                    preview_error_body(&body)
                ),
            ));
        }

        Ok(body)
    }
}

impl Default for YtDlpTranscriptSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TranscriptSource for YtDlpTranscriptSource {
    async fn transcript_for_video(
        &self,
        video_id: &str,
    ) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
        let video = self.video_json(video_id).await?;
        let track = select_caption_track(&video).ok_or_else(|| {
            transcript_error(
                video_id,
                "yt-dlp found no English subtitles or automatic captions",
            )
        })?;
        tracing::info!(
            video_id,
            extension = %track.extension,
            "selected caption track"
        );
        let body = self.fetch_caption(video_id, &track).await?;

        match track.extension.as_str() {
            "json3" => parse_json3_transcript(video_id, &body),
            "vtt" => parse_vtt_transcript(video_id, &body),
            extension => Err(transcript_error(
                video_id,
                format!("unsupported caption extension {extension}"),
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CaptionTrack {
    url: String,
    extension: String,
}

#[derive(Debug, Deserialize)]
struct Json3Transcript {
    #[serde(default)]
    events: Vec<Json3Event>,
}

#[derive(Debug, Deserialize)]
struct Json3Event {
    #[serde(rename = "tStartMs")]
    start_ms: Option<TimestampMs>,
    #[serde(rename = "dDurationMs")]
    duration_ms: Option<DurationMs>,
    #[serde(default)]
    segs: Vec<Json3Segment>,
}

#[derive(Debug, Deserialize)]
struct Json3Segment {
    utf8: Option<String>,
}

fn select_caption_track(video: &Value) -> Option<CaptionTrack> {
    ["subtitles", "automatic_captions"]
        .into_iter()
        .find_map(|section| select_caption_track_from_section(video.get(section)?))
}

fn select_caption_track_from_section(section: &Value) -> Option<CaptionTrack> {
    for language in CAPTION_LANGUAGES {
        let tracks = section.get(language).and_then(Value::as_array)?;
        for extension in CAPTION_EXTENSIONS {
            if let Some(track) = tracks
                .iter()
                .find_map(|track| caption_track(track, extension))
            {
                return Some(track);
            }
        }
    }

    None
}

fn caption_track(track: &Value, expected_extension: &str) -> Option<CaptionTrack> {
    let extension = track.get("ext")?.as_str()?;
    if extension != expected_extension {
        return None;
    }

    Some(CaptionTrack {
        url: track.get("url")?.as_str()?.to_string(),
        extension: extension.to_string(),
    })
}

fn parse_json3_transcript(
    video_id: &str,
    body: &str,
) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
    let transcript: Json3Transcript = serde_json::from_str(body)
        .map_err(|error| transcript_error(video_id, format!("json3 parse failed: {error}")))?;
    let mut segments = Vec::with_capacity(transcript.events.len());

    for event in transcript.events {
        let Some(start_ms) = event.start_ms else {
            continue;
        };
        let Some(duration_ms) = event.duration_ms else {
            continue;
        };
        let Some(end_ms) = start_ms.checked_add(duration_ms) else {
            continue;
        };

        let text_len = event
            .segs
            .iter()
            .filter_map(|segment| segment.utf8.as_deref())
            .map(str::len)
            .sum();
        let mut text = String::with_capacity(text_len);
        for segment in event.segs {
            if let Some(part) = segment.utf8 {
                text.push_str(&part);
            }
        }

        if has_text(&text) && end_ms > start_ms {
            segments.push(TranscriptSegment {
                start_ms,
                end_ms,
                text,
            });
        }
    }

    if segments.is_empty() {
        return Err(transcript_error(
            video_id,
            "json3 contained no usable captions",
        ));
    }

    Ok(segments)
}

fn parse_vtt_transcript(
    video_id: &str,
    body: &str,
) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
    let mut segments = Vec::new();
    let mut lines = body.lines().peekable();

    while let Some(line) = lines.next() {
        if !line.contains("-->") {
            continue;
        }

        let Some((start, rest)) = line.split_once("-->") else {
            continue;
        };
        let end = rest.split_whitespace().next().unwrap_or_default();
        let Some(start_ms) = parse_vtt_timestamp_ms(start.trim()) else {
            continue;
        };
        let Some(end_ms) = parse_vtt_timestamp_ms(end.trim()) else {
            continue;
        };

        let mut text = String::new();
        while let Some(next) = lines.peek().copied() {
            if next.trim().is_empty() {
                lines.next();
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            append_without_vtt_tags(&mut text, next);
            lines.next();
        }

        if has_text(&text) && end_ms > start_ms {
            segments.push(TranscriptSegment {
                start_ms,
                end_ms,
                text,
            });
        }
    }

    if segments.is_empty() {
        return Err(transcript_error(
            video_id,
            "vtt contained no usable captions",
        ));
    }

    Ok(segments)
}

fn parse_vtt_timestamp_ms(timestamp: &str) -> Option<TimestampMs> {
    let (head, millis) = timestamp.rsplit_once('.')?;
    let millis: u32 = millis.get(..3)?.parse().ok()?;
    let mut parts = head.rsplit(':');
    let seconds: u32 = parts.next()?.parse().ok()?;
    let minutes: u32 = parts.next()?.parse().ok()?;
    let hours: u32 = parts.next().unwrap_or("0").parse().ok()?;

    hours
        .checked_mul(60 * 60 * 1000)?
        .checked_add(minutes.checked_mul(60 * 1000)?)?
        .checked_add(seconds.checked_mul(1000)?)?
        .checked_add(millis)
}

fn append_without_vtt_tags(output: &mut String, input: &str) {
    let mut in_tag = false;
    for character in input.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
}

fn timeout_http_client() -> HttpClient {
    HttpClient::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .expect("caption HTTP client configuration should be valid")
}

fn preview_error_body(body: &str) -> String {
    let mut preview = String::with_capacity(ERROR_BODY_PREVIEW_BYTES.min(body.len()));
    for character in body.chars() {
        if preview.len() + character.len_utf8() > ERROR_BODY_PREVIEW_BYTES {
            preview.push_str("...");
            return preview;
        }
        preview.push(character);
    }
    preview
}

fn video_url(video_id: &str) -> String {
    let mut url = String::with_capacity(YOUTUBE_WATCH_URL_PREFIX.len() + video_id.len());
    url.push_str(YOUTUBE_WATCH_URL_PREFIX);
    url.push_str(video_id);
    url
}

fn transcript_error(video_id: &str, reason: impl Into<String>) -> IngestError {
    IngestError::TranscriptUnavailable {
        video_id: video_id.to_string(),
        reason: reason.into(),
    }
}

fn has_text(text: &str) -> bool {
    text.split_whitespace().next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_json3_manual_caption_before_automatic_caption() {
        let video = serde_json::json!({
            "subtitles": {
                "en": [
                    { "ext": "vtt", "url": "https://example.com/manual.vtt" },
                    { "ext": "json3", "url": "https://example.com/manual.json3" }
                ]
            },
            "automatic_captions": {
                "en": [
                    { "ext": "json3", "url": "https://example.com/auto.json3" }
                ]
            }
        });

        let track = select_caption_track(&video).unwrap();

        assert_eq!(track.extension, "json3");
        assert_eq!(track.url, "https://example.com/manual.json3");
    }

    #[test]
    fn parses_json3_transcript() {
        let body = r#"{
            "events": [
                { "tStartMs": 1000, "dDurationMs": 2000, "segs": [{ "utf8": "hello " }, { "utf8": "world" }] },
                { "tStartMs": 3000, "dDurationMs": 1000, "segs": [{ "utf8": "\n" }] }
            ]
        }"#;

        let segments = parse_json3_transcript("abc", body).unwrap();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms, 1_000);
        assert_eq!(segments[0].end_ms, 3_000);
        assert_eq!(segments[0].text, "hello world");
    }

    #[test]
    fn parses_vtt_transcript() {
        let body = "WEBVTT\n\n00:00:01.000 --> 00:00:03.500\n<v Speaker>hello</v> world\n";

        let segments = parse_vtt_transcript("abc", body).unwrap();

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_ms, 1_000);
        assert_eq!(segments[0].end_ms, 3_500);
        assert_eq!(segments[0].text, "hello world");
    }
}
