use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type TimestampMs = u32;
pub type DurationMs = u32;

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
    pub duration_ms: Option<DurationMs>,
    pub thumbnail: Option<String>,
    pub published_at: Option<String>,
    pub upload_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct TranscriptSegment {
    pub start_ms: TimestampMs,
    pub end_ms: TimestampMs,
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
    pub duration: Option<DurationMs>,
    pub t_start_ms: TimestampMs,
    pub t_end_ms: TimestampMs,
    pub thumbnail: Option<String>,
    pub upload_date: Option<String>,
    pub published_at: Option<String>,
    pub published_at_timestamp: Option<i64>,
    pub uploader_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
