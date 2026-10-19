//! Optional read-only HTTP boundary. Model/runtime/Qdrant write keys never enter this crate.
use super::*;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Mode {
    #[default]
    Lexical,
    Semantic,
    Hybrid,
}
impl Mode {
    /// The engine that answers. Low-signal queries and explicit sorts stay
    /// lexical; a video-scoped moment lookup in hybrid mode uses semantic, which
    /// filters by video directly.
    pub(super) fn route(self, sort: SearchSort, low_signal: bool, video_scope: bool) -> Self {
        match self {
            _ if sort != SearchSort::Relevance || low_signal => Self::Lexical,
            Self::Hybrid if video_scope => Self::Semantic,
            mode => mode,
        }
    }
}
#[derive(Clone)]
pub(super) struct Client {
    http: reqwest::Client,
    url: reqwest::Url,
}
impl Client {
    pub(super) fn from_env() -> Result<Option<Self>, ApiError> {
        env::var("SEMANTIC_SEARCH_URL")
            .ok()
            .map(|url| Self::new(&url))
            .transpose()
    }
    fn new(url: &str) -> Result<Self, ApiError> {
        let mut url = reqwest::Url::parse(url)
            .map_err(|_| ApiError::BadRequest("invalid SEMANTIC_SEARCH_URL".into()))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(ApiError::BadRequest(
                "SEMANTIC_SEARCH_URL must be an origin without credentials".into(),
            ));
        }
        url.set_path("/search");
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ApiError::SemanticUnavailable)?;
        Ok(Self { http, url })
    }
    pub(super) async fn hits(
        &self,
        query: &str,
        params: &SearchParams,
    ) -> Result<Vec<SearchHit>, ApiError> {
        let creator = non_empty(params.youtuber.as_deref()).map(canonical_creator);
        let video_id = non_empty(params.video_id.as_deref());
        let mut response = self
            .http
            .post(self.url.clone())
            .json(&json!({"q":query,"filter":{"creator":creator,"video_id":video_id},"limit":100}))
            .send()
            .await
            .map_err(|_| ApiError::SemanticUnavailable)?;
        if response.status() == StatusCode::BAD_REQUEST {
            return Err(ApiError::BadRequest(
                "invalid semantic query or filters".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(ApiError::SemanticUnavailable);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiError::SemanticUnavailable)?
        {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(ApiError::SemanticUnavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::SemanticUnavailable)?;
        if value["mode"] != "semantic"
            || !value["hits"].is_array()
            || value["hits"].as_array().is_some_and(|h| h.len() > 100)
        {
            return Err(ApiError::SemanticUnavailable);
        }
        parse_hits(&value).map_err(|_| ApiError::SemanticUnavailable)
    }
}
fn canonical_creator(handle: &str) -> String {
    normalize_handle(handle).to_ascii_lowercase()
}
pub(super) async fn response(
    state: &AppState,
    params: &SearchParams,
    query: &str,
    per_page: u8,
    page: u32,
) -> Result<Response, ApiError> {
    let client = state
        .semantic
        .as_ref()
        .ok_or(ApiError::SemanticUnavailable)?;
    let started = Instant::now();
    let hits = client.hits(query, params).await?;
    let moments = hits.len() as u64;
    // Preserve Qdrant order through the existing card/timestamp hydration path.
    let videos = video_results_from_hits_with_policy(
        hits,
        SearchSort::Relevance,
        query,
        RelevanceOrdering::Backend,
    );
    let total_videos = videos.len() as u64;
    let offset = (page - 1).saturating_mul(u32::from(per_page)) as usize;
    let (videos, has_next_page) = page_of_videos(videos, offset, per_page);
    Ok(no_store_json(SearchResponse {
        query,
        total_moments: moments,
        total_videos,
        totals_approximate: true,
        total_found: moments,
        out_of: moments,
        time_to_first_result_ms: started.elapsed().as_millis() as u64,
        page,
        per_page,
        sort: SearchSort::Relevance,
        mode: "semantic",
        has_next_page,
        hits: [],
        videos: &videos,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opt_in_preserves_low_signal_and_explicit_sorts() {
        let relevance = |mode: Mode, query: &str, video_scope: bool| {
            mode.route(
                SearchSort::Relevance,
                is_low_signal_query(query),
                video_scope,
            )
        };
        assert_eq!(relevance(Mode::Lexical, "rust", false), Mode::Lexical);
        assert_eq!(relevance(Mode::Semantic, "rust", false), Mode::Semantic);
        assert_eq!(relevance(Mode::Hybrid, "why rust?", false), Mode::Hybrid);
        assert_eq!(relevance(Mode::Hybrid, "why rust?", true), Mode::Semantic);
        for mode in [Mode::Semantic, Mode::Hybrid] {
            assert_eq!(relevance(mode, "I", false), Mode::Lexical);
        }
        for sort in [
            SearchSort::PublishedAsc,
            SearchSort::PublishedDesc,
            SearchSort::ViewsDesc,
            SearchSort::RelevanceEngagement,
            SearchSort::LikesDesc,
            SearchSort::CommentsDesc,
        ] {
            for mode in [Mode::Semantic, Mode::Hybrid] {
                assert_eq!(mode.route(sort, false, false), Mode::Lexical);
            }
        }
        assert_eq!(canonical_creator("@aiDotEngineer"), "@aidotengineer");
        assert_eq!(canonical_creator("aiDotEngineer"), "@aidotengineer");
        assert!(Client::new("http://user:password@127.0.0.1:8790").is_err());
        assert!(serde_json::from_value::<Mode>(json!("fused")).is_err());
        assert_eq!(
            serde_json::from_value::<Mode>(json!("hybrid")).unwrap(),
            Mode::Hybrid
        );
        assert_eq!(
            serde_json::from_value::<Mode>(json!("semantic")).unwrap(),
            Mode::Semantic
        );
    }

    // search_handler answers lexically on SemanticUnavailable, so a busy or
    // unreachable service must surface as exactly that error, never a 4xx/5xx.
    #[tokio::test]
    async fn busy_unreachable_or_malformed_service_triggers_the_lexical_fallback() {
        let params: SearchParams = serde_json::from_value(json!({"q":"rust"})).unwrap();
        let busy = axum::Router::new().route(
            "/search",
            axum::routing::post(|| async { StatusCode::TOO_MANY_REQUESTS }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let busy_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, busy).await.unwrap() });
        let error = Client::new(&busy_url).unwrap().hits("rust", &params).await;
        assert!(matches!(error, Err(ApiError::SemanticUnavailable)));
        server.abort();

        // A valid envelope cannot make malformed backend documents into API errors.
        let malformed = axum::Router::new().route(
            "/search",
            axum::routing::post(|| async {
                Json(json!({"mode":"semantic", "hits":[{"document":{}}]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let malformed_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, malformed).await.unwrap() });
        let error = Client::new(&malformed_url)
            .unwrap()
            .hits("rust", &params)
            .await;
        assert!(matches!(error, Err(ApiError::SemanticUnavailable)));
        server.abort();

        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_url = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let error = Client::new(&closed_url)
            .unwrap()
            .hits("rust", &params)
            .await;
        assert!(matches!(error, Err(ApiError::SemanticUnavailable)));
    }
}
