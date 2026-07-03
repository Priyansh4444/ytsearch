use clap::{Parser, Subcommand};
use serde_json::Value;
use thiserror::Error;
use youtube_client::{ListResource, Page, Query, YouTubeClient};
use youtube_typesense::{
    CreateCollectionOutcome, DEFAULT_COLLECTION, TextSearchQuery, TypesenseAdminClient,
    TypesenseAdminConfig, TypesenseSearchClient, TypesenseSearchConfig,
};

#[derive(Debug, Parser)]
#[command(name = "ytsearch")]
#[command(about = "YouTube transcript search indexing utilities")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Validate that a YouTube handle resolves and show recent uploads")]
    Validate {
        #[arg(value_name = "HANDLE", help = "YouTube handle, e.g. @t3dotgg")]
        youtuber: String,
        #[arg(long, default_value_t = 5, help = "Number of recent uploads to list")]
        max_videos: u8,
        #[arg(long, env = "YOUTUBE_API_KEY", help = "YouTube Data API key")]
        youtube_api_key: String,
    },
    #[command(about = "Validate a YouTuber and create the Typesense transcript index")]
    InitTypesense {
        #[arg(value_name = "HANDLE", help = "YouTube handle, e.g. @t3dotgg")]
        youtuber: String,
        #[arg(long, default_value_t = 5, help = "Number of recent uploads to verify")]
        max_videos: u8,
        #[arg(long, env = "YOUTUBE_API_KEY", help = "YouTube Data API key")]
        youtube_api_key: String,
        #[arg(
            long,
            env = "TYPESENSE_API_URL",
            help = "Typesense node/API URL, e.g. https://xxx.a1.typesense.net"
        )]
        typesense_api_url: String,
        #[arg(
            long,
            env = "TYPESENSE_ADMIN_API_KEY",
            help = "Typesense Admin API key; used only for schema/import operations"
        )]
        typesense_admin_api_key: String,
        #[arg(long, env = "TYPESENSE_COLLECTION", default_value = DEFAULT_COLLECTION)]
        collection: String,
    },
    #[command(about = "Search transcript chunks using the Typesense search-only API key")]
    Search {
        #[arg(value_name = "QUERY")]
        query: String,
        #[arg(long, help = "Optional YouTube handle filter, e.g. @t3dotgg")]
        youtuber: Option<String>,
        #[arg(
            long,
            env = "TYPESENSE_API_URL",
            help = "Typesense node/API URL, e.g. https://xxx.a1.typesense.net"
        )]
        typesense_api_url: String,
        #[arg(
            long,
            env = "TYPESENSE_SEARCH_API_KEY",
            help = "Typesense search-only API key"
        )]
        typesense_search_api_key: String,
        #[arg(long, env = "TYPESENSE_COLLECTION", default_value = DEFAULT_COLLECTION)]
        collection: String,
        #[arg(long, default_value_t = 10)]
        per_page: u8,
    },
}

