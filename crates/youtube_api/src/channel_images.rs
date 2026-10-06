use crate::ApiError;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, RwLock};
use youtube_search_client::{ChannelDocument, SearchClient};

/// Minimum time between creator-list refreshes caused by unknown channel IDs.
/// Without it, every request for a made-up but well-formed ID re-listed every
/// creator from the reader.
const MISS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub(crate) struct ChannelImageCache {
    urls: Arc<RwLock<HashMap<String, String>>>,
    /// Serializes refreshes and records when the creator list was last loaded.
    last_refresh: Arc<Mutex<Option<Instant>>>,
}

impl ChannelImageCache {
    pub(crate) async fn replace(&self, channels: &[ChannelDocument]) {
        let urls = channel_image_urls(channels.iter());
        *self.urls.write().await = urls;
        *self.last_refresh.lock().await = Some(Instant::now());
    }

    pub(crate) async fn get_or_refresh(
        &self,
        channel_id: &str,
        search: &SearchClient,
    ) -> Result<String, ApiError> {
        if let Some(url) = self.urls.read().await.get(channel_id).cloned() {
            return Ok(url);
        }

        let unavailable = || ApiError::BadRequest("channel image unavailable".to_string());
        // Serialize refreshes without holding the URL map lock across network I/O.
        let mut last_refresh = self.last_refresh.lock().await;
        if let Some(url) = self.urls.read().await.get(channel_id).cloned() {
            return Ok(url);
        }
        if !refresh_due(*last_refresh, Instant::now()) {
            return Err(unavailable());
        }

        let channels = search.list_youtubers().await?;
        let refreshed = channel_image_urls(channels.iter());
        let result = refreshed.get(channel_id).cloned();
        *self.urls.write().await = refreshed;
        *last_refresh = Some(Instant::now());
        result.ok_or_else(unavailable)
    }
}

fn refresh_due(last_refresh: Option<Instant>, now: Instant) -> bool {
    last_refresh.is_none_or(|last| now.duration_since(last) >= MISS_REFRESH_INTERVAL)
}

fn channel_image_urls<'a>(
    channels: impl Iterator<Item = &'a ChannelDocument>,
) -> HashMap<String, String> {
    channels
        .filter_map(|channel| {
            channel
                .avatar_url
                .as_ref()
                .filter(|url| is_allowed_youtube_image_url(url))
                .map(|url| (channel.channel_id.clone(), url.clone()))
        })
        .collect()
}

pub(crate) fn resized_youtube_image_url(url: &str, size: u16) -> String {
    let Some(marker) = url.rfind("=s") else {
        return url.to_string();
    };
    let digits_start = marker + 2;
    let digits_end = url[digits_start..]
        .find(|character: char| !character.is_ascii_digit())
        .map_or(url.len(), |offset| digits_start + offset);
    if digits_end == digits_start {
        return url.to_string();
    }
    format!("{}{size}{}", &url[..digits_start], &url[digits_end..])
}

pub(crate) fn is_valid_channel_id(channel_id: &str) -> bool {
    channel_id.starts_with("UC")
        && (20..=30).contains(&channel_id.len())
        && channel_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub(crate) fn is_allowed_youtube_image_url(url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && matches!(
            url.host_str(),
            Some("yt3.ggpht.com" | "yt3.googleusercontent.com")
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refreshes_for_unknown_ids_at_most_once_per_interval() {
        let start = Instant::now();
        assert!(refresh_due(None, start));
        assert!(!refresh_due(Some(start), start + Duration::from_secs(59)));
        assert!(refresh_due(Some(start), start + MISS_REFRESH_INTERVAL));
    }

    #[test]
    fn only_proxies_canonical_youtube_channel_images() {
        assert!(is_valid_channel_id("UC6nSFpj9HTCZ5t-N3Rm3-HA"));
        assert!(!is_valid_channel_id("../etc/passwd"));
        assert!(is_allowed_youtube_image_url(
            "https://yt3.googleusercontent.com/channel-avatar=s800-c-k-c0x00ffffff-no-rj"
        ));
        assert!(is_allowed_youtube_image_url(
            "https://yt3.ggpht.com/channel-avatar=s800-c-k-c0x00ffffff-no-rj"
        ));
        assert!(!is_allowed_youtube_image_url(
            "https://example.com/avatar.png"
        ));
        assert!(!is_allowed_youtube_image_url(
            "http://yt3.ggpht.com/avatar.png"
        ));
        assert_eq!(
            resized_youtube_image_url(
                "https://yt3.ggpht.com/channel-avatar=s800-c-k-c0x00ffffff-no-rj",
                96
            ),
            "https://yt3.ggpht.com/channel-avatar=s96-c-k-c0x00ffffff-no-rj"
        );
        assert_eq!(
            resized_youtube_image_url("https://yt3.ggpht.com/avatar.png", 96),
            "https://yt3.ggpht.com/avatar.png"
        );
    }
}
