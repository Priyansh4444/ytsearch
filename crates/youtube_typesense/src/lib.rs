use async_trait::async_trait;
use reqwest::{Client as HttpClient, StatusCode, Url, header::CONTENT_TYPE};
use serde_json::{Value, json};
use std::fmt;
use thiserror::Error;
use youtube_ingest::{ChunkSink, IngestError, TranscriptChunk};

pub const DEFAULT_COLLECTION: &str = "youtube_transcript_chunks";

#[derive(Clone, Eq, PartialEq)]
pub struct TypesenseAdminConfig {
    pub api_url: String,
    pub admin_api_key: String,
    pub collection: String,
}

impl TypesenseAdminConfig {
    pub fn new(api_url: impl Into<String>, admin_api_key: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
            admin_api_key: admin_api_key.into(),
            collection: DEFAULT_COLLECTION.to_string(),
        }
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }
}

impl fmt::Debug for TypesenseAdminConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypesenseAdminConfig")
            .field("api_url", &self.api_url)
            .field("admin_api_key", &"<redacted>")
            .field("collection", &self.collection)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TypesenseSearchConfig {
    pub api_url: String,
    pub search_api_key: String,
    pub collection: String,
}

impl TypesenseSearchConfig {
    pub fn new(api_url: impl Into<String>, search_api_key: impl Into<String>) -> Self {
        Self {
            api_url: api_url.into(),
            search_api_key: search_api_key.into(),
            collection: DEFAULT_COLLECTION.to_string(),
        }
    }

    pub fn with_collection(mut self, collection: impl Into<String>) -> Self {
        self.collection = collection.into();
        self
    }
}

impl fmt::Debug for TypesenseSearchConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypesenseSearchConfig")
            .field("api_url", &self.api_url)
            .field("search_api_key", &"<redacted>")
            .field("collection", &self.collection)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct TypesenseAdminClient {
    http: HttpClient,
    config: TypesenseAdminConfig,
}

impl TypesenseAdminClient {
    pub fn new(config: TypesenseAdminConfig) -> Self {
        Self {
            http: HttpClient::new(),
            config,
        }
    }