#[derive(Debug, Error)]
enum CliError {
    #[error("youtube api failed: {0}")]
    YouTube(#[from] youtube_client::YouTubeError),
    #[error("typesense api failed: {0}")]
    Typesense(#[from] youtube_typesense::TypesenseError),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug)]
struct ValidatedYoutuber {
    handle: String,
    channel_id: String,
    title: String,
    uploads_playlist: Option<String>,
    recent_uploads: Vec<UploadPreview>,
    uploads_warning: Option<String>,
}

#[derive(Debug)]
struct UploadPreview {
    video_id: String,
    title: String,
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let _ = dotenvy::dotenv();

    match Cli::parse().command {
        Command::Validate {
            youtuber,
            max_videos,
            youtube_api_key,
        } => {
            let youtube = YouTubeClient::with_api_key(youtube_api_key)?;
            let youtuber = validate_youtuber(&youtube, &youtuber, max_videos).await?;
            print_validated_youtuber(&youtuber);
        }
        Command::InitTypesense {
            youtuber,
            max_videos,
            youtube_api_key,
            typesense_api_url,
            typesense_admin_api_key,
            collection,
        } => {
            let youtube = YouTubeClient::with_api_key(youtube_api_key)?;
            let youtuber = validate_youtuber(&youtube, &youtuber, max_videos).await?;
            print_validated_youtuber(&youtuber);

            let config = TypesenseAdminConfig::new(typesense_api_url, typesense_admin_api_key)
                .with_collection(collection.clone());
            let typesense = TypesenseAdminClient::new(config);
            match typesense.create_collection_if_missing().await? {
                CreateCollectionOutcome::Created(_) => {
                    println!("Created Typesense collection: {collection}");
                }
                CreateCollectionOutcome::AlreadyExists { .. } => {
                    println!("Typesense collection already exists: {collection}");
                }
            }

            println!("Ready to import transcript chunks for {}.", youtuber.handle);
        }
        Command::Search {
            query,
            youtuber,
            typesense_api_url,
            typesense_search_api_key,
            collection,
            per_page,
        } => {
            let config = TypesenseSearchConfig::new(typesense_api_url, typesense_search_api_key)
                .with_collection(collection);
            let typesense = TypesenseSearchClient::new(config);
            let mut search = TextSearchQuery::new(query).per_page(per_page);

            if let Some(youtuber) = youtuber {
                search = search.youtuber_handle(normalize_handle(&youtuber)?);
            }

            let response = typesense.search(&search).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&response).unwrap_or(response.to_string())
            );
        }
    }

    Ok(())
}

async fn validate_youtuber(
    youtube: &YouTubeClient,
    raw_handle: &str,
    max_videos: u8,
) -> Result<ValidatedYoutuber, CliError> {
    let handle = normalize_handle(raw_handle)?;
    let channel_page = youtube
        .list(ListResource::Channels)
        .query(
            Query::new()
                .part(["id", "snippet", "contentDetails"])
                .set("forHandle", &handle)
                .fields(
                    "items(id,snippet(title,customUrl),contentDetails/relatedPlaylists/uploads)",
                ),
        )
        .page()
        .await?;

    let channel = first_item(&channel_page).ok_or_else(|| {
        CliError::Message(format!("No YouTube channel found for handle {handle}"))
    })?;
    let channel_id = required_string(channel, "/id", "channel id")?;
    let title = required_string(channel, "/snippet/title", "channel title")?;
    let resolved_handle = optional_string(channel, "/snippet/customUrl").unwrap_or(handle);
    let uploads_playlist = optional_string(channel, "/contentDetails/relatedPlaylists/uploads");
    let (recent_uploads, uploads_warning) = match uploads_playlist.as_deref() {
        Some(uploads_playlist) => {
            match fetch_recent_uploads(youtube, uploads_playlist, max_videos).await {
                Ok(recent_uploads) => (recent_uploads, None),
                Err(error) => (
                    Vec::new(),
                    Some(format!(
                        "could not list recent uploads from playlist {uploads_playlist}: {error}"
                    )),
                ),
            }
        }
        None => (
            Vec::new(),
            Some("channel did not expose an uploads playlist".to_string()),
        ),
    };

    Ok(ValidatedYoutuber {
        handle: resolved_handle,
        channel_id,
        title,
        uploads_playlist,
        recent_uploads,
        uploads_warning,
    })
}

async fn fetch_recent_uploads(
    youtube: &YouTubeClient,
    uploads_playlist: &str,
    max_videos: u8,
) -> Result<Vec<UploadPreview>, CliError> {
    let page = youtube
        .list(ListResource::PlaylistItems)
        .query(
            Query::new()
                .part(["snippet", "contentDetails"])
                .set("playlistId", uploads_playlist)
                .max_results(max_videos.clamp(1, 50))
                .fields("items(snippet(title),contentDetails/videoId)"),
        )
        .page()
        .await?;

    Ok(page
        .items
        .iter()
        .filter_map(|item| {
            Some(UploadPreview {
                video_id: optional_string(item, "/contentDetails/videoId")?,
                title: optional_string(item, "/snippet/title")?,
            })
        })
        .collect())
}

fn normalize_handle(raw_handle: &str) -> Result<String, CliError> {
    let trimmed = raw_handle.trim();
    if trimmed.is_empty() {
        return Err(CliError::Message(
            "YouTube handle cannot be empty".to_string(),
        ));
    }

    let without_at = trimmed.strip_prefix('@').unwrap_or(trimmed);
    let valid = without_at
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || byte == b'-');

    if !valid {
        return Err(CliError::Message(format!(
            "Invalid YouTube handle: {raw_handle}. Expected something like @t3dotgg"
        )));
    }

    Ok(format!("@{without_at}"))
}

fn first_item(page: &Page<Value>) -> Option<&Value> {
    page.items.first()
}

fn required_string(value: &Value, pointer: &str, label: &str) -> Result<String, CliError> {
    optional_string(value, pointer)
        .ok_or_else(|| CliError::Message(format!("YouTube response was missing {label}")))
}

fn optional_string(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer)?.as_str().map(ToOwned::to_owned)
}

fn print_validated_youtuber(youtuber: &ValidatedYoutuber) {
    println!("Validated YouTuber: {}", youtuber.handle);
    println!("  title: {}", youtuber.title);
    println!("  channel_id: {}", youtuber.channel_id);
    match &youtuber.uploads_playlist {
        Some(uploads_playlist) => println!("  uploads_playlist: {uploads_playlist}"),
        None => println!("  uploads_playlist: unavailable"),
    }

    if let Some(warning) = &youtuber.uploads_warning {
        println!("  uploads warning: {warning}");
    }

    if youtuber.recent_uploads.is_empty() {
        println!("  recent uploads: none found or unavailable");
        return;
    }

    println!("  recent uploads:");
    for upload in &youtuber.recent_uploads {
        println!("    - {} {}", upload.video_id, upload.title);
    }
}
