use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::Value;
use std::path::PathBuf;
use thiserror::Error;
use youtube_client::{ListResource, Page, Query, YouTubeClient};
use youtube_ingest::{Chunker, TranscriptSource, VideoMetadataSource};
use youtube_sources::{YouTubeMetadataSource, YtDlpTranscriptSource};
use youtube_transcribe::{TranscriptionConfig, VoxtypeTranscriptSource};
use youtube_typesense::{
    CreateCollectionOutcome, DEFAULT_COLLECTION, TextSearchQuery, TypesenseAdminClient,
    TypesenseAdminConfig, TypesenseSearchClient, TypesenseSearchConfig, YoutuberDocument,
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
    Validate(ValidateCommand),
    #[command(about = "Validate a YouTuber and create the Typesense transcript index")]
    InitTypesense(InitTypesenseCommand),
    #[command(about = "Create/update the YouTuber document and report discoverable public videos")]
    IndexYoutuber(IndexYoutuberCommand),
    #[command(about = "Search transcript chunks using the Typesense search-only API key")]
    Search(SearchCommand),
}

#[derive(Debug, Args)]
struct ValidateCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[arg(long, default_value_t = 5, help = "Number of recent uploads to list")]
    max_videos: u8,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
}

#[derive(Debug, Args)]
struct InitTypesenseCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[arg(long, default_value_t = 5, help = "Number of recent uploads to verify")]
    max_videos: u8,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    typesense: TypesenseAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
}

#[derive(Debug, Args)]
struct IndexYoutuberCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    typesense: TypesenseAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(long, help = "Optional cap for a test indexing run")]
    max_videos: Option<usize>,
    #[arg(
        long,
        help = "Keep existing chunks for this YouTuber instead of wiping before import"
    )]
    no_wipe: bool,
    #[arg(long, value_enum, default_value_t = ContentKindFilter::All)]
    content_kind: ContentKindFilter,
    #[arg(
        long,
        help = "When captions are missing, extract audio with yt-dlp and transcribe locally with voxtype"
    )]
    transcribe_missing: bool,
    #[arg(long, default_value = "data/transcription-cache")]
    transcription_cache_dir: PathBuf,
    #[arg(long, default_value_t = 30)]
    transcription_chunk_seconds: u32,
    #[arg(
        long,
        help = "Optional voxtype engine override, e.g. whisper or parakeet"
    )]
    voxtype_engine: Option<String>,
}

#[derive(Debug, Args)]
struct SearchCommand {
    #[arg(value_name = "QUERY")]
    query: String,
    #[arg(long, help = "Optional YouTube handle filter, e.g. @t3dotgg")]
    youtuber: Option<String>,
    #[command(flatten)]
    typesense: TypesenseSearchArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(long, default_value_t = 10)]
    per_page: u8,
}

#[derive(Debug, Args)]
struct YoutuberArg {
    #[arg(value_name = "HANDLE", help = "YouTube handle, e.g. @t3dotgg")]
    handle: String,
}

#[derive(Debug, Args)]
struct YouTubeAuthArgs {
    #[arg(
        long,
        env = "YOUTUBE_API_KEY",
        hide_env_values = true,
        help = "YouTube Data API key"
    )]
    youtube_api_key: String,
}

#[derive(Debug, Args)]
struct TypesenseAdminArgs {
    #[arg(
        long,
        env = "TYPESENSE_API_URL",
        hide_env_values = true,
        help = "Typesense node/API URL, e.g. https://xxx.a1.typesense.net"
    )]
    typesense_api_url: String,
    #[arg(
        long,
        env = "TYPESENSE_ADMIN_API_KEY",
        hide_env_values = true,
        help = "Typesense Admin API key; used only for schema/import operations"
    )]
    typesense_admin_api_key: String,
}

#[derive(Debug, Args)]
struct TypesenseSearchArgs {
    #[arg(
        long,
        env = "TYPESENSE_API_URL",
        hide_env_values = true,
        help = "Typesense node/API URL, e.g. https://xxx.a1.typesense.net"
    )]
    typesense_api_url: String,
    #[arg(
        long,
        env = "TYPESENSE_SEARCH_API_KEY",
        hide_env_values = true,
        help = "Typesense search-only API key"
    )]
    typesense_search_api_key: String,
}

