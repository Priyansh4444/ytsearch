use reqwest::{Client, StatusCode, Url};
use serde_json::Value;
use std::{fmt, time::Duration};
use thiserror::Error;
pub use youtube_index_store::ChannelDocument;

pub const DEFAULT_COLLECTION: &str = "youtube_transcript_chunks";
const CHANNEL_COLLECTION: &str = "youtube_youtubers";
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSearchQuery {
    q: String,
    query_by: String,
    filter_by: Option<String>,
    group_by: Option<String>,
    group_limit: Option<u8>,
    per_page: Option<u8>,
    page: Option<u32>,
    offset: Option<u32>,
    limit: Option<u8>,
    highlight_fields: Option<String>,
    sort_by: Option<String>,
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
            offset: None,
            limit: None,
            highlight_fields: Some("text,title".to_string()),
            sort_by: None,
        }
    }

    pub fn query_by(mut self, value: impl Into<String>) -> Self {
        self.query_by = value.into();
        self
    }

    pub fn filter_by(mut self, value: impl Into<String>) -> Self {
        self.filter_by = Some(value.into());
        self
    }

    pub fn youtuber_handle(mut self, handle: impl AsRef<str>) -> Self {
        self.filter_by = Some(format!("youtuber_handle:={}", handle.as_ref()));
        self
    }

    pub fn group_by(mut self, value: impl Into<String>) -> Self {
        self.group_by = Some(value.into());
        self
    }

    pub fn without_grouping(mut self) -> Self {
        self.group_by = None;
        self.group_limit = None;
        self
    }

    pub fn group_limit(mut self, value: u8) -> Self {
        self.group_limit = Some(value);
        self
    }

    pub fn per_page(mut self, value: u8) -> Self {
        self.per_page = Some(value);
        self.limit = None;
        self
    }

    pub fn page(mut self, value: u32) -> Self {
        self.page = Some(value.max(1));
        self.offset = None;
        self
    }

    pub fn window(mut self, offset: u32, limit: u8) -> Self {
        self.offset = Some(offset);
        self.limit = Some(limit);
        self.per_page = None;
        self.page = None;
        self
    }

    pub fn highlight_fields(mut self, value: impl Into<String>) -> Self {
        self.highlight_fields = Some(value.into());
        self
    }

    pub fn sort_by(mut self, value: impl Into<String>) -> Self {
        self.sort_by = Some(value.into());
        self
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct SearchClientConfig {
    base_url: String,
    api_key: String,
    collection: String,
}

impl SearchClientConfig {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            collection: DEFAULT_COLLECTION.to_string(),
        }
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }
}

impl fmt::Debug for SearchClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchClientConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("collection", &self.collection)
            .finish()
    }
}

#[derive(Clone)]
pub struct SearchClient {
    http: Client,
    config: SearchClientConfig,
}

impl SearchClient {
    pub fn new(config: SearchClientConfig) -> Result<Self, SearchError> {
        let http = Client::builder().timeout(HTTP_TIMEOUT).build()?;
        validate_base_url(&config.base_url)?;
        if config.api_key.is_empty() {
            return Err(SearchError::InvalidInput(
                "Raw search API key must not be empty".to_string(),
            ));
        }
        Ok(Self { http, config })
    }

    pub async fn search(&self, query: &TextSearchQuery) -> Result<Value, SearchError> {
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = endpoint(&self.config.base_url, &path)?;
        append_search_query_params(&mut url, query);
        self.get_json(url).await
    }

