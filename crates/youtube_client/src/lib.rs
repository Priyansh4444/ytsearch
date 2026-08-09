use reqwest::{Client as HttpClient, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};
use url::Url;

const DEFAULT_BASE_URL: &str = "https://www.googleapis.com/youtube/v3/";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum YouTubeError {
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(#[from] url::ParseError),
    #[error("http request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json decode failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("youtube api returned {status}: {body}")]
    Api { status: StatusCode, body: Value },
}

pub type Result<T> = std::result::Result<T, YouTubeError>;

#[derive(Clone, Debug)]
pub struct YouTubeClient {
    http: HttpClient,
    base_url: Url,
    api_key: String,
}

impl YouTubeClient {
    pub fn with_api_key(api_key: impl Into<String>) -> Result<Self> {
        Ok(Self {
            http: timeout_http_client(),
            base_url: Url::parse(DEFAULT_BASE_URL)?,
            api_key: api_key.into(),
        })
    }

    pub fn list(&self, resource: ListResource) -> RequestBuilder<'_> {
        RequestBuilder::new(self, resource)
    }

    fn url_for(&self, resource: ListResource) -> Result<Url> {
        Ok(self.base_url.join(resource.path())?)
    }

    async fn execute<T: DeserializeOwned>(
        &self,
        resource: ListResource,
        query: &Query,
    ) -> Result<T> {
        let mut url = self.url_for(resource)?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &query.params {
                pairs.append_pair(key, value);
            }
            pairs.append_pair("key", &self.api_key);
        }

        let response = self.http.get(url).send().await?;
        let status = response.status();
        let body = response.json::<Value>().await?;
        if !status.is_success() {
            return Err(YouTubeError::Api { status, body });
        }

        Ok(serde_json::from_value(body)?)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    params: BTreeMap<String, String>,
}

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, key: impl Into<String>, value: impl ToString) -> Self {
        self.params.insert(key.into(), value.to_string());
        self
    }

    pub fn part<I, S>(self, parts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.set("part", join(parts))
    }

    pub fn fields(self, fields: impl ToString) -> Self {
        self.set("fields", fields)
    }

    pub fn ids(self, ids: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        self.set("id", join(ids))
    }

    pub fn max_results(self, max_results: u8) -> Self {
        self.set("maxResults", max_results.clamp(0, 50))
    }

    pub fn page_token(self, token: impl ToString) -> Self {
        self.set("pageToken", token)
    }
}

pub struct RequestBuilder<'a> {
    client: &'a YouTubeClient,
    resource: ListResource,
    query: Query,
}

impl<'a> RequestBuilder<'a> {
    fn new(client: &'a YouTubeClient, resource: ListResource) -> Self {
        Self {
            client,
            resource,
            query: Query::new(),
        }
    }

    pub fn query(mut self, query: Query) -> Self {
        self.query = query;
        self
    }

    pub async fn page(self) -> Result<Page<Value>> {
        self.client.execute(self.resource, &self.query).await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListResource {
    Channels,
    PlaylistItems,
    Search,
    Videos,
}

impl ListResource {
    fn path(self) -> &'static str {
        match self {
            Self::Channels => "channels",
            Self::PlaylistItems => "playlistItems",
            Self::Search => "search",
            Self::Videos => "videos",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T = Value> {
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub items: Vec<T>,
}

fn timeout_http_client() -> HttpClient {
    HttpClient::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .expect("youtube HTTP client configuration should be valid")
}

fn join<I, S>(values: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    values
        .into_iter()
        .map(|value| value.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        task::JoinHandle,
    };

    async fn spawn_mock_server(
        status: &'static str,
        body: &'static str,
    ) -> (Url, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0_u8; 16 * 1024];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8(buffer[..read].to_vec()).unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        });
        let base_url = Url::parse(&format!("http://{address}/youtube/v3/")).unwrap();
        (base_url, server)
    }

    #[test]
    fn default_base_url_keeps_youtube_v3_prefix() {
        let client = YouTubeClient::with_api_key("test-key").unwrap();
        let url = client.url_for(ListResource::Videos).unwrap();

        assert_eq!(url.as_str(), "https://www.googleapis.com/youtube/v3/videos");
    }

    #[tokio::test]
    async fn list_request_sends_api_key_and_query_contract_to_mock_server() {
        let (base_url, server) = spawn_mock_server(
            "200 OK",
            r#"{"items":[{"id":"video-1"}],"nextPageToken":"next"}"#,
        )
        .await;
        let mut client = YouTubeClient::with_api_key("test-key").unwrap();
        client.base_url = base_url;

        let page = client
            .list(ListResource::Videos)
            .query(
                Query::new()
                    .part(["snippet", "contentDetails"])
                    .ids(["video-1"])
                    .max_results(7),
            )
            .page()
            .await
            .unwrap();
        let request = server.await.unwrap();
        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap();
        let request_url = Url::parse(&format!("http://mock{target}")).unwrap();
        let params = request_url.query_pairs().collect::<BTreeMap<_, _>>();

        assert_eq!(page.items[0]["id"], "video-1");
        assert_eq!(page.next_page_token.as_deref(), Some("next"));
        assert_eq!(request_url.path(), "/youtube/v3/videos");
        assert_eq!(
            params.get("part").map(|value| value.as_ref()),
            Some("snippet,contentDetails")
        );
        assert_eq!(
            params.get("id").map(|value| value.as_ref()),
            Some("video-1")
        );
        assert_eq!(
            params.get("maxResults").map(|value| value.as_ref()),
            Some("7")
        );
        assert_eq!(
            params.get("key").map(|value| value.as_ref()),
            Some("test-key")
        );
    }

    #[tokio::test]
    async fn api_errors_from_mock_server_preserve_status_and_json_body() {
        let (base_url, server) = spawn_mock_server(
            "403 Forbidden",
            r#"{"error":{"message":"quota exhausted"}}"#,
        )
        .await;
        let mut client = YouTubeClient::with_api_key("test-key").unwrap();
        client.base_url = base_url;

        let error = client
            .list(ListResource::Videos)
            .query(Query::new().ids(["video-1"]))
            .page()
            .await
            .unwrap_err();
        server.await.unwrap();

        match error {
            YouTubeError::Api { status, body } => {
                assert_eq!(status, StatusCode::FORBIDDEN);
                assert_eq!(body["error"]["message"], "quota exhausted");
            }
            other => panic!("expected structured API error, got {other}"),
        }
    }
}