#[derive(Debug, Args)]
struct TranscriptCollectionArg {
    #[arg(
        long,
        env = "TYPESENSE_COLLECTION",
        hide_env_values = true,
        default_value = DEFAULT_COLLECTION
    )]
    collection: String,
}

impl YouTubeAuthArgs {
    fn client(self) -> Result<YouTubeClient, CliError> {
        Ok(YouTubeClient::with_api_key(self.youtube_api_key)?)
    }
}

impl TypesenseAdminArgs {
    fn client(&self, collection: impl Into<String>) -> TypesenseAdminClient {
        let config = TypesenseAdminConfig::new(
            self.typesense_api_url.clone(),
            self.typesense_admin_api_key.clone(),
        )
        .with_collection(collection);
        TypesenseAdminClient::new(config)
    }

    fn youtuber_client(&self) -> TypesenseAdminClient {
        TypesenseAdminClient::new(TypesenseAdminConfig::new(
            self.typesense_api_url.clone(),
            self.typesense_admin_api_key.clone(),
        ))
    }
}

impl TypesenseSearchArgs {
    fn client(&self, collection: impl Into<String>) -> TypesenseSearchClient {
        let config = TypesenseSearchConfig::new(
            self.typesense_api_url.clone(),
            self.typesense_search_api_key.clone(),
        )
        .with_collection(collection);
        TypesenseSearchClient::new(config)
    }
}

