use reqwest::{Client as HttpClient, Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::BTreeMap, fmt, time::Duration};
use url::Url;

pub const DEFAULT_BASE_URL: &str = "https://www.googleapis.com/youtube/v3/";
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
pub enum Auth {
    ApiKey(String),
    Bearer(String),
    None,
}

#[derive(Clone, Debug)]
pub struct YouTubeClient {
    http: HttpClient,
    base_url: Url,
    auth: Auth,
}

impl YouTubeClient {
    pub fn with_api_key(api_key: impl Into<String>) -> Result<Self> {
        Self::new(Auth::ApiKey(api_key.into()))
    }

    pub fn with_bearer_token(token: impl Into<String>) -> Result<Self> {
        Self::new(Auth::Bearer(token.into()))
    }

    pub fn unauthenticated() -> Result<Self> {
        Self::new(Auth::None)
    }

    pub fn new(auth: Auth) -> Result<Self> {
        Self::with_http_client(timeout_http_client(), auth)
    }

    pub fn with_http_client(http: HttpClient, auth: Auth) -> Result<Self> {
        Ok(Self {
            http,
            base_url: Url::parse(DEFAULT_BASE_URL)?,
            auth,
        })
    }

    pub fn endpoint(&self, resource: Resource, method: ApiMethod) -> RequestBuilder<'_> {
        RequestBuilder::new(self, resource, method)
    }

    pub fn list(&self, resource: ListResource) -> RequestBuilder<'_> {
        self.endpoint(resource.into(), ApiMethod::List)
    }

    fn url_for(
        &self,
        resource: Resource,
        method: ApiMethod,
        path_params: &BTreeMap<String, String>,
    ) -> Result<Url> {
        let mut path = resource.path(method).to_string();
        for (key, value) in path_params {
            path = path.replace(&format!("{{{key}}}"), value);
        }

        Ok(self.base_url.join(&path)?)
    }

    async fn execute<T: DeserializeOwned>(
        &self,
        resource: Resource,
        method: ApiMethod,
        query: &Query,
        path_params: &BTreeMap<String, String>,
        body: Option<Value>,
    ) -> Result<T> {
        let mut url = self.url_for(resource, method, path_params)?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &query.params {
                pairs.append_pair(key, value);
            }
            if let Auth::ApiKey(key) = &self.auth {
                pairs.append_pair("key", key);
            }
        }

        let mut request = self.http.request(method.http_method(), url);
        if let Auth::Bearer(token) = &self.auth {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }

        let response = request.send().await?;
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

    pub fn maybe_set(mut self, key: impl Into<String>, value: Option<impl ToString>) -> Self {
        if let Some(value) = value {
            self.params.insert(key.into(), value.to_string());
        }
        self
    }

    pub fn part(self, parts: impl IntoParts) -> Self {
        self.set("part", parts.into_parts())
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
    resource: Resource,
    method: ApiMethod,
    query: Query,
    path_params: BTreeMap<String, String>,
    body: Option<Value>,
}

impl<'a> RequestBuilder<'a> {
    fn new(client: &'a YouTubeClient, resource: Resource, method: ApiMethod) -> Self {
        Self {
            client,
            resource,
            method,
            query: Query::new(),
            path_params: BTreeMap::new(),
            body: None,
        }
    }

    pub fn query(mut self, query: Query) -> Self {
        self.query = query;
        self
    }

    pub fn param(mut self, key: impl Into<String>, value: impl ToString) -> Self {
        self.query = self.query.set(key, value);
        self
    }

    pub fn path_param(mut self, key: impl Into<String>, value: impl ToString) -> Self {
        self.path_params.insert(key.into(), value.to_string());
        self
    }

    pub fn part(mut self, parts: impl IntoParts) -> Self {
        self.query = self.query.part(parts);
        self
    }

    pub fn body(mut self, body: impl Serialize) -> serde_json::Result<Self> {
        self.body = Some(serde_json::to_value(body)?);
        Ok(self)
    }

    pub async fn send<T: DeserializeOwned>(self) -> Result<T> {
        self.client
            .execute(
                self.resource,
                self.method,
                &self.query,
                &self.path_params,
                self.body,
            )
            .await
    }

