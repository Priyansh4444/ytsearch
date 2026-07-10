use async_trait::async_trait;
use reqwest::{Client as HttpClient, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, fmt, time::Duration};
use thiserror::Error;
use tokio::time::sleep;
use youtube_ingest::{
    ChunkSink, IngestError, TranscriptChunk, TranscriptChunkPage, TranscriptChunkSource,
};
use youtube_search::{
    DEFAULT_COLLECTION, SearchBackendError, TextSearchQuery as SharedTextSearchQuery,
    TranscriptSearchBackend, YOUTUBERS_COLLECTION, YoutuberDocument as SharedYoutuberDocument,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const IMPORT_BATCH_SIZE: usize = 500;
const IMPORT_ATTEMPTS: u8 = 6;
const DELETE_ATTEMPTS: u8 = 3;
const TASK_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TASK_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Eq, PartialEq)]
pub struct MeilisearchAdminConfig {
    pub api_url: String,
    pub admin_api_key: String,
    pub collection: String,
}

impl MeilisearchAdminConfig {
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

impl fmt::Debug for MeilisearchAdminConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MeilisearchAdminConfig")
            .field("api_url", &self.api_url)
            .field("admin_api_key", &"<redacted>")
            .field("collection", &self.collection)
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct MeilisearchSearchConfig {
    pub api_url: String,
    pub search_api_key: String,
    pub collection: String,
}

impl MeilisearchSearchConfig {
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

impl fmt::Debug for MeilisearchSearchConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MeilisearchSearchConfig")
            .field("api_url", &self.api_url)
            .field("search_api_key", &"<redacted>")
            .field("collection", &self.collection)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct MeilisearchAdminClient {
    http: HttpClient,
    config: MeilisearchAdminConfig,
}

impl MeilisearchAdminClient {
    pub fn new(config: MeilisearchAdminConfig) -> Self {
        Self {
            http: timeout_http_client(),
            config,
        }
    }

    pub fn with_http_client(http: HttpClient, config: MeilisearchAdminConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &MeilisearchAdminConfig {
        &self.config
    }

    pub async fn create_collection(&self) -> Result<Value> {
        let url = endpoint(&self.config.api_url, "indexes")?;
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&json!({
                "uid": self.config.collection,
                "primaryKey": "id",
            }))
            .send()
            .await?;

        let body = json_response(response).await?;
        self.configure_index(
            &self.config.collection,
            &transcript_chunk_schema(&self.config.collection),
        )
        .await?;
        Ok(body)
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
            self.configure_index(name, &schema).await?;
            return Ok(CreateCollectionOutcome::AlreadyExists { body });
        }

        let url = endpoint(&self.config.api_url, "indexes")?;
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&json!({
                "uid": name,
                "primaryKey": "id",
            }))
            .send()
            .await?;

        match json_response(response).await {
            Ok(response) => {
                self.configure_index(name, &schema).await?;
                Ok(CreateCollectionOutcome::Created(response))
            }
            Err(MeilisearchError::Api { status, body })
                if status == StatusCode::CONFLICT
                    || (status == StatusCode::BAD_REQUEST
                        && body.contains("index_already_exists")) =>
            {
                self.configure_index(name, &schema).await?;
                Ok(CreateCollectionOutcome::AlreadyExists { body })
            }
            Err(error) => Err(error),
        }
    }

    async fn configure_index(&self, name: &str, schema: &Value) -> Result<()> {
        let url = endpoint(&self.config.api_url, &format!("indexes/{name}/settings"))?;
        let response = self
            .http
            .patch(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&meili_settings_from_schema(schema))
            .send()
            .await?;
        json_response(response).await.map(|_| ())
    }

