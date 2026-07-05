use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const DEFAULT_COLLECTION: &str = "youtube_transcript_chunks";
pub const YOUTUBERS_COLLECTION: &str = "youtube_youtubers";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct YoutuberDocument {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub channel_id: String,
    pub uploads_playlist: Option<String>,
    pub public_video_count: u32,
    pub indexed_video_count: u32,
    pub indexed_chunk_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSearchQuery {
    pub q: String,
    pub query_by: String,
    pub filter_by: Option<String>,
    pub group_by: Option<String>,
    pub group_limit: Option<u8>,
    pub per_page: Option<u8>,
    pub page: Option<u32>,
    pub highlight_fields: Option<String>,
}

impl TextSearchQuery {
    pub fn new(q: impl Into<String>) -> Self {
        Self {
            q: q.into(),
            query_by: "text,title,youtuber_name,youtuber_handle".to_string(),
            filter_by: None,
            group_by: Some("video_id".to_string()),
            group_limit: Some(5),
            per_page: None,
            page: None,
            highlight_fields: Some("text,title".to_string()),
        }
    }

    pub fn query_by(mut self, query_by: impl Into<String>) -> Self {
        self.query_by = query_by.into();
        self
    }

    pub fn filter_by(mut self, filter_by: impl Into<String>) -> Self {
        self.filter_by = Some(filter_by.into());
        self
    }

    pub fn youtuber_handle(mut self, handle: impl AsRef<str>) -> Self {
        self.filter_by = Some(format!("youtuber_handle:={}", handle.as_ref()));
        self
    }

    pub fn group_by(mut self, group_by: impl Into<String>) -> Self {
        self.group_by = Some(group_by.into());
        self
    }

    pub fn without_grouping(mut self) -> Self {
        self.group_by = None;
        self.group_limit = None;
        self
    }

    pub fn group_limit(mut self, group_limit: u8) -> Self {
        self.group_limit = Some(group_limit);
        self
    }

    pub fn per_page(mut self, per_page: u8) -> Self {
        self.per_page = Some(per_page);
        self
    }

    pub fn page(mut self, page: u32) -> Self {
        self.page = Some(page.max(1));
        self
    }

    pub fn highlight_fields(mut self, highlight_fields: impl Into<String>) -> Self {
        self.highlight_fields = Some(highlight_fields.into());
        self
    }
}

#[derive(Debug, Error)]
#[error("{backend} backend failed: {message}")]
pub struct SearchBackendError {
    backend: &'static str,
    message: String,
}

impl SearchBackendError {
    pub fn new(backend: &'static str, source: impl ToString) -> Self {
        Self {
            backend,
            message: source.to_string(),
        }
    }
}

#[async_trait]
pub trait TranscriptSearchBackend: Send + Sync {
    async fn search(&self, query: &TextSearchQuery) -> Result<Value, SearchBackendError>;
    async fn list_youtubers(&self) -> Result<Vec<YoutuberDocument>, SearchBackendError>;
}
