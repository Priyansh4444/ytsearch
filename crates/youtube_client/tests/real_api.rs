use serde_json::Value;
use youtube_client::{ListResource, Page, Query, YouTubeClient};

fn api_key() -> Option<String> {
    let _ = dotenvy::from_path("../../.env");
    std::env::var("YOUTUBE_API_KEY").ok()
}

fn first_item(page: &Page<Value>) -> &Value {
    page.items.first().expect("expected at least one API item")
}

#[tokio::test]
#[ignore = "hits the real YouTube Data API and consumes quota"]
async fn resolves_t3dotgg_channel_by_handle() -> youtube_client::Result<()> {
    let Some(key) = api_key() else {
        eprintln!("YOUTUBE_API_KEY is not set; skipping real API test");
        return Ok(());
    };

    let youtube = YouTubeClient::with_api_key(key)?;
    let page = youtube
        .list(ListResource::Channels)
        .query(
            Query::new()
                .part([
                    "id",
                    "snippet",
                    "statistics",
                    "contentDetails",
                    "brandingSettings",
                ])
                .set("forHandle", "@t3dotgg")
                .fields(
                    "items(id,snippet(title,customUrl),statistics,contentDetails/relatedPlaylists/uploads,brandingSettings/channel/title)",
                ),
        )
        .page()
        .await?;

    let channel = first_item(&page);
    assert_eq!(
        channel
            .pointer("/snippet/customUrl")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("@t3dotgg")
    );

    let title = channel
        .pointer("/snippet/title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    assert!(
        title.contains("theo") || title.contains("t3"),
        "unexpected channel title: {title}"
    );

    assert!(
        channel
            .pointer("/contentDetails/relatedPlaylists/uploads")
            .and_then(Value::as_str)
            .is_some_and(|uploads| !uploads.is_empty()),
        "channel should expose an uploads playlist"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "hits the real YouTube Data API and consumes quota"]
async fn hydrates_recent_t3dotgg_upload_video_details() -> youtube_client::Result<()> {
    let Some(key) = api_key() else {
        eprintln!("YOUTUBE_API_KEY is not set; skipping real API test");
        return Ok(());
    };

    let youtube = YouTubeClient::with_api_key(key)?;
    let channel_page = youtube
        .list(ListResource::Channels)
        .query(
            Query::new()
                .part(["contentDetails"])
                .set("forHandle", "@t3dotgg")
                .fields("items(contentDetails/relatedPlaylists/uploads)"),
        )
        .page()
        .await?;

    let uploads_playlist = first_item(&channel_page)
        .pointer("/contentDetails/relatedPlaylists/uploads")
        .and_then(Value::as_str)
        .expect("expected uploads playlist");

    let uploads_page = youtube
        .list(ListResource::PlaylistItems)
        .query(
            Query::new()
                .part(["snippet", "contentDetails"])
                .set("playlistId", uploads_playlist)
                .max_results(1)
                .fields("items(snippet(title,channelTitle),contentDetails/videoId)"),
        )
        .page()
        .await?;

    let upload = first_item(&uploads_page);
    let video_id = upload
        .pointer("/contentDetails/videoId")
        .and_then(Value::as_str)
        .expect("expected video id");

    let video_page = youtube
        .list(ListResource::Videos)
        .query(
            Query::new()
                .part(["snippet", "contentDetails", "statistics", "status"])
                .ids([video_id])
                .fields("items(id,snippet(title,channelTitle,publishedAt,thumbnails),contentDetails(duration),statistics(viewCount),status/privacyStatus)"),
        )
        .page()
        .await?;

    let video = first_item(&video_page);
    assert_eq!(video.pointer("/id").and_then(Value::as_str), Some(video_id));
    assert!(
        video
            .pointer("/snippet/title")
            .and_then(Value::as_str)
            .is_some_and(|title| !title.is_empty()),
        "video title should be populated"
    );
    assert!(
        video
            .pointer("/snippet/thumbnails")
            .and_then(Value::as_object)
            .is_some_and(|thumbnails| !thumbnails.is_empty()),
        "video thumbnails should be populated"
    );

    Ok(())
}