    pub async fn get_collection(&self, name: &str) -> Result<Option<String>> {
        let path = format!("indexes/{name}");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.config.admin_api_key)
            .send()
            .await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        json_response(response)
            .await
            .map(|body| Some(body.to_string()))
    }

    pub async fn upsert_youtuber(&self, youtuber: &YoutuberDocument) -> Result<Value> {
        let path = format!("indexes/{YOUTUBERS_COLLECTION}/documents");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .put(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&[youtuber])
            .send()
            .await?;

        json_response(response).await
    }

    pub async fn delete_youtuber(&self, id: &str) -> Result<Option<Value>> {
        let path = format!("indexes/{YOUTUBERS_COLLECTION}/documents/{id}");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .delete(url)
            .bearer_auth(&self.config.admin_api_key)
            .send()
            .await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        json_response(response).await.map(Some)
    }

    pub async fn get_youtuber(&self, id: &str) -> Result<Option<YoutuberDocument>> {
        let path = format!("indexes/{YOUTUBERS_COLLECTION}/documents/{id}");
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .get(url)
            .bearer_auth(&self.config.admin_api_key)
            .send()
            .await?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        json_response(response)
            .await
            .and_then(|body| serde_json::from_value(body).map_err(MeilisearchError::Json))
            .map(Some)
    }

    pub async fn delete_chunks_by_youtuber_id(&self, youtuber_id: &str) -> Result<Value> {
        self.delete_documents_by_meili_filter(&youtuber_id_filter(youtuber_id))
            .await
    }

    pub async fn delete_chunks_by_video_id(&self, video_id: &str) -> Result<Value> {
        self.delete_documents_by_filter(&format!("video_id:={video_id}"))
            .await
    }

    pub async fn count_chunks_by_video_id(&self, video_id: &str) -> Result<u64> {
        let path = format!("indexes/{}/search", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;

        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&json!({
                "q": "",
                "filter": format!("video_id = {video_id:?}"),
                "limit": 0,
            }))
            .send()
            .await?;
        let body = json_response(response).await?;
        Ok(body
            .get("estimatedTotalHits")
            .and_then(Value::as_u64)
            .unwrap_or_default())
    }

    pub async fn count_chunks_by_youtuber_id(&self, youtuber_id: &str) -> Result<u64> {
        let path = format!("indexes/{}/search", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;

        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&json!({
                "q": "",
                "filter": youtuber_id_filter(youtuber_id),
                "limit": 0,
            }))
            .send()
            .await?;
        let body = json_response(response).await?;
        Ok(body
            .get("estimatedTotalHits")
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
        let path = format!("indexes/{}/documents/fetch", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&json!({
                "filter": youtuber_id_filter(youtuber_id),
                "offset": offset,
                "limit": limit,
            }))
            .send()
            .await?;
        let body = json_response(response).await?;
        let chunks = body
            .get("results")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();

        Ok(TranscriptChunkPage {
            chunks,
            offset: body
                .get("offset")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            total: body
                .get("total")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
        })
    }

    pub async fn indexed_video_ids_by_youtuber_id(
        &self,
        youtuber_id: &str,
    ) -> Result<HashSet<String>> {
        let path = format!("indexes/{}/search", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(&json!({
                "q": "",
                "filter": youtuber_id_filter(youtuber_id),
                "attributesToRetrieve": ["video_id"],
                "distinct": "video_id",
                "limit": 20_000,
            }))
            .send()
            .await?;
        let body = json_response(response).await?;
        Ok(body
            .get("hits")
            .and_then(Value::as_array)
            .map(|hits| {
                hits.iter()
                    .filter_map(|hit| hit.get("video_id").and_then(Value::as_str))
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn delete_documents_by_filter(&self, filter_by: &str) -> Result<Value> {
        self.delete_documents_by_meili_filter(&filter_to_meili(filter_by))
            .await
    }

    async fn delete_documents_by_meili_filter(&self, filter: &str) -> Result<Value> {
        let path = format!("indexes/{}/documents/delete", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;

        let mut last_error = None;
        for attempt in 1..=DELETE_ATTEMPTS {
            let response = self
                .http
                .post(url.clone())
                .bearer_auth(&self.config.admin_api_key)
                .json(&json!({ "filter": filter }))
                .send()
                .await;

            match response {
                Ok(response) => return json_response(response).await,
                Err(error) if attempt < DELETE_ATTEMPTS && error.is_timeout() => {
                    last_error = Some(error);
                    sleep(Duration::from_millis(u64::from(attempt) * 1000)).await;
                }
                Err(error) => return Err(MeilisearchError::Http(error)),
            }
        }

        Err(MeilisearchError::Http(
            last_error.expect("delete retry loop should capture an error"),
        ))
    }

    pub async fn import_chunks(&self, chunks: &[TranscriptChunk]) -> Result<MeilisearchImport> {
        self.import_chunk_batch_with_retries(chunks).await
    }

    pub async fn import_chunks_batched(
        &self,
        chunks: &[TranscriptChunk],
    ) -> Result<MeilisearchImport> {
        let mut body = String::new();
        for batch in chunks.chunks(IMPORT_BATCH_SIZE) {
            let result = self.import_chunk_batch_with_retries(batch).await?;
            body.push_str(&result.body);
            if !body.ends_with('\n') {
                body.push('\n');
            }
        }

        Ok(MeilisearchImport { body })
    }

    async fn import_chunk_batch_with_retries(
        &self,
        chunks: &[TranscriptChunk],
    ) -> Result<MeilisearchImport> {
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

        Err(last_error.unwrap_or(MeilisearchError::RetryExhausted))
    }

    async fn import_chunk_batch(&self, chunks: &[TranscriptChunk]) -> Result<MeilisearchImport> {
        let path = format!("indexes/{}/documents", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;
        let response = self
            .http
            .put(url)
            .bearer_auth(&self.config.admin_api_key)
            .json(chunks)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(MeilisearchError::Api { status, body });
        }

        self.wait_for_import_task_if_enabled(&body).await?;

        Ok(MeilisearchImport { body })
    }

    async fn wait_for_import_task_if_enabled(&self, body: &str) -> Result<()> {
        if std::env::var("MEILI_WAIT_FOR_TASKS")
            .is_ok_and(|value| value == "0" || value.eq_ignore_ascii_case("false"))
        {
            return Ok(());
        }

        let task: Value = serde_json::from_str(body)?;
        let Some(task_uid) = task
            .get("taskUid")
            .or_else(|| task.get("uid"))
            .and_then(Value::as_u64)
        else {
            return Ok(());
        };

        self.wait_for_task(task_uid).await
    }

    async fn wait_for_task(&self, task_uid: u64) -> Result<()> {
        let deadline = tokio::time::Instant::now() + TASK_TIMEOUT;
        loop {
            let path = format!("tasks/{task_uid}");
            let url = endpoint(&self.config.api_url, &path)?;
            let response = self
                .http
                .get(url)
                .bearer_auth(&self.config.admin_api_key)
                .send()
                .await?;
            let task = json_response(response).await?;
            match task.get("status").and_then(Value::as_str) {
                Some("succeeded") => return Ok(()),
                Some("failed") | Some("canceled") => {
                    return Err(MeilisearchError::TaskFailed(task.to_string()));
                }
                _ if tokio::time::Instant::now() >= deadline => {
                    return Err(MeilisearchError::TaskTimeout(task_uid));
                }
                _ => sleep(TASK_POLL_INTERVAL).await,
            }
        }
    }
}

#[async_trait]
impl ChunkSink for MeilisearchAdminClient {
    async fn upsert_chunks(&self, chunks: &[TranscriptChunk]) -> youtube_ingest::Result<()> {
        self.import_chunks(chunks)
            .await
            .map(|_| ())
            .map_err(|error| IngestError::Sink(error.to_string()))
    }
}

#[async_trait]
impl TranscriptChunkSource for MeilisearchAdminClient {
    async fn chunks_page_by_youtuber_id(
        &self,
        youtuber_id: &str,
        offset: usize,
        limit: usize,
    ) -> youtube_ingest::Result<TranscriptChunkPage> {
        self.chunks_page_by_youtuber_id(youtuber_id, offset, limit)
            .await
            .map_err(|error| IngestError::ChunkSource(error.to_string()))
    }
}

#[derive(Clone, Debug)]
pub struct MeilisearchSearchClient {
    http: HttpClient,
    config: MeilisearchSearchConfig,
}

impl MeilisearchSearchClient {
    pub fn new(config: MeilisearchSearchConfig) -> Self {
        Self {
            http: timeout_http_client(),
            config,
        }
    }

    pub fn with_http_client(http: HttpClient, config: MeilisearchSearchConfig) -> Self {
        Self { http, config }
    }

    pub fn config(&self) -> &MeilisearchSearchConfig {
        &self.config
    }

    pub async fn search(&self, query: &TextSearchQuery) -> Result<Value> {
        let path = format!("indexes/{}/search", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;
        let limit = query.per_page.unwrap_or(20);
        let page = query.page.unwrap_or(1).max(1);
        let offset = (page - 1).saturating_mul(u32::from(limit));
        let mut body = json!({
            "q": query.q,
            "limit": limit,
            "offset": offset,
            "attributesToSearchOn": split_csv(&query.query_by),
            "attributesToRetrieve": [
                "id",
                "video_id",
                "title",
                "text",
                "prev_text",
                "next_text",
                "youtuber_handle",
                "youtuber_name",
                "thumbnail",
                "published_at",
                "duration",
                "t_start_ms",
                "t_end_ms",
            ],
            "attributesToHighlight": query
                .highlight_fields
                .as_deref()
                .map(split_csv)
                .unwrap_or_default(),
            "highlightPreTag": "<mark>",
            "highlightPostTag": "</mark>",
        });
        if let Some(filter_by) = &query.filter_by {
            body["filter"] = Value::String(filter_to_meili(filter_by));
        }
        if let Some(group_by) = &query.group_by {
            body["distinct"] = Value::String(group_by.clone());
        }
        if let Some(sort_by) = &query.sort_by {
            body["sort"] = Value::Array(vec![Value::String(sort_by.clone())]);
        }

        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.search_api_key)
            .json(&body)
            .send()
            .await?;

        json_response(response).await
    }

    pub async fn count_distinct(&self, query: &TextSearchQuery, field: &str) -> Result<u64> {
        let path = format!("indexes/{}/search", self.config.collection);
        let url = endpoint(&self.config.api_url, &path)?;
        let mut body = json!({
            "q": query.q,
            "limit": 0,
            "attributesToSearchOn": split_csv(&query.query_by),
            "facets": [field],
        });
        if let Some(filter_by) = &query.filter_by {
            body["filter"] = Value::String(filter_to_meili(filter_by));
        }

        let response = self
            .http
            .post(url)
            .bearer_auth(&self.config.search_api_key)
            .json(&body)
            .send()
            .await?;

        let body = json_response(response).await?;
        Ok(body
            .get("facetDistribution")
            .and_then(|facets| facets.get(field))
            .and_then(Value::as_object)
            .map_or(0, |values| values.len() as u64))
    }

    pub async fn list_youtubers(&self) -> Result<Vec<YoutuberDocument>> {
        let path = format!("indexes/{YOUTUBERS_COLLECTION}/search");
        let url = endpoint(&self.config.api_url, &path)?;
        const PAGE_SIZE: u64 = 1_000;
        let mut offset = 0_u64;
        let mut channels = Vec::new();

        loop {
            let response = self
                .http
                .post(url.clone())
                .bearer_auth(&self.config.search_api_key)
                .json(&json!({
                    "q": "",
                    "limit": PAGE_SIZE,
                    "offset": offset,
                    "attributesToSearchOn": ["name", "handle"],
                }))
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
                    .map(serde_json::from_value)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(MeilisearchError::Json)?,
            );

            let total = body
                .get("totalHits")
                .or_else(|| body.get("estimatedTotalHits"))
                .and_then(Value::as_u64)
                .unwrap_or(offset.saturating_add(hit_count));
            offset = offset.saturating_add(hit_count);
            if hit_count == 0 || offset >= total {
                return Ok(channels);
            }
        }
    }
}

#[async_trait]
impl TranscriptSearchBackend for MeilisearchSearchClient {
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
        MeilisearchSearchClient::search(self, &query)
            .await
            .map_err(|error| SearchBackendError::new("meilisearch", error))
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
        MeilisearchSearchClient::count_distinct(self, &query, field)
            .await
            .map_err(|error| SearchBackendError::new("meilisearch", error))
    }

    async fn list_youtubers(
        &self,
    ) -> std::result::Result<Vec<SharedYoutuberDocument>, SearchBackendError> {
        MeilisearchSearchClient::list_youtubers(self)
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
            .map_err(|error| SearchBackendError::new("meilisearch", error))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeilisearchImport {
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
pub enum MeilisearchError {
    #[error("invalid meilisearch url: {0}")]
    InvalidUrl(String),
    #[error("meilisearch http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("meilisearch json failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("meilisearch returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("meilisearch task did not finish before timeout: {0}")]
    TaskTimeout(u64),
    #[error("meilisearch task failed: {0}")]
    TaskFailed(String),
    #[error("meilisearch import retry attempts exhausted")]
    RetryExhausted,
}

impl MeilisearchError {
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
            | Self::TaskTimeout(_)
            | Self::TaskFailed(_)
            | Self::RetryExhausted => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, MeilisearchError>;

pub fn transcript_chunk_schema(collection: &str) -> Value {
    json!({
        "name": collection,
        "fields": [
            { "name": "id", "type": "string" },
            { "name": "youtuber_id", "type": "string", "facet": true },
            { "name": "youtuber_handle", "type": "string", "facet": true },
            { "name": "youtuber_name", "type": "string" },
            { "name": "channel_id", "type": "string", "index": false },
            { "name": "video_id", "type": "string", "facet": true },
            { "name": "title", "type": "string" },
            { "name": "text", "type": "string" },
            { "name": "prev_text", "type": "string", "optional": true, "index": false },
            { "name": "next_text", "type": "string", "optional": true, "index": false },
            { "name": "sequence", "type": "int32", "index": false },
            { "name": "chapter", "type": "string", "optional": true, "index": false },
            { "name": "chapter_index", "type": "int32", "optional": true, "index": false },
            { "name": "duration", "type": "int32", "optional": true, "index": false },
            { "name": "t_start_ms", "type": "int32" },
            { "name": "t_end_ms", "type": "int32", "index": false },
            { "name": "thumbnail", "type": "string", "optional": true, "index": false },
            { "name": "published_at", "type": "string", "optional": true, "index": false },
            { "name": "upload_date", "type": "string", "optional": true, "index": false },
            { "name": "uploader_id", "type": "string", "index": false }
        ],
        "default_sorting_field": "t_start_ms"
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
        .expect("meilisearch HTTP client configuration should be valid")
}

fn meili_settings_from_schema(schema: &Value) -> Value {
    let fields = schema
        .get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let filterable = fields
        .iter()
        .filter(|field| field.get("facet").and_then(Value::as_bool) == Some(true))
        .filter_map(|field| field.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let searchable = fields
        .iter()
        .filter(|field| field.get("index").and_then(Value::as_bool) != Some(false))
        .filter_map(|field| field.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();

    json!({
        "filterableAttributes": filterable,
        "pagination": {
            "maxTotalHits": 1_000_000,
        },
        "faceting": {
            "maxValuesPerFacet": 100_000,
        },
        "sortableAttributes": ["published_at", "t_start_ms"],
        "searchableAttributes": searchable,
        "displayedAttributes": ["*"],
    })
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn filter_to_meili(filter_by: &str) -> String {
    filter_by
        .split("&&")
        .map(str::trim)
        .map(|part| {
            if let Some((field, value)) = part.split_once(":=") {
                format!("{} = {:?}", field.trim(), value.trim())
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn youtuber_id_filter(youtuber_id: &str) -> String {
    let canonical = youtuber_id.trim_start_matches('@');
    let legacy = format!("@{canonical}");
    format!("youtuber_id = {canonical:?} OR youtuber_id = {legacy:?}")
}

fn endpoint(api_url: &str, path: &str) -> Result<Url> {
    let base = normalized_api_url(api_url)?;
    Url::parse(&format!("{base}/{path}"))
        .map_err(|source| MeilisearchError::InvalidUrl(source.to_string()))
}

fn normalized_api_url(api_url: &str) -> Result<String> {
    let api_url = api_url.trim().trim_end_matches('/');
    if api_url.is_empty() {
        return Err(MeilisearchError::InvalidUrl(
            "MEILI_HTTP_ADDR cannot be empty".to_string(),
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
        return Err(MeilisearchError::Api { status, body });
    }

    Ok(serde_json::from_str(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_config_debug_redacts_api_key() {
        let config = MeilisearchAdminConfig::new("http://localhost:8108", "admin-secret");
        let debug = format!("{config:?}");

        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("admin-secret"));
    }

    #[test]
    fn search_config_debug_redacts_api_key() {
        let config = MeilisearchSearchConfig::new("http://localhost:8108", "search-secret");
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
    fn import_batches_multiple_chunks_per_request() {
        assert!(IMPORT_BATCH_SIZE > 1);
    }

    #[test]
    fn meili_settings_raise_total_hit_cap_for_counts() {
        let settings = meili_settings_from_schema(&transcript_chunk_schema(DEFAULT_COLLECTION));

        assert_eq!(settings["pagination"]["maxTotalHits"], 1_000_000);
    }

    #[test]
    fn meili_settings_raise_facet_value_cap_for_distinct_counts() {
        let settings = meili_settings_from_schema(&transcript_chunk_schema(DEFAULT_COLLECTION));

        assert_eq!(settings["faceting"]["maxValuesPerFacet"], 100_000);
    }

    #[test]
    fn search_query_defaults_to_grouped_transcript_search() {
        let query = TextSearchQuery::new("react server components");

        assert_eq!(query.query_by, "text,title,youtuber_name,youtuber_handle");
        assert_eq!(query.group_by.as_deref(), Some("video_id"));
        assert_eq!(query.group_limit, Some(5));
    }

    #[test]
    fn endpoint_accepts_meilisearch_host_without_scheme() {
        let url = endpoint("example.fly.dev", "indexes").unwrap();

        assert_eq!(url.as_str(), "https://example.fly.dev/indexes");
    }

    #[test]
    fn youtuber_filter_matches_canonical_and_legacy_ids() {
        let filter = youtuber_id_filter("@t3dotgg");

        assert_eq!(
            filter,
            r#"youtuber_id = "t3dotgg" OR youtuber_id = "@t3dotgg""#
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