    pub fn with_http_client(http: HttpClient, config: TypesenseAdminConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &TypesenseAdminConfig {
        &self.config
    }

    pub async fn create_collection(&self) -> Result<Value> {
        let url = endpoint(&self.config.api_url, "collections")?;
        let response = self
            .http
            .post(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .json(&transcript_chunk_schema(&self.config.collection))
            .send()
            .await?;

        json_response(response).await
    }

    pub async fn create_collection_if_missing(&self) -> Result<CreateCollectionOutcome> {
        match self.create_collection().await {
            Ok(response) => Ok(CreateCollectionOutcome::Created(response)),
            Err(TypesenseError::Api { status, body }) if status == StatusCode::CONFLICT => {
                Ok(CreateCollectionOutcome::AlreadyExists { body })
            }
            Err(error) => Err(error),
        }
    }

    pub async fn import_chunks(&self, chunks: &[TranscriptChunk]) -> Result<TypesenseImport> {
        let mut body = Vec::with_capacity(chunks.len().saturating_mul(512));
        for chunk in chunks {
            serde_json::to_writer(&mut body, chunk)?;
            body.push(b'\n');
        }

        let path = format!(
            "collections/{}/documents/import?action=upsert",
            self.config.collection
        );
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .post(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .header(CONTENT_TYPE, "text/plain")
            .body(body)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(TypesenseError::Api { status, body });
        }

        Ok(TypesenseImport { body })
    }
}

#[async_trait]
impl ChunkSink for TypesenseAdminClient {
    async fn upsert_chunks(&self, chunks: &[TranscriptChunk]) -> youtube_ingest::Result<()> {
        self.import_chunks(chunks)
            .await
            .map(|_| ())
            .map_err(|error| IngestError::Sink(error.to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct TypesenseSearchClient {
    http: HttpClient,
    config: TypesenseSearchConfig,
}

impl TypesenseSearchClient {
    pub fn new(config: TypesenseSearchConfig) -> Self {
        Self {
            http: HttpClient::new(),
            config,
        }
    }

    pub fn with_http_client(http: HttpClient, config: TypesenseSearchConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &TypesenseSearchConfig {
        &self.config
    }

    pub async fn search(&self, query: &TextSearchQuery) -> Result<Value> {
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = endpoint(&self.config.api_url, &path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", &query.q);
            pairs.append_pair("query_by", &query.query_by);

            if let Some(filter_by) = &query.filter_by {
                pairs.append_pair("filter_by", filter_by);
            }
            if let Some(group_by) = &query.group_by {
                pairs.append_pair("group_by", group_by);
            }
            if let Some(group_limit) = query.group_limit {
                pairs.append_pair("group_limit", &group_limit.to_string());
            }
            if let Some(per_page) = query.per_page {
                pairs.append_pair("per_page", &per_page.to_string());
            }
        }

        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.search_api_key)
            .send()
            .await?;

        json_response(response).await
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypesenseImport {
    pub body: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CreateCollectionOutcome {
    Created(Value),
    AlreadyExists { body: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSearchQuery {
    pub q: String,
    pub query_by: String,
    pub filter_by: Option<String>,
    pub group_by: Option<String>,
    pub group_limit: Option<u8>,
    pub per_page: Option<u8>,
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
}

#[derive(Debug, Error)]
pub enum TypesenseError {
    #[error("invalid typesense url: {0}")]
    InvalidUrl(String),
    #[error("typesense http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("typesense json failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("typesense returned {status}: {body}")]
    Api { status: StatusCode, body: String },
}

pub type Result<T> = std::result::Result<T, TypesenseError>;

pub fn transcript_chunk_schema(collection: &str) -> Value {
    json!({
        "name": collection,
        "fields": [
            { "name": "id", "type": "string" },
            { "name": "youtuber_id", "type": "string", "facet": true },
            { "name": "youtuber_handle", "type": "string", "facet": true },
            { "name": "youtuber_name", "type": "string" },
            { "name": "channel_id", "type": "string", "facet": true },
            { "name": "video_id", "type": "string", "facet": true },
            { "name": "title", "type": "string" },
            { "name": "text", "type": "string" },
            { "name": "prev_text", "type": "string", "optional": true },
            { "name": "next_text", "type": "string", "optional": true },
            { "name": "sequence", "type": "int32" },
            { "name": "chapter", "type": "string", "optional": true },
            { "name": "chapter_index", "type": "int32", "optional": true },
            { "name": "duration", "type": "int32", "optional": true },
            { "name": "t_start_ms", "type": "int32" },
            { "name": "t_end_ms", "type": "int32" },
            { "name": "thumbnail", "type": "string", "optional": true },
            { "name": "published_at", "type": "string", "facet": true, "optional": true },
            { "name": "upload_date", "type": "string", "facet": true, "optional": true },
            { "name": "uploader_id", "type": "string", "facet": true }
        ],
        "default_sorting_field": "t_start_ms"
    })
}

fn endpoint(api_url: &str, path: &str) -> Result<Url> {
    let base = api_url.trim_end_matches('/');
    Url::parse(&format!("{base}/{path}"))
        .map_err(|source| TypesenseError::InvalidUrl(source.to_string()))
}

async fn json_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(TypesenseError::Api { status, body });
    }

    Ok(serde_json::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_config_debug_redacts_api_key() {
        let config = TypesenseAdminConfig::new("http://localhost:8108", "admin-secret");
        let debug = format!("{config:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("admin-secret"));
    }

    #[test]
    fn search_config_debug_redacts_api_key() {
        let config = TypesenseSearchConfig::new("http://localhost:8108", "search-secret");
        let debug = format!("{config:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("search-secret"));
    }

    #[test]
    fn schema_uses_document_fields_from_ingest_model() {
        let schema = transcript_chunk_schema(DEFAULT_COLLECTION);
        let fields = schema["fields"].as_array().unwrap();

        assert!(fields.iter().any(|field| field["name"] == "text"));
        assert!(fields.iter().any(|field| field["name"] == "video_id"));
        assert!(fields.iter().any(|field| field["name"] == "t_start_ms"));
    }

    #[test]
    fn search_query_defaults_to_grouped_transcript_search() {
        let query = TextSearchQuery::new("react server components");

        assert_eq!(query.query_by, "text,title,youtuber_name,youtuber_handle");
        assert_eq!(query.group_by.as_deref(), Some("video_id"));
        assert_eq!(query.group_limit, Some(5));
    }

    #[test]
    fn youtuber_filter_uses_handle_facet() {
        let query = TextSearchQuery::new("rust").youtuber_handle("@t3dotgg");

        assert_eq!(
            query.filter_by.as_deref(),
            Some("youtuber_handle:=@t3dotgg")
        );
    }
}
