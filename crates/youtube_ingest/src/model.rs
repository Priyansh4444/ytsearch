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
    pub(crate) id: String,
    pub(crate) youtuber_id: String,
    pub(crate) youtuber_handle: String,
    pub(crate) youtuber_name: String,
    pub(crate) channel_id: String,
    pub(crate) video_id: String,
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) prev_text: Option<String>,
    pub(crate) next_text: Option<String>,
    pub(crate) sequence: u32,
    pub(crate) chapter: Option<String>,
    pub(crate) chapter_index: Option<u32>,
    pub(crate) duration: Option<DurationMs>,
    pub(crate) t_start_ms: TimestampMs,
    pub(crate) t_end_ms: TimestampMs,
    pub(crate) thumbnail: Option<String>,
    pub(crate) upload_date: Option<String>,
    pub(crate) published_at: Option<String>,
    pub(crate) published_at_timestamp: Option<i64>,
    pub(crate) uploader_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) embedding: Option<Vec<f32>>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, serde_json::Value>,
}

impl TranscriptChunk {
    pub fn video_id(&self) -> &str {
        &self.video_id
    }

    pub fn youtuber_id(&self) -> &str {
        &self.youtuber_id
    }
}
