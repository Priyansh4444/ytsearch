use async_trait::async_trait;
use reqwest::{Client as HttpClient, StatusCode, Url, header::CONTENT_TYPE};
use serde_json::{Value, json};
use std::fmt;
use thiserror::Error;
use youtube_ingest::{ChunkSink, IngestError, TranscriptChunk};

pub const DEFAULT_COLLECTION: &str = "youtube_transcript_chunks";

#[derive(Clone, Eq, PartialEq)]
pub struct TypesenseConfig {
    pub base_url: String,
    pub api_key: String,
    pub collection: String,
}

impl TypesenseConfig {
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

impl fmt::Debug for TypesenseConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypesenseConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("collection", &self.collection)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct TypesenseClient {
    http: HttpClient,
    config: TypesenseConfig,
}

impl TypesenseClient {
    pub fn new(config: TypesenseConfig) -> Self {
        Self {
            http: HttpClient::new(),
            config,
        }
    }

    pub fn with_http_client(http: HttpClient, config: TypesenseConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &TypesenseConfig {
        &self.config
    }

    pub async fn create_collection(&self) -> Result<Value> {
        let url = self.endpoint("collections")?;
        let response = self
            .http
            .post(url)
            .header("X-TYPESENSE-API-KEY", &self.config.api_key)
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
        let url = self.endpoint(&path)?;
        let response = self
            .http
            .post(url)
            .header("X-TYPESENSE-API-KEY", &self.config.api_key)
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

    pub async fn search(&self, query: &TextSearchQuery) -> Result<Value> {
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = self.endpoint(&path)?;
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
            .header("X-TYPESENSE-API-KEY", &self.config.api_key)
            .send()
            .await?;

        json_response(response).await
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        let base = self.config.base_url.trim_end_matches('/');
        Url::parse(&format!("{base}/{path}"))
            .map_err(|source| TypesenseError::InvalidUrl(source.to_string()))
    }
}

#[async_trait]
impl ChunkSink for TypesenseClient {
    async fn upsert_chunks(&self, chunks: &[TranscriptChunk]) -> youtube_ingest::Result<()> {
        self.import_chunks(chunks)
            .await
            .map(|_| ())
            .map_err(|error| IngestError::Sink(error.to_string()))
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
    fn config_debug_redacts_api_key() {
        let config = TypesenseConfig::new("http://localhost:8108", "secret-key");
        let debug = format!("{config:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("secret-key"));
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
}
