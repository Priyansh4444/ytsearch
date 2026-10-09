//! YouTube upload notifications over WebSub (PubSubHubbub).
//!
//! The hub pushes an Atom entry to the callback whenever a subscribed channel
//! publishes or edits a video. A verified push for an indexed creator queues
//! one update request for that creator, so new uploads no longer wait for the
//! daily sweep of every creator. Subscriptions expire, so the API renews every
//! indexed channel once a day.

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use std::time::Duration;

pub(crate) const DEFAULT_HUB_URL: &str = "https://pubsubhubbub.appspot.com/subscribe";
const TOPIC_PREFIX: &str = "https://www.youtube.com/xml/feeds/videos.xml?channel_id=";
/// The hub caps leases at a few days; renewing daily keeps every lease alive
/// through a missed renewal or two.
const LEASE_SECONDS: u64 = 10 * 24 * 60 * 60;
pub(crate) const RENEW_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub(crate) const MAX_NOTIFICATION_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub(crate) struct WebSubConfig {
    pub(crate) secret: String,
    pub(crate) callback_url: String,
    pub(crate) hub_url: String,
}

impl WebSubConfig {
    /// Enabled only when both the signing secret and the public callback URL
    /// are configured; otherwise the routes answer 404.
    pub(crate) fn from_env() -> Option<Self> {
        let secret = non_empty_env("YTSEARCH_WEBSUB_SECRET")?;
        let callback_url = non_empty_env("YTSEARCH_WEBSUB_CALLBACK_URL")?;
        let hub_url =
            non_empty_env("YTSEARCH_WEBSUB_HUB_URL").unwrap_or_else(|| DEFAULT_HUB_URL.to_string());
        Some(Self {
            secret,
            callback_url,
            hub_url,
        })
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn topic_for_channel(channel_id: &str) -> String {
    format!("{TOPIC_PREFIX}{channel_id}")
}

/// The channel ID of a YouTube channel feed topic, or `None` for any other URL.
pub(crate) fn channel_id_from_topic(topic: &str) -> Option<&str> {
    topic
        .strip_prefix(TOPIC_PREFIX)
        .filter(|channel_id| crate::channel_images::is_valid_channel_id(channel_id))
}

/// Checks `X-Hub-Signature: sha1=<hex>` against the raw body in constant time.
pub(crate) fn signature_valid(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(expected) = header
        .and_then(|value| value.trim().strip_prefix("sha1="))
        .and_then(decode_hex)
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(value.get(index..index + 2)?, 16).ok())
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct UploadNotification {
    pub(crate) channel_id: String,
    pub(crate) video_id: String,
}

/// Reads the channel and video from a published-or-edited entry. Deletions
/// (`at:deleted-entry`) carry no `yt:videoId` and are ignored.
pub(crate) fn parse_notification(body: &str) -> Option<UploadNotification> {
    let entry = &body[body.find("<entry")?..];
    let channel_id = element_text(entry, "yt:channelId")?;
    let video_id = element_text(entry, "yt:videoId")?;
    let video_id_valid = video_id.len() == 11
        && video_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    (crate::channel_images::is_valid_channel_id(channel_id) && video_id_valid).then(|| {
        UploadNotification {
            channel_id: channel_id.to_string(),
            video_id: video_id.to_string(),
        }
    })
}

fn element_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find("</")?;
    Some(xml[start..end].trim())
}

/// The form body of one subscription request to the hub.
pub(crate) fn subscribe_form(config: &WebSubConfig, channel_id: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("hub.callback", &config.callback_url)
        .append_pair("hub.topic", &topic_for_channel(channel_id))
        .append_pair("hub.verify", "async")
        .append_pair("hub.mode", "subscribe")
        .append_pair("hub.secret", &config.secret)
        .append_pair("hub.lease_seconds", &LEASE_SECONDS.to_string())
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNEL: &str = "UCXuqSBlHAE6Xw-yeJA0Tunw";

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha1>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let digest = mac.finalize().into_bytes();
        format!(
            "sha1={}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        )
    }

    fn feed(channel_id: &str, video_id: &str) -> String {
        format!(
            r#"<?xml version='1.0' encoding='UTF-8'?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns="http://www.w3.org/2005/Atom">
 <link rel="hub" href="https://pubsubhubbub.appspot.com"/>
 <title>YouTube video feed</title>
 <entry>
  <id>yt:video:{video_id}</id>
  <yt:videoId>{video_id}</yt:videoId>
  <yt:channelId>{channel_id}</yt:channelId>
  <title>Video title</title>
 </entry>
</feed>"#
        )
    }

    #[test]
    fn signature_accepts_only_the_shared_secret_over_the_exact_body() {
        let body = feed(CHANNEL, "dQw4w9WgXcQ");
        let header = sign("secret", body.as_bytes());
        assert!(signature_valid("secret", body.as_bytes(), Some(&header)));
        assert!(signature_valid(
            "secret",
            body.as_bytes(),
            Some(&header.to_uppercase().replace("SHA1=", "sha1="))
        ));
        assert!(!signature_valid("other", body.as_bytes(), Some(&header)));
        assert!(!signature_valid("secret", b"tampered", Some(&header)));
        assert!(!signature_valid("secret", body.as_bytes(), None));
        assert!(!signature_valid("secret", body.as_bytes(), Some("sha1=zz")));
        assert!(!signature_valid(
            "secret",
            body.as_bytes(),
            Some(&header.replace("sha1=", "sha256="))
        ));
    }

    #[test]
    fn parses_published_entries_and_ignores_deletions_and_bad_ids() {
        assert_eq!(
            parse_notification(&feed(CHANNEL, "dQw4w9WgXcQ")),
            Some(UploadNotification {
                channel_id: CHANNEL.to_string(),
                video_id: "dQw4w9WgXcQ".to_string(),
            })
        );
        let deleted = r#"<feed xmlns:at="http://purl.org/atompub/tombstones/1.0">
 <at:deleted-entry ref="yt:video:dQw4w9WgXcQ" when="2026-10-09T00:00:00+00:00">
  <at:by><name>Name</name><uri>https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw</uri></at:by>
 </at:deleted-entry>
</feed>"#;
        assert_eq!(parse_notification(deleted), None);
        assert_eq!(
            parse_notification(&feed("not-a-channel", "dQw4w9WgXcQ")),
            None
        );
        assert_eq!(parse_notification(&feed(CHANNEL, "short")), None);
        assert_eq!(parse_notification(&feed(CHANNEL, "dQw4w9WgXc<")), None);
    }

    #[test]
    fn topics_round_trip_and_reject_other_feeds() {
        assert_eq!(
            channel_id_from_topic(&topic_for_channel(CHANNEL)),
            Some(CHANNEL)
        );
        assert_eq!(
            channel_id_from_topic("https://www.youtube.com/xml/feeds/videos.xml?playlist_id=PL1"),
            None
        );
        assert_eq!(
            channel_id_from_topic("https://evil.example/?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw"),
            None
        );
    }

    #[test]
    fn subscribe_form_encodes_callback_topic_and_secret() {
        let config = WebSubConfig {
            secret: "s&cret".to_string(),
            callback_url: "https://search.example/api/websub/youtube".to_string(),
            hub_url: DEFAULT_HUB_URL.to_string(),
        };
        let form = subscribe_form(&config, CHANNEL);
        let pairs: Vec<(String, String)> = form_urlencoded::parse(form.as_bytes())
            .into_owned()
            .collect();
        assert!(pairs.contains(&(
            "hub.callback".to_string(),
            "https://search.example/api/websub/youtube".to_string()
        )));
        assert!(pairs.contains(&("hub.topic".to_string(), topic_for_channel(CHANNEL))));
        assert!(pairs.contains(&("hub.mode".to_string(), "subscribe".to_string())));
        assert!(pairs.contains(&("hub.secret".to_string(), "s&cret".to_string())));
    }
}