    pub async fn send_json(self) -> Result<Value> {
        self.send().await
    }

    pub async fn page(self) -> Result<Page<Value>> {
        self.send().await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiMethod {
    List,
    Insert,
    Update,
    Delete,
    Set,
    Unset,
    GetRating,
    Rate,
    ReportAbuse,
    Download,
    MarkAsSpam,
    SetModerationStatus,
    Transition,
    Bind,
    InsertCuepoint,
    BatchGetStats,
    Get,
}

impl ApiMethod {
    pub fn http_method(self) -> Method {
        match self {
            Self::List | Self::GetRating | Self::Download | Self::BatchGetStats | Self::Get => {
                Method::GET
            }
            Self::Delete => Method::DELETE,
            Self::Update => Method::PUT,
            Self::Insert | Self::ReportAbuse | Self::Bind | Self::InsertCuepoint => Method::POST,
            Self::Set
            | Self::Unset
            | Self::Rate
            | Self::MarkAsSpam
            | Self::SetModerationStatus
            | Self::Transition => Method::POST,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListResource {
    Activities,
    Captions,
    ChannelSections,
    Channels,
    CommentThreads,
    Comments,
    I18nLanguages,
    I18nRegions,
    LiveBroadcasts,
    LiveChatMessages,
    LiveChatModerators,
    LiveStreams,
    Members,
    MembershipsLevels,
    PlaylistImages,
    PlaylistItems,
    Playlists,
    Search,
    Subscriptions,
    SuperChatEvents,
    ThirdPartyLinks,
    VideoAbuseReportReasons,
    VideoCategories,
    Videos,
}

impl From<ListResource> for Resource {
    fn from(value: ListResource) -> Self {
        match value {
            ListResource::Activities => Self::Activities,
            ListResource::Captions => Self::Captions,
            ListResource::ChannelSections => Self::ChannelSections,
            ListResource::Channels => Self::Channels,
            ListResource::CommentThreads => Self::CommentThreads,
            ListResource::Comments => Self::Comments,
            ListResource::I18nLanguages => Self::I18nLanguages,
            ListResource::I18nRegions => Self::I18nRegions,
            ListResource::LiveBroadcasts => Self::LiveBroadcasts,
            ListResource::LiveChatMessages => Self::LiveChatMessages,
            ListResource::LiveChatModerators => Self::LiveChatModerators,
            ListResource::LiveStreams => Self::LiveStreams,
            ListResource::Members => Self::Members,
            ListResource::MembershipsLevels => Self::MembershipsLevels,
            ListResource::PlaylistImages => Self::PlaylistImages,
            ListResource::PlaylistItems => Self::PlaylistItems,
            ListResource::Playlists => Self::Playlists,
            ListResource::Search => Self::Search,
            ListResource::Subscriptions => Self::Subscriptions,
            ListResource::SuperChatEvents => Self::SuperChatEvents,
            ListResource::ThirdPartyLinks => Self::ThirdPartyLinks,
            ListResource::VideoAbuseReportReasons => Self::VideoAbuseReportReasons,
            ListResource::VideoCategories => Self::VideoCategories,
            ListResource::Videos => Self::Videos,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resource {
    AbuseReports,
    Activities,
    Captions,
    ChannelBanners,
    ChannelSections,
    Channels,
    CommentThreads,
    Comments,
    I18nLanguages,
    I18nRegions,
    LiveBroadcasts,
    LiveChatBans,
    LiveChatMessages,
    LiveChatModerators,
    LiveStreams,
    Members,
    MembershipsLevels,
    PlaylistImages,
    PlaylistItems,
    Playlists,
    Search,
    Subscriptions,
    SuperChatEvents,
    Tests,
    ThirdPartyLinks,
    Thumbnails,
    VideoAbuseReportReasons,
    VideoCategories,
    VideoTrainability,
    Videos,
    Watermarks,
}

impl Resource {
    pub fn path(self, method: ApiMethod) -> &'static str {
        match (self, method) {
            (Self::ChannelBanners, ApiMethod::Insert) => "channelBanners/insert",
            (Self::Comments, ApiMethod::MarkAsSpam) => "comments/markAsSpam",
            (Self::Comments, ApiMethod::SetModerationStatus) => "comments/setModerationStatus",
            (Self::LiveBroadcasts, ApiMethod::Bind) => "liveBroadcasts/bind",
            (Self::LiveBroadcasts, ApiMethod::InsertCuepoint) => "liveBroadcasts/cuepoint",
            (Self::LiveBroadcasts, ApiMethod::Transition) => "liveBroadcasts/transition",
            (Self::LiveChatMessages, ApiMethod::Transition) => "liveChat/messages/transition",
            (Self::Thumbnails, ApiMethod::Set) => "thumbnails/set",
            (Self::Videos, ApiMethod::BatchGetStats) => "videos:batchGetStats",
            (Self::Videos, ApiMethod::GetRating) => "videos/getRating",
            (Self::Videos, ApiMethod::Rate) => "videos/rate",
            (Self::Videos, ApiMethod::ReportAbuse) => "videos/reportAbuse",
            (Self::Watermarks, ApiMethod::Set) => "watermarks/set",
            (Self::Watermarks, ApiMethod::Unset) => "watermarks/unset",
            (Self::Captions, ApiMethod::Download) => "captions/{id}",
            (resource, _) => resource.base_path(),
        }
    }

    pub fn base_path(self) -> &'static str {
        match self {
            Self::AbuseReports => "abuseReports",
            Self::Activities => "activities",
            Self::Captions => "captions",
            Self::ChannelBanners => "channelBanners",
            Self::ChannelSections => "channelSections",
            Self::Channels => "channels",
            Self::CommentThreads => "commentThreads",
            Self::Comments => "comments",
            Self::I18nLanguages => "i18nLanguages",
            Self::I18nRegions => "i18nRegions",
            Self::LiveBroadcasts => "liveBroadcasts",
            Self::LiveChatBans => "liveChat/bans",
            Self::LiveChatMessages => "liveChat/messages",
            Self::LiveChatModerators => "liveChat/moderators",
            Self::LiveStreams => "liveStreams",
            Self::Members => "members",
            Self::MembershipsLevels => "membershipsLevels",
            Self::PlaylistImages => "playlistImages",
            Self::PlaylistItems => "playlistItems",
            Self::Playlists => "playlists",
            Self::Search => "search",
            Self::Subscriptions => "subscriptions",
            Self::SuperChatEvents => "superChatEvents",
            Self::Tests => "tests",
            Self::ThirdPartyLinks => "thirdPartyLinks",
            Self::Thumbnails => "thumbnails",
            Self::VideoAbuseReportReasons => "videoAbuseReportReasons",
            Self::VideoCategories => "videoCategories",
            Self::VideoTrainability => "videoTrainability",
            Self::Videos => "videos",
            Self::Watermarks => "watermarks",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T = Value> {
    pub kind: Option<String>,
    pub etag: Option<String>,
    pub next_page_token: Option<String>,
    pub prev_page_token: Option<String>,
    pub page_info: Option<PageInfo>,
    #[serde(default)]
    pub items: Vec<T>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub total_results: Option<u64>,
    pub results_per_page: Option<u64>,
}

pub trait IntoParts {
    fn into_parts(self) -> String;
}

impl IntoParts for &str {
    fn into_parts(self) -> String {
        self.to_string()
    }
}

impl IntoParts for String {
    fn into_parts(self) -> String {
        self
    }
}

impl<const N: usize> IntoParts for [&str; N] {
    fn into_parts(self) -> String {
        join(self)
    }
}

impl IntoParts for &[&str] {
    fn into_parts(self) -> String {
        join(self.iter().copied())
    }
}

impl IntoParts for Vec<&str> {
    fn into_parts(self) -> String {
        join(self)
    }
}

impl IntoParts for Vec<String> {
    fn into_parts(self) -> String {
        join(self)
    }
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

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.base_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_base_url_keeps_youtube_v3_prefix() {
        let client = YouTubeClient::unauthenticated().unwrap();

        let url = client
            .url_for(Resource::Videos, ApiMethod::GetRating, &BTreeMap::default())
            .unwrap();

        assert_eq!(
            url.as_str(),
            "https://www.googleapis.com/youtube/v3/videos/getRating"
        );
    }
}
