use async_trait::async_trait;
use reqwest::{Client as HttpClient, StatusCode, Url, header::CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, fmt, time::Duration};
use thiserror::Error;
use tokio::time::sleep;
use youtube_ingest::{
    ChunkSink, IngestError, TranscriptChunk, TranscriptChunkPage, TranscriptChunkSource,
};
use youtube_search::{
    SearchBackendError, TextSearchQuery as SharedTextSearchQuery, TranscriptSearchBackend,
    YoutuberDocument as SharedYoutuberDocument,
};

#[deprecated(
    since = "0.1.0",
    note = "Typesense support is retained for compatibility. Prefer youtube_meilisearch for new deployments."
)]
pub mod deprecated_notice {}

pub const DEFAULT_COLLECTION: &str = "youtube_transcript_chunks";
pub const YOUTUBERS_COLLECTION: &str = "youtube_youtubers";
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const IMPORT_BATCH_SIZE: usize = 500;
const IMPORT_ATTEMPTS: u8 = 6;
const DELETE_ATTEMPTS: u8 = 3;
const DELETE_BATCH_SIZE: &str = "1000";
const DELETE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CONTROL_PLANE_ATTEMPTS: u8 = 3;
const EXPORT_PAGE_SIZE_MAX: usize = 250;
const EXPORT_QUERY_BY: &str = "text";

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
            http: timeout_http_client(),
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
        self.create_named_collection_if_missing(
            &self.config.collection,
            transcript_chunk_schema(&self.config.collection),
        )
        .await
    }

    pub async fn create_youtubers_collection_if_missing(&self) -> Result<CreateCollectionOutcome> {
        self.create_named_collection_if_missing(
            YOUTUBERS_COLLECTION,
            youtubers_schema(YOUTUBERS_COLLECTION),
        )
        .await
    }

    async fn create_named_collection_if_missing(
        &self,
        name: &str,
        schema: Value,
    ) -> Result<CreateCollectionOutcome> {
        if let Some(body) = self.get_collection(name).await? {
            self.add_missing_schema_fields(name, &body, &schema).await?;
            return Ok(CreateCollectionOutcome::AlreadyExists { body });
        }

        let url = endpoint(&self.config.api_url, "collections")?;
        let response = self
            .http
            .post(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .json(&schema)
            .send()
            .await?;

        match json_response(response).await {
            Ok(response) => Ok(CreateCollectionOutcome::Created(response)),
            Err(TypesenseError::Api { status, body }) if status == StatusCode::CONFLICT => {
                Ok(CreateCollectionOutcome::AlreadyExists { body })
            }
            Err(error) => Err(error),
        }
    }

    async fn add_missing_schema_fields(
        &self,
        name: &str,
        existing_collection: &str,
        desired_schema: &Value,
    ) -> Result<()> {
        let existing: Value = serde_json::from_str(existing_collection)?;
        let missing = missing_schema_fields(&existing, desired_schema);
        if missing.is_empty() {
            return Ok(());
        }

        let path = format!("collections/{name}");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .patch(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .json(&json!({ "fields": missing }))
            .send()
            .await?;
        json_response(response).await.map(|_| ())
    }

    pub async fn get_collection(&self, name: &str) -> Result<Option<String>> {
        let path = format!("collections/{name}");
        let url = endpoint(&self.config.api_url, &path)?;
        let mut response = None;
        for attempt in 1..=CONTROL_PLANE_ATTEMPTS {
            match self
                .http
                .get(url.clone())
                .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
                .send()
                .await
            {
                Ok(result) => {
                    response = Some(result);
                    break;
                }
                Err(error) if error.is_timeout() && attempt < CONTROL_PLANE_ATTEMPTS => {
                    sleep(Duration::from_secs(u64::from(attempt))).await;
                }
                Err(error) => return Err(TypesenseError::Http(error)),
            }
        }
        let response = response.expect("successful response or timeout error");

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        json_response(response)
            .await
            .map(|body| Some(body.to_string()))
    }

    pub async fn upsert_youtuber(&self, youtuber: &YoutuberDocument) -> Result<Value> {
        let path = format!("collections/{YOUTUBERS_COLLECTION}/documents?action=upsert");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .post(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .json(youtuber)
            .send()
            .await?;

        json_response(response).await
    }

    pub async fn delete_youtuber(&self, id: &str) -> Result<Option<Value>> {
        let path = format!("collections/{YOUTUBERS_COLLECTION}/documents/{id}");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .delete(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .send()
            .await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        json_response(response).await.map(Some)
    }

    pub async fn get_youtuber(&self, id: &str) -> Result<Option<YoutuberDocument>> {
        let path = format!("collections/{YOUTUBERS_COLLECTION}/documents/{id}");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .send()
            .await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        json_response(response)
            .await
            .and_then(|body| serde_json::from_value(body).map_err(TypesenseError::Json))
            .map(Some)
    }

    pub async fn delete_chunks_by_youtuber_id(&self, youtuber_id: &str) -> Result<Value> {
        self.delete_documents_by_filter(&format!("youtuber_id:={youtuber_id}"))
            .await
    }

    pub async fn delete_chunks_by_video_id(&self, video_id: &str) -> Result<Value> {
        self.delete_documents_by_filter(&format!("video_id:={video_id}"))
            .await
    }

    pub async fn count_chunks_by_video_id(&self, video_id: &str) -> Result<u64> {
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = endpoint(&self.config.api_url, &path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", "*");
            pairs.append_pair("filter_by", &format!("video_id:={video_id}"));
            pairs.append_pair("per_page", "0");
        }

        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .send()
            .await?;
        let body = json_response(response).await?;
        Ok(body
            .get("found")
            .and_then(Value::as_u64)
            .unwrap_or_default())
    }

    pub async fn count_chunks_by_youtuber_id(&self, youtuber_id: &str) -> Result<u64> {
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = endpoint(&self.config.api_url, &path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", "*");
            pairs.append_pair("filter_by", &format!("youtuber_id:={youtuber_id}"));
            pairs.append_pair("per_page", "0");
        }

        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .send()
            .await?;
        let body = json_response(response).await?;
        Ok(body
            .get("found")
            .and_then(Value::as_u64)
            .unwrap_or_default())
    }

    pub async fn chunk_stats_by_youtuber_id(&self, youtuber_id: &str) -> Result<ChunkStats> {
        let indexed_chunks = self.count_chunks_by_youtuber_id(youtuber_id).await?;
        let indexed_video_ids = self.indexed_video_ids_by_youtuber_id(youtuber_id).await?;
        Ok(ChunkStats {
            indexed_videos: indexed_video_ids.len() as u64,
            indexed_chunks,
        })
    }

    pub async fn chunks_page_by_youtuber_id(
        &self,
        youtuber_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<TranscriptChunkPage> {
        let page_size = limit.clamp(1, EXPORT_PAGE_SIZE_MAX);
        let page = offset / page_size + 1;
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = endpoint(&self.config.api_url, &path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", "*");
            pairs.append_pair("query_by", EXPORT_QUERY_BY);
            pairs.append_pair("filter_by", &typesense_youtuber_id_filter(youtuber_id));
            pairs.append_pair("per_page", &page_size.to_string());
            pairs.append_pair("page", &page.to_string());
        }
        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .send()
            .await?;
        let body = json_response(response).await?;
        let chunks = body
            .get("hits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|hit| hit.get("document").cloned())
            .map(serde_json::from_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(TranscriptChunkPage {
            chunks,
            offset,
            total: body
                .get("found")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
        })
    }

    pub async fn indexed_video_ids_by_youtuber_id(
        &self,
        youtuber_id: &str,
    ) -> Result<HashSet<String>> {
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = endpoint(&self.config.api_url, &path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", "*");
            pairs.append_pair("filter_by", &format!("youtuber_id:={youtuber_id}"));
            pairs.append_pair("facet_by", "video_id");
            pairs.append_pair("max_facet_values", "20000");
            pairs.append_pair("per_page", "0");
        }

        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
            .send()
            .await?;
        let body = json_response(response).await?;
        Ok(body
            .get("facet_counts")
            .and_then(Value::as_array)
            .and_then(|facets| {
                facets.iter().find(|facet| {
                    facet
                        .get("field_name")
                        .and_then(Value::as_str)
                        .is_some_and(|field| field == "video_id")
                })
            })
            .and_then(|facet| facet.get("counts"))
            .and_then(Value::as_array)
            .map(|counts| {
                counts
                    .iter()
                    .filter_map(|count| count.get("value").and_then(Value::as_str))
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn delete_documents_by_filter(&self, filter_by: &str) -> Result<Value> {
        let url = delete_documents_url(&self.config, filter_by)?;

        let mut last_error = None;
        for attempt in 1..=DELETE_ATTEMPTS {
            let response = self
                .http
                .delete(url.clone())
                .header("X-TYPESENSE-API-KEY", &self.config.admin_api_key)
                .timeout(DELETE_TIMEOUT)
                .send()
                .await;

            match response {
                Ok(response) => return json_response(response).await,
                Err(error) if attempt < DELETE_ATTEMPTS && error.is_timeout() => {
                    last_error = Some(error);
                    sleep(Duration::from_millis(u64::from(attempt) * 1000)).await;
                }
                Err(error) => return Err(TypesenseError::Http(error)),
            }
        }

        Err(TypesenseError::Http(
            last_error.expect("delete retry loop should capture an error"),
        ))
    }

    pub async fn import_chunks(&self, chunks: &[TranscriptChunk]) -> Result<TypesenseImport> {
        self.import_chunk_batch_with_retries(chunks).await
    }

    pub async fn import_chunks_batched(
        &self,
        chunks: &[TranscriptChunk],
    ) -> Result<TypesenseImport> {
        let mut body = String::new();
        for batch in chunks.chunks(IMPORT_BATCH_SIZE) {
            let result = self.import_chunk_batch_with_retries(batch).await?;
            body.push_str(&result.body);
            if !body.ends_with('\n') {
                body.push('\n');
            }
        }

        Ok(TypesenseImport { body })
    }

    async fn import_chunk_batch_with_retries(
        &self,
        chunks: &[TranscriptChunk],
    ) -> Result<TypesenseImport> {
        let mut last_error = None;

        for attempt in 1..=IMPORT_ATTEMPTS {
            match self.import_chunk_batch(chunks).await {
                Ok(result) => return Ok(result),
                Err(error) if attempt < IMPORT_ATTEMPTS && error.is_transient() => {
                    last_error = Some(error);
                    sleep(Duration::from_millis(u64::from(attempt) * 500)).await;
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_error.unwrap_or(TypesenseError::RetryExhausted))
    }

    async fn import_chunk_batch(&self, chunks: &[TranscriptChunk]) -> Result<TypesenseImport> {
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

        validate_import_body(&body)?;

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

#[async_trait]
impl TranscriptChunkSource for TypesenseAdminClient {
    async fn chunks_page_by_youtuber_id(
        &self,
        youtuber_id: &str,
        offset: usize,
        limit: usize,
    ) -> youtube_ingest::Result<TranscriptChunkPage> {
        TypesenseAdminClient::chunks_page_by_youtuber_id(self, youtuber_id, offset, limit)
            .await
            .map_err(|error| IngestError::ChunkSource(error.to_string()))
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
            http: timeout_http_client(),
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
        append_search_query_params(&mut url, query);

        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.search_api_key)
            .send()
            .await?;

        json_response(response).await
    }

    pub async fn count_distinct(&self, query: &TextSearchQuery, field: &str) -> Result<u64> {
        let path = format!("collections/{}/documents/search", self.config.collection);
        let mut url = endpoint(&self.config.api_url, &path)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", &query.q);
            pairs.append_pair("query_by", &query.query_by);
            pairs.append_pair("per_page", "0");
            pairs.append_pair("facet_by", field);
            pairs.append_pair("max_facet_values", "100000");
            pairs.append_pair("use_cache", "true");
            pairs.append_pair("cache_ttl", "60");
            if let Some(filter_by) = &query.filter_by {
                pairs.append_pair("filter_by", filter_by);
            }
        }

        let response = self
            .http
            .get(url)
            .header("X-TYPESENSE-API-KEY", &self.config.search_api_key)
            .send()
            .await?;

        let body = json_response(response).await?;
        Ok(body
            .get("facet_counts")
            .and_then(Value::as_array)
            .and_then(|facets| {
                facets.iter().find(|facet| {
                    facet
                        .get("field_name")
                        .and_then(Value::as_str)
                        .is_some_and(|field_name| field_name == field)
                })
            })
            .and_then(|facet| facet.get("counts"))
            .and_then(Value::as_array)
            .map_or(0, |values| values.len() as u64))
    }

    pub async fn list_youtubers(&self) -> Result<Vec<YoutuberDocument>> {
        let path = format!("collections/{YOUTUBERS_COLLECTION}/documents/search");
        const PAGE_SIZE: u64 = 250;
        let mut page = 1_u64;
        let mut channels = Vec::new();

        loop {
            let mut url = endpoint(&self.config.api_url, &path)?;
            {
                let mut pairs = url.query_pairs_mut();
                pairs.append_pair("q", "*");
                pairs.append_pair("query_by", "name,handle");
                pairs.append_pair("per_page", &PAGE_SIZE.to_string());
                pairs.append_pair("page", &page.to_string());
            }

            let response = self
                .http
                .get(url)
                .header("X-TYPESENSE-API-KEY", &self.config.search_api_key)
                .send()
                .await?;
            let body = json_response(response).await?;
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
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(TypesenseError::Json)?,
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
}

#[async_trait]
impl TranscriptSearchBackend for TypesenseSearchClient {
    async fn search(
        &self,
        query: &SharedTextSearchQuery,
    ) -> std::result::Result<Value, SearchBackendError> {
        let query = TextSearchQuery {
            q: query.q.clone(),
            query_by: query.query_by.clone(),
            filter_by: query.filter_by.clone(),
            group_by: query.group_by.clone(),
            group_limit: query.group_limit,
            per_page: query.per_page,
            page: query.page,
            highlight_fields: query.highlight_fields.clone(),
            sort_by: query.sort_by.clone(),
        };
        TypesenseSearchClient::search(self, &query)
            .await
            .map_err(|error| SearchBackendError::new("typesense", error))
    }

    async fn count_distinct(
        &self,
        query: &SharedTextSearchQuery,
        field: &str,
    ) -> std::result::Result<u64, SearchBackendError> {
        let query = TextSearchQuery {
            q: query.q.clone(),
            query_by: query.query_by.clone(),
            filter_by: query.filter_by.clone(),
            group_by: query.group_by.clone(),
            group_limit: query.group_limit,
            per_page: query.per_page,
            page: query.page,
            highlight_fields: query.highlight_fields.clone(),
            sort_by: query.sort_by.clone(),
        };
        TypesenseSearchClient::count_distinct(self, &query, field)
            .await
            .map_err(|error| SearchBackendError::new("typesense", error))
    }

    async fn list_youtubers(
        &self,
    ) -> std::result::Result<Vec<SharedYoutuberDocument>, SearchBackendError> {
        TypesenseSearchClient::list_youtubers(self)
            .await
            .map(|channels| {
                channels
                    .into_iter()
                    .map(|channel| SharedYoutuberDocument {
                        id: channel.id,
                        handle: channel.handle,
                        name: channel.name,
                        channel_id: channel.channel_id,
                        uploads_playlist: channel.uploads_playlist,
                        public_video_count: channel.public_video_count,
                        indexed_video_count: channel.indexed_video_count,
                        indexed_chunk_count: channel.indexed_chunk_count,
                        avatar_url: channel.avatar_url,
                    })
                    .collect()
            })
            .map_err(|error| SearchBackendError::new("typesense", error))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypesenseImport {
    pub body: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChunkStats {
    pub indexed_videos: u64,
    pub indexed_chunks: u64,
}

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
    pub page: Option<u32>,
    pub highlight_fields: Option<String>,
    pub sort_by: Option<String>,
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
    #[error("typesense import failed for {failed} document(s): {sample}")]
    ImportFailures { failed: usize, sample: String },
    #[error("typesense import retry attempts exhausted")]
    RetryExhausted,
}

impl TypesenseError {
    fn is_transient(&self) -> bool {
        match self {
            Self::Http(error) => error.is_timeout() || error.is_connect() || error.is_request(),
            Self::Api { status, .. } => {
                status.is_server_error()
                    || *status == StatusCode::TOO_MANY_REQUESTS
                    || *status == StatusCode::REQUEST_TIMEOUT
            }
            Self::InvalidUrl(_)
            | Self::Json(_)
            | Self::ImportFailures { .. }
            | Self::RetryExhausted => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, TypesenseError>;

fn missing_schema_fields(existing: &Value, desired_schema: &Value) -> Vec<Value> {
    let existing_names = existing
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|field| field.get("name").and_then(Value::as_str))
        .collect::<HashSet<_>>();

    desired_schema
        .get("fields")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|field| {
            field
                .get("name")
                .and_then(Value::as_str)
                // `id` is built into Typesense. It is accepted at collection creation,
                // but Typesense rejects it in a subsequent schema PATCH.
                .is_some_and(|name| name != "id" && !existing_names.contains(name))
        })
        .cloned()
        .collect()
}

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
            { "name": "published_at_timestamp", "type": "int64", "optional": true },
            { "name": "upload_date", "type": "string", "facet": true, "optional": true },
            { "name": "uploader_id", "type": "string", "facet": true }
        ],
        "default_sorting_field": "published_at_timestamp"
    })
}

pub fn youtubers_schema(collection: &str) -> Value {
    json!({
        "name": collection,
        "fields": [
            { "name": "id", "type": "string" },
            { "name": "handle", "type": "string", "facet": true },
            { "name": "name", "type": "string" },
            { "name": "channel_id", "type": "string", "facet": true },
            { "name": "uploads_playlist", "type": "string", "optional": true },
            { "name": "public_video_count", "type": "int32" },
            { "name": "indexed_video_count", "type": "int32" },
            { "name": "indexed_chunk_count", "type": "int32" },
            { "name": "avatar_url", "type": "string", "optional": true }
        ]
    })
}

fn timeout_http_client() -> HttpClient {
    HttpClient::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .expect("typesense HTTP client configuration should be valid")
}

fn endpoint(api_url: &str, path: &str) -> Result<Url> {
    let base = normalized_api_url(api_url)?;
    Url::parse(&format!("{base}/{path}"))
        .map_err(|source| TypesenseError::InvalidUrl(source.to_string()))
}

fn delete_documents_url(config: &TypesenseAdminConfig, filter_by: &str) -> Result<Url> {
    let path = format!("collections/{}/documents", config.collection);
    let mut url = endpoint(&config.api_url, &path)?;
    let mut pairs = url.query_pairs_mut();
    pairs.append_pair("filter_by", filter_by);
    pairs.append_pair("batch_size", DELETE_BATCH_SIZE);
    drop(pairs);
    Ok(url)
}

fn typesense_youtuber_id_filter(youtuber_id: &str) -> String {
    let canonical = youtuber_id.trim_start_matches('@');
    format!("youtuber_id:={canonical} || youtuber_id:=@{canonical}")
}

fn append_search_query_params(url: &mut Url, query: &TextSearchQuery) {
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
    if let Some(page) = query.page {
        pairs.append_pair("page", &page.to_string());
    }
    if let Some(highlight_fields) = &query.highlight_fields {
        pairs.append_pair("highlight_fields", highlight_fields);
    }
    if let Some(sort_by) = &query.sort_by {
        pairs.append_pair("sort_by", sort_by);
    }
    pairs.append_pair(
        "include_fields",
        "id,video_id,title,text,prev_text,next_text,youtuber_handle,youtuber_name,thumbnail,published_at,duration,t_start_ms,t_end_ms",
    );
    pairs.append_pair("use_cache", "true");
    pairs.append_pair("cache_ttl", "60");
}

fn normalized_api_url(api_url: &str) -> Result<String> {
    let api_url = api_url.trim().trim_end_matches('/');
    if api_url.is_empty() {
        return Err(TypesenseError::InvalidUrl(
            "TYPESENSE_API_URL cannot be empty".to_string(),
        ));
    }

    if api_url.starts_with("http://") || api_url.starts_with("https://") {
        Ok(api_url.to_string())
    } else {
        Ok(format!("https://{api_url}"))
    }
}

async fn json_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(TypesenseError::Api { status, body });
    }

    Ok(serde_json::from_str(&body)?)
}

fn validate_import_body(body: &str) -> Result<()> {
    let mut failed = 0usize;
    let mut sample = None;

    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        if value.get("success").and_then(Value::as_bool) == Some(false) {
            failed = failed.saturating_add(1);
            if sample.is_none() {
                sample = Some(line.to_string());
            }
        }
    }

    if failed > 0 {
        return Err(TypesenseError::ImportFailures {
            failed,
            sample: sample.unwrap_or_default(),
        });
    }

    Ok(())
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
    fn schema_has_numeric_publication_sort_field() {
        let schema = transcript_chunk_schema(DEFAULT_COLLECTION);
        let fields = schema["fields"].as_array().expect("schema fields");

        assert!(fields.iter().any(|field| {
            field["name"] == "published_at_timestamp" && field["type"] == "int64"
        }));
        assert_eq!(
            schema["default_sorting_field"],
            Value::String("published_at_timestamp".to_string())
        );
    }

    #[test]
    fn schema_patch_excludes_typesense_builtin_id() {
        let existing = serde_json::json!({
            "fields": [{ "name": "video_id", "type": "string" }]
        });
        let missing =
            missing_schema_fields(&existing, &transcript_chunk_schema(DEFAULT_COLLECTION));
        let missing_names = missing
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect::<Vec<_>>();

        assert!(!missing_names.contains(&"id"));
        assert!(missing_names.contains(&"published_at_timestamp"));
    }

    #[test]
    fn import_batches_multiple_chunks_per_request() {
        assert!(IMPORT_BATCH_SIZE > 1);
    }

    #[test]
    fn validates_typesense_import_failures() {
        let body = r#"{"success":true}
{"success":false,"error":"bad doc"}"#;
        let error = validate_import_body(body).unwrap_err();
        assert!(error.to_string().contains("bad doc"));
    }

    #[test]
    fn search_query_defaults_to_grouped_transcript_search() {
        let query = TextSearchQuery::new("react server components");

        assert_eq!(query.query_by, "text,title,youtuber_name,youtuber_handle");
        assert_eq!(query.group_by.as_deref(), Some("video_id"));
        assert_eq!(query.group_limit, Some(5));
    }

    #[test]
    fn search_query_params_limit_payload_and_enable_cache() {
        let mut url = endpoint(
            "http://localhost:8108",
            "collections/chunks/documents/search",
        )
        .expect("valid url");
        let query = TextSearchQuery::new("react")
            .query_by("text,title")
            .youtuber_handle("@t3dotgg")
            .without_grouping()
            .per_page(240)
            .page(2);

        append_search_query_params(&mut url, &query);
        let pairs = url.query_pairs().collect::<Vec<_>>();

        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "per_page" && value == "240")
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "include_fields" && value.contains("video_id"))
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "use_cache" && value == "true")
        );
        assert!(
            pairs
                .iter()
                .any(|(key, value)| key == "cache_ttl" && value == "60")
        );
    }

    #[test]
    fn endpoint_accepts_host_without_scheme() {
        let url = endpoint("example.a1.typesense.net", "collections").unwrap();

        assert_eq!(url.as_str(), "https://example.a1.typesense.net/collections");
    }

    #[test]
    fn delete_url_uses_bounded_large_batch() {
        let config = TypesenseAdminConfig::new("localhost:8108", "admin-secret");
        let url = delete_documents_url(&config, "youtuber_id:=t3dotgg").expect("delete URL");
        let params = url.query_pairs().collect::<Vec<_>>();

        assert!(
            params
                .iter()
                .any(|(key, value)| { key == "filter_by" && value == "youtuber_id:=t3dotgg" })
        );
        assert!(
            params
                .iter()
                .any(|(key, value)| key == "batch_size" && value == DELETE_BATCH_SIZE)
        );
    }

    #[test]
    fn export_filter_matches_canonical_and_legacy_youtuber_ids() {
        assert_eq!(EXPORT_QUERY_BY, "text");
        assert_eq!(
            typesense_youtuber_id_filter("@t3dotgg"),
            "youtuber_id:=t3dotgg || youtuber_id:=@t3dotgg"
        );
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