#[derive(Debug, Error)]
enum CliError {
    #[error("youtube api failed: {0}")]
    YouTube(#[from] youtube_client::YouTubeError),
    #[error("typesense api failed: {0}")]
    Typesense(#[from] youtube_typesense::TypesenseError),
    #[error("ingest failed: {0}")]
    Ingest(#[from] youtube_ingest::IngestError),
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

#[derive(Debug)]
struct DiscoveredVideo {
    video_id: String,
    title: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ContentKindFilter {
    All,
    Videos,
    Shorts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContentKind {
    Video,
    Short,
    Unknown,
}

#[derive(Debug, Default)]
struct IndexStats {
    discovered_videos: usize,
    attempted_videos: usize,
    indexed_videos: u32,
    skipped_videos: u32,
    kind_filtered_videos: u32,
    audio_transcribed_videos: u32,
    indexed_chunks: u32,
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let _ = dotenvy::dotenv();

    match Cli::parse().command {
        Command::Validate(command) => validate_command(command).await,
        Command::InitTypesense(command) => init_typesense_command(command).await,
        Command::IndexYoutuber(command) => index_youtuber_command(command).await,
        Command::Search(command) => search_command(command).await,
    }
}

async fn validate_command(command: ValidateCommand) -> Result<(), CliError> {
    let youtube = command.youtube.client()?;
    let youtuber =
        validate_youtuber(&youtube, &command.youtuber.handle, command.max_videos).await?;
    print_validated_youtuber(&youtuber);
    Ok(())
}

async fn init_typesense_command(command: InitTypesenseCommand) -> Result<(), CliError> {
    let youtube = command.youtube.client()?;
    let youtuber =
        validate_youtuber(&youtube, &command.youtuber.handle, command.max_videos).await?;
    print_validated_youtuber(&youtuber);

    let collection = command.collection.collection;
    let typesense = command.typesense.client(collection.clone());
    match typesense.create_collection_if_missing().await? {
        CreateCollectionOutcome::Created(_) => {
            println!("Created Typesense collection: {collection}");
        }
        CreateCollectionOutcome::AlreadyExists { .. } => {
            println!("Typesense collection already exists: {collection}");
        }
    }

    println!("Ready to import transcript chunks for {}.", youtuber.handle);
    Ok(())
}

async fn index_youtuber_command(command: IndexYoutuberCommand) -> Result<(), CliError> {
    let youtube = command.youtube.client()?;
    let youtuber = validate_youtuber(&youtube, &command.youtuber.handle, 5).await?;
    print_validated_youtuber(&youtuber);

    let public_videos = discover_public_videos(&youtube, &youtuber.channel_id).await?;
    println!("  public videos discovered: {}", public_videos.len());
    for video in public_videos.iter().take(5) {
        println!("    - {} {}", video.video_id, video.title);
    }

    let collection = command.collection.collection;
    let transcript_typesense = command.typesense.client(collection.clone());
    match transcript_typesense.create_collection_if_missing().await? {
        CreateCollectionOutcome::Created(_) => {
            println!("Created Typesense collection: {collection}")
        }
        CreateCollectionOutcome::AlreadyExists { .. } => {
            println!("Typesense collection already exists: {collection}")
        }
    }

    let youtuber_typesense = command.typesense.youtuber_client();
    match youtuber_typesense
        .create_youtubers_collection_if_missing()
        .await?
    {
        CreateCollectionOutcome::Created(_) => {
            println!("Created Typesense collection: youtube_youtubers")
        }
        CreateCollectionOutcome::AlreadyExists { .. } => {
            println!("Typesense collection already exists: youtube_youtubers")
        }
    }

    let youtuber_id = youtuber_id_from_handle(&youtuber.handle, &youtuber.channel_id);
    if command.no_wipe {
        println!("Keeping existing transcript chunks for {}", youtuber.handle);
    } else {
        println!(
            "Deleting existing transcript chunks for {} before reindex",
            youtuber.handle
        );
        let delete_response = transcript_typesense
            .delete_chunks_by_youtuber_id(&youtuber_id)
            .await?;
        println!(
            "Deleted existing chunks response: {}",
            serde_json::to_string(&delete_response).unwrap_or_else(|_| delete_response.to_string())
        );
    }

    let metadata_source = YouTubeMetadataSource::new(youtube.clone());
    let caption_source = YtDlpTranscriptSource::new();
    let transcribe_source = command.transcribe_missing.then(|| {
        VoxtypeTranscriptSource::with_config(
            TranscriptionConfig::default()
                .with_cache_dir(command.transcription_cache_dir.clone())
                .with_chunk_seconds(command.transcription_chunk_seconds)
                .with_voxtype_engine(command.voxtype_engine.clone()),
        )
    });
    let chunker = Chunker::five_seconds();
    let mut stats = IndexStats {
        discovered_videos: public_videos.len(),
        attempted_videos: command
            .max_videos
            .map_or(public_videos.len(), |max| max.min(public_videos.len())),
        ..IndexStats::default()
    };

    if stats.attempted_videos == 0 {
        println!(
            "No public videos were discoverable for {}. No transcript chunks were imported.",
            youtuber.handle
        );
    }

    for (index, video) in public_videos
        .iter()
        .take(stats.attempted_videos)
        .enumerate()
    {
        println!(
            "Indexing video {}/{}: {} {}",
            index + 1,
            stats.attempted_videos,
            video.video_id,
            video.title
        );

        let metadata = match metadata_source.metadata_for_video(&video.video_id).await {
            Ok(metadata) => metadata,
            Err(error) => {
                stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                println!("  skipped metadata: {error}");
                continue;
            }
        };

        let content_kind = content_kind_for_duration(metadata.duration_ms);
        if !command.content_kind.includes(content_kind) {
            stats.kind_filtered_videos = stats.kind_filtered_videos.saturating_add(1);
            println!("  skipped by content kind: {content_kind:?}");
            continue;
        }

        let transcript = match caption_source.transcript_for_video(&video.video_id).await {
            Ok(transcript) => transcript,
            Err(caption_error) => {
                let Some(transcribe_source) = &transcribe_source else {
                    stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                    println!("  skipped transcript: {caption_error}");
                    continue;
                };

                println!(
                    "  captions unavailable, trying local audio transcription: {caption_error}"
                );
                match transcribe_source
                    .transcript_for_video(&video.video_id)
                    .await
                {
                    Ok(transcript) => {
                        stats.audio_transcribed_videos =
                            stats.audio_transcribed_videos.saturating_add(1);
                        transcript
                    }
                    Err(transcribe_error) => {
                        stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                        println!(
                            "  skipped transcript: captions failed and local transcription failed: {transcribe_error}"
                        );
                        continue;
                    }
                }
            }
        };

        match chunker.chunks(&metadata, &transcript) {
            Ok(chunks) => {
                let chunk_count = chunks.len();
                transcript_typesense.import_chunks(&chunks).await?;
                stats.indexed_videos = stats.indexed_videos.saturating_add(1);
                stats.indexed_chunks = stats
                    .indexed_chunks
                    .saturating_add(saturating_u32(chunk_count));
                println!("  imported {chunk_count} transcript chunks");
            }
            Err(error) => {
                stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                println!("  skipped chunking: {error}");
            }
        }
    }

    let youtuber_doc = YoutuberDocument {
        id: youtuber_id,
        handle: youtuber.handle.clone(),
        name: youtuber.title.clone(),
        channel_id: youtuber.channel_id.clone(),
        uploads_playlist: youtuber.uploads_playlist.clone(),
        public_video_count: saturating_u32(stats.discovered_videos),
        indexed_video_count: stats.indexed_videos,
        indexed_chunk_count: stats.indexed_chunks,
    };
    youtuber_typesense.upsert_youtuber(&youtuber_doc).await?;
    println!("Upserted YouTuber document: {}", youtuber_doc.id);
    println!(
        "Index summary: discovered={}, attempted={}, indexed={}, skipped={}, kind_filtered={}, audio_transcribed={}, chunks={}",
        stats.discovered_videos,
        stats.attempted_videos,
        stats.indexed_videos,
        stats.skipped_videos,
        stats.kind_filtered_videos,
        stats.audio_transcribed_videos,
        stats.indexed_chunks
    );

    Ok(())
}

async fn search_command(command: SearchCommand) -> Result<(), CliError> {
    let typesense = command.typesense.client(command.collection.collection);
    let mut search = TextSearchQuery::new(command.query).per_page(command.per_page);

    if let Some(youtuber) = command.youtuber {
        search = search.youtuber_handle(normalize_handle(&youtuber)?);
    }

    let response = typesense.search(&search).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&response).unwrap_or(response.to_string())
    );
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

async fn discover_public_videos(
    youtube: &YouTubeClient,
    channel_id: &str,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let mut videos = Vec::new();
    let mut page_token = None;

    loop {
        let mut query = Query::new()
            .part(["snippet"])
            .set("channelId", channel_id)
            .set("type", "video")
            .set("order", "date")
            .max_results(50)
            .fields("nextPageToken,items(id/videoId,snippet/title)");

        if let Some(token) = page_token {
            query = query.page_token(token);
        }

        let page = youtube
            .list(ListResource::Search)
            .query(query)
            .page()
            .await?;
        for item in &page.items {
            if let Some(video) = discovered_video_from_search_item(item) {
                videos.push(video);
            }
        }

        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    Ok(videos)
}

fn discovered_video_from_search_item(item: &Value) -> Option<DiscoveredVideo> {
    Some(DiscoveredVideo {
        video_id: optional_string(item, "/id/videoId")?,
        title: optional_string(item, "/snippet/title")?,
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

impl ContentKindFilter {
    fn includes(self, content_kind: ContentKind) -> bool {
        match self {
            Self::All => true,
            Self::Videos => matches!(content_kind, ContentKind::Video | ContentKind::Unknown),
            Self::Shorts => matches!(content_kind, ContentKind::Short),
        }
    }
}

fn content_kind_for_duration(duration_ms: Option<youtube_ingest::DurationMs>) -> ContentKind {
    match duration_ms {
        Some(duration_ms) if duration_ms <= 60_000 => ContentKind::Short,
        Some(_) => ContentKind::Video,
        None => ContentKind::Unknown,
    }
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn youtuber_id_from_handle(handle: &str, channel_id: &str) -> String {
    let source = handle.strip_prefix('@').unwrap_or(handle);
    let source = if source.is_empty() {
        channel_id
    } else {
        source
    };
    let mut id = String::with_capacity(source.len());
    let mut previous_was_separator = false;

    for byte in source.bytes() {
        if byte.is_ascii_alphanumeric() {
            id.push(char::from(byte.to_ascii_lowercase()));
            previous_was_separator = false;
        } else if !previous_was_separator && !id.is_empty() {
            id.push('-');
            previous_was_separator = true;
        }
    }

    if id.ends_with('-') {
        id.pop();
    }
    id
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
