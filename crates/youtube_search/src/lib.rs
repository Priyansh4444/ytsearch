use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const DEFAULT_COLLECTION: &str = "youtube_transcript_chunks";
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
    pub sort_by: Option<String>,
    pub vector_query: Option<String>,
    pub rerank_hybrid_matches: bool,
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
            sort_by: None,
            vector_query: None,
            rerank_hybrid_matches: false,
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

    pub fn sort_by(mut self, sort_by: impl Into<String>) -> Self {
        self.sort_by = Some(sort_by.into());
        self
    }

    pub fn hybrid(mut self, vector_query: impl Into<String>, rerank: bool) -> Self {
        self.vector_query = Some(vector_query.into());
        self.rerank_hybrid_matches = rerank;
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
    async fn count_distinct(
        &self,
        query: &TextSearchQuery,
        field: &str,
    ) -> Result<u64, SearchBackendError>;
    async fn list_youtubers(&self) -> Result<Vec<YoutuberDocument>, SearchBackendError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_defaults_match_the_public_search_contract() {
        let query = TextSearchQuery::new("rust testing");

        assert_eq!(query.q, "rust testing");
        assert_eq!(query.query_by, "text,title,youtuber_name,youtuber_handle");
        assert_eq!(query.group_by.as_deref(), Some("video_id"));
        assert_eq!(query.group_limit, Some(5));
        assert_eq!(query.highlight_fields.as_deref(), Some("text,title"));
    }

    #[test]
    fn query_builder_normalizes_pages_and_can_disable_grouping() {
        let query = TextSearchQuery::new("captions")
            .youtuber_handle("@marimo-team")
            .page(0)
            .per_page(20)
            .sort_by("published_at_timestamp:desc")
            .without_grouping();

        assert_eq!(
            query.filter_by.as_deref(),
            Some("youtuber_handle:=@marimo-team")
        );
        assert_eq!(query.page, Some(1));
        assert_eq!(query.per_page, Some(20));
        assert_eq!(
            query.sort_by.as_deref(),
            Some("published_at_timestamp:desc")
        );
        assert_eq!(query.group_by, None);
        assert_eq!(query.group_limit, None);
    }

    #[test]
    fn backend_errors_include_actionable_context() {
        let error = SearchBackendError::new("typesense", "connection refused");
        assert_eq!(
            error.to_string(),
            "typesense backend failed: connection refused"
        );
    }
}