    pub async fn list_youtubers(&self) -> Result<Vec<ChannelDocument>, SearchError> {
        const PAGE_SIZE: u64 = 250;
        let path = format!("collections/{CHANNEL_COLLECTION}/documents/search");
        let mut page = 1_u64;
        let mut channels = Vec::new();
        loop {
            let mut url = endpoint(&self.config.base_url, &path)?;
            url.query_pairs_mut()
                .append_pair("q", "*")
                .append_pair("query_by", "name,handle")
                .append_pair("per_page", &PAGE_SIZE.to_string())
                .append_pair("page", &page.to_string());
            let body = self.get_json(url).await?;
            let hits = body
                .get("hits")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let hit_count = hits.len() as u64;
            channels.extend(
                hits.into_iter()
                    .filter_map(|hit| hit.get("document").cloned())
                    .map(serde_json::from_value)
                    .collect::<Result<Vec<_>, _>>()?,
            );
            let total = body
                .get("found")
                .and_then(Value::as_u64)
                .unwrap_or(channels.len() as u64);
            if hit_count == 0 || channels.len() as u64 >= total {
                return Ok(channels);
            }
            page = page.saturating_add(1);
        }
    }

    async fn get_json(&self, url: Url) -> Result<Value, SearchError> {
        // Temporary expand-migrate-contract bridge: the deployed immutable
        // reader still authenticates with the former compatibility header,
        // while the mutable reader accepts the Raw header. Remove the former
        // header immediately after both production readers are mutable.
        let response = self
            .http
            .get(url)
            .header("x-raw-search-key", &self.config.api_key)
            .header("x-typesense-api-key", &self.config.api_key)
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(SearchError::Api { status, body });
        }
        Ok(serde_json::from_str(&body)?)
    }
}

fn append_search_query_params(url: &mut Url, query: &TextSearchQuery) {
    let mut pairs = url.query_pairs_mut();
    pairs
        .append_pair("q", &query.q)
        .append_pair("query_by", &query.query_by)
        .append_pair("query_by_weights", &search_field_weights(&query.query_by))
        .append_pair("text_match_type", "sum_score")
        .append_pair("prioritize_exact_match", "true")
        .append_pair("prioritize_token_position", "true")
        .append_pair("typo_tokens_threshold", "5")
        .append_pair("split_join_tokens", "always");
    if let Some(value) = &query.filter_by {
        pairs.append_pair("filter_by", value);
    }
    if let Some(value) = &query.group_by {
        pairs.append_pair("group_by", value);
    }
    if let Some(value) = query.group_limit {
        pairs.append_pair("group_limit", &value.to_string());
    }
    if let Some(value) = query.per_page {
        pairs.append_pair("per_page", &value.to_string());
    }
    if let Some(value) = query.page {
        pairs.append_pair("page", &value.to_string());
    }
    if let Some(value) = query.offset {
        pairs.append_pair("offset", &value.to_string());
    }
    if let Some(value) = query.limit {
        pairs.append_pair("limit", &value.to_string());
    }
    if let Some(value) = &query.highlight_fields {
        pairs.append_pair("highlight_fields", value);
    }
    if let Some(value) = &query.sort_by {
        pairs.append_pair("sort_by", value);
    }
    pairs
        .append_pair(
            "include_fields",
            "id,video_id,title,text,prev_text,next_text,youtuber_handle,youtuber_name,thumbnail,published_at,duration,t_start_ms,t_end_ms",
        )
        .append_pair("use_cache", "true")
        .append_pair("cache_ttl", "60");
}

fn search_field_weights(query_by: &str) -> String {
    query_by
        .split(',')
        .map(|field| match field.trim() {
            "title" => "6",
            "text" => "4",
            "youtuber_name" | "youtuber_handle" => "2",
            _ => "1",
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn endpoint(base_url: &str, path: &str) -> Result<Url, SearchError> {
    let base_url = base_url.trim().trim_end_matches('/');
    validate_base_url(base_url)?;
    Url::parse(&format!("{base_url}/{path}"))
        .map_err(|error| SearchError::InvalidInput(error.to_string()))
}

fn validate_base_url(base_url: &str) -> Result<(), SearchError> {
    let url = Url::parse(base_url).map_err(|error| SearchError::InvalidInput(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SearchError::InvalidInput(
            "Raw search URL must use http or https".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("invalid Raw search input: {0}")]
    InvalidInput(String),
    #[error("Raw search request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Raw search response was invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Raw search returned {status}: {body}")]
    Api { status: StatusCode, body: String },
}

#[cfg(test)]
mod tests;
