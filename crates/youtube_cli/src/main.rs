use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::Value;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::task::JoinSet;
use youtube_client::{ListResource, Page, Query, YouTubeClient};
use youtube_ingest::{
    Chunker, TranscriptChunkSource, TranscriptSegment, TranscriptSource, VideoMetadataSource,
};
use youtube_meilisearch::{
    CreateCollectionOutcome as MeiliCreateCollectionOutcome, MeilisearchAdminClient,
    MeilisearchAdminConfig, MeilisearchSearchClient, MeilisearchSearchConfig,
};
use youtube_search::{DEFAULT_COLLECTION, TextSearchQuery, YoutuberDocument};
use youtube_sources::{FallbackMetadataSource, YouTubeMetadataSource};
use youtube_transcribe::{TranscriptionConfig, VoxtypeTranscriptSource};
use youtube_typesense::{
    CreateCollectionOutcome as TypesenseCreateCollectionOutcome, TypesenseAdminClient,
    TypesenseAdminConfig, TypesenseSearchClient, TypesenseSearchConfig,
};
use youtube_ytdlp::{YtDlpTranscriptSource, YtDlpVideo};

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
    #[command(about = "Validate a YouTuber and create the search backend transcript index")]
    InitTypesense(InitTypesenseCommand),
    #[command(about = "Create/update the YouTuber document and report discoverable public videos")]
    IndexYoutuber(IndexYoutuberCommand),
    #[command(about = "Index YouTubers from a text file")]
    IndexYoutubers(IndexYoutubersCommand),
    #[command(about = "Benchmark indexing phases for one YouTuber")]
    BenchmarkIndexYoutuber(BenchmarkIndexYoutuberCommand),
    #[command(about = "Refresh the indexed YouTuber profile document without reindexing videos")]
    RefreshYoutuber(RefreshYoutuberCommand),
    #[command(about = "Refresh indexed YouTuber profile documents from a text file")]
    RefreshYoutubers(RefreshYoutubersCommand),
    #[command(about = "Reconcile one creator's indexed counts from backend chunks only")]
    RefreshYoutuberCounts(RefreshYoutuberCountsCommand),
    #[command(about = "Reconcile creator indexed counts from a text file without YouTube")]
    RefreshYoutuberCountsList(RefreshYoutuberCountsListCommand),
    #[command(about = "Remove a YouTuber document and transcript chunks from the search backend")]
    RemoveYoutuber(RemoveYoutuberCommand),
    #[command(about = "Copy canonical transcript chunks between search backends without YouTube")]
    CopyYoutuberChunks(CopyYoutuberChunksCommand),
    #[command(about = "Index one video by ID or URL")]
    IndexVideo(IndexVideoCommand),
    #[command(about = "Search transcript chunks using the selected search backend")]
    Search(SearchCommand),
}

#[derive(Clone, Debug, Args)]
struct ValidateCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[arg(long, default_value_t = 5, help = "Number of recent uploads to list")]
    max_videos: u8,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
}

#[derive(Clone, Debug, Args)]
struct InitTypesenseCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[arg(long, default_value_t = 5, help = "Number of recent uploads to verify")]
    max_videos: u8,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
struct RefreshYoutuberCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    search: SearchAdminArgs,
}

#[derive(Clone, Debug, Args)]
struct RefreshYoutubersCommand {
    #[arg(value_name = "LIST_FILE")]
    list_file: PathBuf,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    search: SearchAdminArgs,
}

#[derive(Clone, Debug, Args)]
struct RefreshYoutuberCountsCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
struct RefreshYoutuberCountsListCommand {
    #[arg(value_name = "LIST_FILE")]
    list_file: PathBuf,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
struct RemoveYoutuberCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
struct CopyYoutuberChunksCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[arg(long, default_value = "meilisearch")]
    source_backend: SearchBackend,
    #[arg(long, hide_env_values = true)]
    source_api_url: Option<String>,
    #[arg(long, hide_env_values = true)]
    source_admin_api_key: Option<String>,
    #[arg(long, default_value = "typesense")]
    target_backend: SearchBackend,
    #[arg(long, hide_env_values = true)]
    target_api_url: Option<String>,
    #[arg(long, hide_env_values = true)]
    target_admin_api_key: Option<String>,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(long, default_value_t = 500)]
    batch_size: usize,
    #[arg(long, help = "Delete the target creator chunks before copying")]
    replace: bool,
}

#[derive(Clone, Debug, Args)]
struct IndexYoutuberCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(long, help = "Optional cap for a test indexing run")]
    max_videos: Option<usize>,
    #[arg(
        long,
        help = "In update mode, stop scanning newest-first after this many consecutive already-indexed videos. Use 0 to disable"
    )]
    stop_after_existing: Option<usize>,
    #[arg(long, value_enum, default_value_t = IndexMode::Update)]
    mode: IndexMode,
    #[arg(
        long,
        help = "Acknowledge that rewipe deletes existing chunks before replacement data is imported"
    )]
    allow_destructive_rewipe: bool,
    #[arg(long, default_value_t = 4, help = "Concurrent videos to index")]
    jobs: usize,
    #[arg(
        long,
        default_value_t = 300,
        help = "Maximum seconds to spend on one video"
    )]
    video_timeout_seconds: u64,
    #[arg(
        long,
        help = "Skip videos that already have chunks in the search backend"
    )]
    skip_existing: bool,
    #[arg(
        long,
        help = "Keep existing chunks for this YouTuber instead of wiping before import"
    )]
    no_wipe: bool,
    #[arg(long, value_enum, default_value_t = ContentKindFilter::All)]
    content_kind: ContentKindFilter,
    #[arg(
        long,
        default_value_t = 10,
        help = "Transcript chunk window in seconds"
    )]
    chunk_seconds: u32,
    #[arg(
        long,
        help = "When captions are missing, extract audio with yt-dlp and transcribe locally with voxtype"
    )]
    transcribe_missing: bool,
    #[arg(long, default_value = "data/transcription-cache")]
    transcription_cache_dir: PathBuf,
    #[arg(long, default_value = "data/transcript-cache")]
    transcript_cache_dir: PathBuf,
    #[arg(long, help = "Disable the yt-dlp transcript cache")]
    no_transcript_cache: bool,
    #[arg(long, default_value_t = 30)]
    transcription_chunk_seconds: u32,
    #[arg(
        long,
        help = "Optional voxtype engine override, e.g. whisper or parakeet"
    )]
    voxtype_engine: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct IndexYoutubersCommand {
    #[arg(value_name = "LIST_FILE")]
    list_file: PathBuf,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(long, value_enum, default_value_t = IndexMode::Update)]
    mode: IndexMode,
    #[arg(
        long,
        help = "Acknowledge that rewipe deletes existing chunks before replacement data is imported"
    )]
    allow_destructive_rewipe: bool,
    #[arg(long, help = "Optional cap per creator for a test indexing run")]
    max_videos: Option<usize>,
    #[arg(
        long,
        help = "In update mode, stop scanning newest-first after this many consecutive already-indexed videos. Use 0 to disable"
    )]
    stop_after_existing: Option<usize>,
    #[arg(
        long,
        default_value_t = 4,
        help = "Concurrent videos to index per creator"
    )]
    jobs: usize,
    #[arg(
        long,
        default_value_t = 300,
        help = "Maximum seconds to spend on one video"
    )]
    video_timeout_seconds: u64,
    #[arg(long, value_enum, default_value_t = ContentKindFilter::All)]
    content_kind: ContentKindFilter,
    #[arg(
        long,
        default_value_t = 10,
        help = "Transcript chunk window in seconds"
    )]
    chunk_seconds: u32,
    #[arg(
        long,
        help = "When captions are missing, extract audio with yt-dlp and transcribe locally with voxtype"
    )]
    transcribe_missing: bool,
    #[arg(long, default_value = "data/transcription-cache")]
    transcription_cache_dir: PathBuf,
    #[arg(long, default_value = "data/transcript-cache")]
    transcript_cache_dir: PathBuf,
    #[arg(long, help = "Disable the yt-dlp transcript cache")]
    no_transcript_cache: bool,
    #[arg(long, default_value_t = 30)]
    transcription_chunk_seconds: u32,
    #[arg(
        long,
        help = "Optional voxtype engine override, e.g. whisper or parakeet"
    )]
    voxtype_engine: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct BenchmarkIndexYoutuberCommand {
    #[command(flatten)]
    youtuber: YoutuberArg,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(long, default_value_t = 10, help = "Videos to benchmark newest-first")]
    max_videos: usize,
    #[arg(long, help = "Import chunks while benchmarking")]
    import: bool,
    #[arg(
        long,
        help = "Skip videos that already have chunks in the search backend"
    )]
    skip_existing: bool,
    #[arg(long, value_enum, default_value_t = ContentKindFilter::All)]
    content_kind: ContentKindFilter,
    #[arg(
        long,
        default_value_t = 10,
        help = "Transcript chunk window in seconds"
    )]
    chunk_seconds: u32,
    #[arg(
        long,
        help = "When captions are missing, extract audio with yt-dlp and transcribe locally with voxtype"
    )]
    transcribe_missing: bool,
    #[arg(long, default_value = "data/transcription-cache")]
    transcription_cache_dir: PathBuf,
    #[arg(long, default_value = "data/transcript-cache")]
    transcript_cache_dir: PathBuf,
    #[arg(long, help = "Disable the yt-dlp transcript cache")]
    no_transcript_cache: bool,
    #[arg(long, default_value_t = 30)]
    transcription_chunk_seconds: u32,
    #[arg(
        long,
        help = "Optional voxtype engine override, e.g. whisper or parakeet"
    )]
    voxtype_engine: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct IndexVideoCommand {
    #[arg(value_name = "VIDEO_ID_OR_URL")]
    video: String,
    #[command(flatten)]
    youtube: YouTubeAuthArgs,
    #[command(flatten)]
    search: SearchAdminArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(
        long,
        help = "Keep existing chunks for this video instead of wiping before import"
    )]
    no_wipe: bool,
    #[arg(
        long,
        default_value_t = 10,
        help = "Transcript chunk window in seconds"
    )]
    chunk_seconds: u32,
    #[arg(
        long,
        help = "When captions are missing, extract audio with yt-dlp and transcribe locally with voxtype"
    )]
    transcribe_missing: bool,
    #[arg(long, default_value = "data/transcription-cache")]
    transcription_cache_dir: PathBuf,
    #[arg(long, default_value = "data/transcript-cache")]
    transcript_cache_dir: PathBuf,
    #[arg(long, help = "Disable the yt-dlp transcript cache")]
    no_transcript_cache: bool,
    #[arg(long, default_value_t = 30)]
    transcription_chunk_seconds: u32,
    #[arg(
        long,
        help = "Optional voxtype engine override, e.g. whisper or parakeet"
    )]
    voxtype_engine: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct SearchCommand {
    #[arg(value_name = "QUERY")]
    query: String,
    #[arg(long, help = "Optional YouTube handle filter, e.g. @t3dotgg")]
    youtuber: Option<String>,
    #[command(flatten)]
    search: SearchQueryArgs,
    #[command(flatten)]
    collection: TranscriptCollectionArg,
    #[arg(long, default_value_t = 10)]
    per_page: u8,
}

#[derive(Clone, Debug, Args)]
struct YoutuberArg {
    #[arg(value_name = "HANDLE", help = "YouTube handle, e.g. @t3dotgg")]
    handle: String,
}

#[derive(Clone, Debug, Args)]
struct YouTubeAuthArgs {
    #[arg(
        long,
        env = "YOUTUBE_API_KEY",
        hide_env_values = true,
        help = "YouTube Data API key"
    )]
    youtube_api_key: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct SearchAdminArgs {
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_BACKEND",
        default_value = "meilisearch",
        help = "Search backend to use: meilisearch or typesense"
    )]
    backend: SearchBackend,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_API_URL",
        hide_env_values = true,
        help = "Search backend admin URL. Defaults to MEILI_HTTP_ADDR or TYPESENSE_API_URL based on backend"
    )]
    api_url: Option<String>,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_ADMIN_API_KEY",
        hide_env_values = true,
        help = "Search backend admin key. Defaults to MEILI_MASTER_KEY or TYPESENSE_ADMIN_API_KEY based on backend"
    )]
    admin_api_key: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct SearchQueryArgs {
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_BACKEND",
        default_value = "meilisearch",
        help = "Search backend to use: meilisearch or typesense"
    )]
    backend: SearchBackend,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_API_URL",
        hide_env_values = true,
        help = "Search backend URL. Defaults to MEILI_HTTP_ADDR or TYPESENSE_API_URL based on backend"
    )]
    api_url: Option<String>,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_API_KEY",
        hide_env_values = true,
        help = "Search key. Defaults to MEILI_SEARCH_API_KEY/MEILI_MASTER_KEY or TYPESENSE_SEARCH_API_KEY based on backend"
    )]
    search_api_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SearchBackend {
    Meilisearch,
    Typesense,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum IndexMode {
    Update,
    Rewipe,
}

#[derive(Clone, Debug, Args)]
struct TranscriptCollectionArg {
    #[arg(
        long,
        env = "MEILI_TRANSCRIPTS_INDEX",
        hide_env_values = true,
        default_value = DEFAULT_COLLECTION
    )]
    collection: String,
}

impl YouTubeAuthArgs {
    fn client(self) -> Result<YouTubeClient, CliError> {
        let api_key = self.youtube_api_key.ok_or_else(|| {
            CliError::Message("missing YouTube Data API key (YOUTUBE_API_KEY)".to_string())
        })?;
        Ok(YouTubeClient::with_api_key(api_key)?)
    }

    fn optional_client(&self) -> Result<Option<YouTubeClient>, CliError> {
        self.youtube_api_key
            .as_ref()
            .map(|api_key| YouTubeClient::with_api_key(api_key.clone()).map_err(CliError::from))
            .transpose()
    }
}

impl SearchAdminArgs {
    fn resolved(&self) -> Result<ResolvedSearchAdminArgs, CliError> {
        let api_url = self.api_url.clone().or_else(|| match self.backend {
            SearchBackend::Meilisearch => std::env::var("MEILI_HTTP_ADDR").ok(),
            SearchBackend::Typesense => std::env::var("TYPESENSE_API_URL").ok(),
        });
        let admin_api_key = self.admin_api_key.clone().or_else(|| match self.backend {
            SearchBackend::Meilisearch => std::env::var("MEILI_MASTER_KEY").ok(),
            SearchBackend::Typesense => std::env::var("TYPESENSE_ADMIN_API_KEY").ok(),
        });

        Ok(ResolvedSearchAdminArgs {
            backend: self.backend,
            api_url: api_url.ok_or_else(|| {
                CliError::Message(format!(
                    "missing admin URL for {} backend",
                    self.backend.name()
                ))
            })?,
            admin_api_key: admin_api_key.ok_or_else(|| {
                CliError::Message(format!(
                    "missing admin key for {} backend",
                    self.backend.name()
                ))
            })?,
        })
    }

    fn client(&self, collection: impl Into<String>) -> Result<SearchAdminClient, CliError> {
        let collection = collection.into();
        let resolved = self.resolved()?;
        Ok(match resolved.backend {
            SearchBackend::Meilisearch => {
                SearchAdminClient::Meilisearch(MeilisearchAdminClient::new(
                    MeilisearchAdminConfig::new(
                        resolved.api_url.clone(),
                        resolved.admin_api_key.clone(),
                    )
                    .with_collection(collection),
                ))
            }
            SearchBackend::Typesense => SearchAdminClient::Typesense(TypesenseAdminClient::new(
                TypesenseAdminConfig::new(resolved.api_url.clone(), resolved.admin_api_key.clone())
                    .with_collection(collection),
            )),
        })
    }

    fn youtuber_client(&self) -> Result<SearchAdminClient, CliError> {
        let resolved = self.resolved()?;
        Ok(match resolved.backend {
            SearchBackend::Meilisearch => SearchAdminClient::Meilisearch(
                MeilisearchAdminClient::new(MeilisearchAdminConfig::new(
                    resolved.api_url.clone(),
                    resolved.admin_api_key.clone(),
                )),
            ),
            SearchBackend::Typesense => SearchAdminClient::Typesense(TypesenseAdminClient::new(
                TypesenseAdminConfig::new(resolved.api_url.clone(), resolved.admin_api_key.clone()),
            )),
        })
    }
}

impl SearchQueryArgs {
    fn resolved(&self) -> Result<ResolvedSearchArgs, CliError> {
        let api_url = self.api_url.clone().or_else(|| match self.backend {
            SearchBackend::Meilisearch => std::env::var("MEILI_HTTP_ADDR").ok(),
            SearchBackend::Typesense => std::env::var("TYPESENSE_API_URL").ok(),
        });
        let search_api_key = self.search_api_key.clone().or_else(|| match self.backend {
            SearchBackend::Meilisearch => std::env::var("MEILI_SEARCH_API_KEY")
                .ok()
                .or_else(|| std::env::var("MEILI_MASTER_KEY").ok()),
            SearchBackend::Typesense => std::env::var("TYPESENSE_SEARCH_API_KEY").ok(),
        });

        Ok(ResolvedSearchArgs {
            backend: self.backend,
            api_url: api_url.ok_or_else(|| {
                CliError::Message(format!(
                    "missing search URL for {} backend",
                    self.backend.name()
                ))
            })?,
            search_api_key: search_api_key.ok_or_else(|| {
                CliError::Message(format!(
                    "missing search key for {} backend",
                    self.backend.name()
                ))
            })?,
        })
    }

    fn client(&self, collection: impl Into<String>) -> Result<SearchClient, CliError> {
        let collection = collection.into();
        let resolved = self.resolved()?;
        Ok(match resolved.backend {
            SearchBackend::Meilisearch => SearchClient::Meilisearch(MeilisearchSearchClient::new(
                MeilisearchSearchConfig::new(
                    resolved.api_url.clone(),
                    resolved.search_api_key.clone(),
                )
                .with_collection(collection),
            )),
            SearchBackend::Typesense => SearchClient::Typesense(TypesenseSearchClient::new(
                TypesenseSearchConfig::new(
                    resolved.api_url.clone(),
                    resolved.search_api_key.clone(),
                )
                .with_collection(collection),
            )),
        })
    }
}

impl SearchBackend {
    fn name(self) -> &'static str {
        match self {
            Self::Meilisearch => "meilisearch",
            Self::Typesense => "typesense",
        }
    }
}

#[derive(Debug)]
struct ResolvedSearchAdminArgs {
    backend: SearchBackend,
    api_url: String,
    admin_api_key: String,
}

#[derive(Debug)]
struct ResolvedSearchArgs {
    backend: SearchBackend,
    api_url: String,
    search_api_key: String,
}

#[derive(Clone, Debug)]
enum SearchAdminClient {
    Meilisearch(MeilisearchAdminClient),
    Typesense(TypesenseAdminClient),
}

#[derive(Clone, Debug)]
enum SearchClient {
    Meilisearch(MeilisearchSearchClient),
    Typesense(TypesenseSearchClient),
}

enum CollectionOutcome {
    Created,
    AlreadyExists,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BackendChunkStats {
    indexed_videos: u64,
    indexed_chunks: u64,
}

impl SearchAdminClient {
    async fn create_collection_if_missing(&self) -> Result<CollectionOutcome, CliError> {
        match self {
            Self::Meilisearch(client) => Ok(match client.create_collection_if_missing().await? {
                MeiliCreateCollectionOutcome::Created(_) => CollectionOutcome::Created,
                MeiliCreateCollectionOutcome::AlreadyExists { .. } => {
                    CollectionOutcome::AlreadyExists
                }
            }),
            Self::Typesense(client) => Ok(match client.create_collection_if_missing().await? {
                TypesenseCreateCollectionOutcome::Created(_) => CollectionOutcome::Created,
                TypesenseCreateCollectionOutcome::AlreadyExists { .. } => {
                    CollectionOutcome::AlreadyExists
                }
            }),
        }
    }

    async fn create_youtubers_collection_if_missing(&self) -> Result<CollectionOutcome, CliError> {
        match self {
            Self::Meilisearch(client) => Ok(
                match client.create_youtubers_collection_if_missing().await? {
                    MeiliCreateCollectionOutcome::Created(_) => CollectionOutcome::Created,
                    MeiliCreateCollectionOutcome::AlreadyExists { .. } => {
                        CollectionOutcome::AlreadyExists
                    }
                },
            ),
            Self::Typesense(client) => Ok(
                match client.create_youtubers_collection_if_missing().await? {
                    TypesenseCreateCollectionOutcome::Created(_) => CollectionOutcome::Created,
                    TypesenseCreateCollectionOutcome::AlreadyExists { .. } => {
                        CollectionOutcome::AlreadyExists
                    }
                },
            ),
        }
    }

    async fn get_youtuber(&self, id: &str) -> Result<Option<YoutuberDocument>, CliError> {
        match self {
            Self::Meilisearch(client) => Ok(client.get_youtuber(id).await?.map(meili_youtuber)),
            Self::Typesense(client) => Ok(client.get_youtuber(id).await?.map(typesense_youtuber)),
        }
    }

    async fn upsert_youtuber(&self, youtuber: &YoutuberDocument) -> Result<Value, CliError> {
        match self {
            Self::Meilisearch(client) => client
                .upsert_youtuber(&meili_youtuber_from_shared(youtuber))
                .await
                .map_err(CliError::from),
            Self::Typesense(client) => client
                .upsert_youtuber(&typesense_youtuber_from_shared(youtuber))
                .await
                .map_err(CliError::from),
        }
    }

    async fn delete_youtuber(&self, id: &str) -> Result<Option<Value>, CliError> {
        match self {
            Self::Meilisearch(client) => client.delete_youtuber(id).await.map_err(CliError::from),
            Self::Typesense(client) => client.delete_youtuber(id).await.map_err(CliError::from),
        }
    }

    async fn delete_chunks_by_youtuber_id(&self, id: &str) -> Result<Value, CliError> {
        match self {
            Self::Meilisearch(client) => client
                .delete_chunks_by_youtuber_id(id)
                .await
                .map_err(CliError::from),
            Self::Typesense(client) => client
                .delete_chunks_by_youtuber_id(id)
                .await
                .map_err(CliError::from),
        }
    }

    async fn delete_chunks_by_video_id(&self, id: &str) -> Result<Value, CliError> {
        match self {
            Self::Meilisearch(client) => client
                .delete_chunks_by_video_id(id)
                .await
                .map_err(CliError::from),
            Self::Typesense(client) => client
                .delete_chunks_by_video_id(id)
                .await
                .map_err(CliError::from),
        }
    }

    async fn count_chunks_by_video_id(&self, id: &str) -> Result<u64, CliError> {
        match self {
            Self::Meilisearch(client) => client
                .count_chunks_by_video_id(id)
                .await
                .map_err(CliError::from),
            Self::Typesense(client) => client
                .count_chunks_by_video_id(id)
                .await
                .map_err(CliError::from),
        }
    }

    async fn chunk_stats_by_youtuber_id(&self, id: &str) -> Result<BackendChunkStats, CliError> {
        match self {
            Self::Meilisearch(client) => {
                let stats = client.chunk_stats_by_youtuber_id(id).await?;
                Ok(BackendChunkStats {
                    indexed_videos: stats.indexed_videos,
                    indexed_chunks: stats.indexed_chunks,
                })
            }
            Self::Typesense(client) => {
                let stats = client.chunk_stats_by_youtuber_id(id).await?;
                Ok(BackendChunkStats {
                    indexed_videos: stats.indexed_videos,
                    indexed_chunks: stats.indexed_chunks,
                })
            }
        }
    }

    async fn indexed_video_ids_by_youtuber_id(
        &self,
        id: &str,
    ) -> Result<HashSet<String>, CliError> {
        match self {
            Self::Meilisearch(client) => client
                .indexed_video_ids_by_youtuber_id(id)
                .await
                .map_err(CliError::from),
            Self::Typesense(client) => client
                .indexed_video_ids_by_youtuber_id(id)
                .await
                .map_err(CliError::from),
        }
    }

    async fn import_chunks_batched(
        &self,
        chunks: &[youtube_ingest::TranscriptChunk],
    ) -> Result<(), CliError> {
        match self {
            Self::Meilisearch(client) => client
                .import_chunks_batched(chunks)
                .await
                .map(|_| ())
                .map_err(CliError::from),
            Self::Typesense(client) => client
                .import_chunks_batched(chunks)
                .await
                .map(|_| ())
                .map_err(CliError::from),
        }
    }
}

impl SearchClient {
    async fn search(&self, query: &TextSearchQuery) -> Result<Value, CliError> {
        match self {
            Self::Meilisearch(client) => client
                .search(&meili_query_from_shared(query))
                .await
                .map_err(CliError::from),
            Self::Typesense(client) => client
                .search(&search_query_from_shared(query))
                .await
                .map_err(CliError::from),
        }
    }
}

#[derive(Debug, Error)]
enum CliError {
    #[error("youtube api failed: {0}")]
    YouTube(#[from] youtube_client::YouTubeError),
    #[error("search backend api failed: {0}")]
    Typesense(#[from] youtube_typesense::TypesenseError),
    #[error("meilisearch api failed: {0}")]
    Meilisearch(#[from] youtube_meilisearch::MeilisearchError),
    #[error("ingest failed: {0}")]
    Ingest(#[from] youtube_ingest::IngestError),
    #[error("{0}")]
    Message(String),
}

fn meili_youtuber(document: youtube_meilisearch::YoutuberDocument) -> YoutuberDocument {
    YoutuberDocument {
        id: document.id,
        handle: document.handle,
        name: document.name,
        channel_id: document.channel_id,
        uploads_playlist: document.uploads_playlist,
        public_video_count: document.public_video_count,
        indexed_video_count: document.indexed_video_count,
        indexed_chunk_count: document.indexed_chunk_count,
        avatar_url: document.avatar_url,
    }
}

fn typesense_youtuber(document: youtube_typesense::YoutuberDocument) -> YoutuberDocument {
    YoutuberDocument {
        id: document.id,
        handle: document.handle,
        name: document.name,
        channel_id: document.channel_id,
        uploads_playlist: document.uploads_playlist,
        public_video_count: document.public_video_count,
        indexed_video_count: document.indexed_video_count,
        indexed_chunk_count: document.indexed_chunk_count,
        avatar_url: document.avatar_url,
    }
}

fn meili_youtuber_from_shared(
    document: &YoutuberDocument,
) -> youtube_meilisearch::YoutuberDocument {
    youtube_meilisearch::YoutuberDocument {
        id: document.id.clone(),
        handle: document.handle.clone(),
        name: document.name.clone(),
        channel_id: document.channel_id.clone(),
        uploads_playlist: document.uploads_playlist.clone(),
        public_video_count: document.public_video_count,
        indexed_video_count: document.indexed_video_count,
        indexed_chunk_count: document.indexed_chunk_count,
        avatar_url: document.avatar_url.clone(),
    }
}

fn typesense_youtuber_from_shared(
    document: &YoutuberDocument,
) -> youtube_typesense::YoutuberDocument {
    youtube_typesense::YoutuberDocument {
        id: document.id.clone(),
        handle: document.handle.clone(),
        name: document.name.clone(),
        channel_id: document.channel_id.clone(),
        uploads_playlist: document.uploads_playlist.clone(),
        public_video_count: document.public_video_count,
        indexed_video_count: document.indexed_video_count,
        indexed_chunk_count: document.indexed_chunk_count,
        avatar_url: document.avatar_url.clone(),
    }
}

fn meili_query_from_shared(query: &TextSearchQuery) -> youtube_meilisearch::TextSearchQuery {
    youtube_meilisearch::TextSearchQuery {
        q: query.q.clone(),
        query_by: query.query_by.clone(),
        filter_by: query.filter_by.clone(),
        group_by: query.group_by.clone(),
        group_limit: query.group_limit,
        per_page: query.per_page,
        page: query.page,
        highlight_fields: query.highlight_fields.clone(),
        sort_by: query.sort_by.clone(),
    }
}

fn search_query_from_shared(query: &TextSearchQuery) -> youtube_typesense::TextSearchQuery {
    youtube_typesense::TextSearchQuery {
        q: query.q.clone(),
        query_by: query.query_by.clone(),
        filter_by: query.filter_by.clone(),
        group_by: query.group_by.clone(),
        group_limit: query.group_limit,
        per_page: query.per_page,
        page: query.page,
        highlight_fields: query.highlight_fields.clone(),
        sort_by: query.sort_by.clone(),
    }
}

#[derive(Debug)]
struct ValidatedYoutuber {
    handle: String,
    channel_id: String,
    title: String,
    uploads_playlist: Option<String>,
    avatar_url: Option<String>,
    recent_uploads: Vec<UploadPreview>,
    uploads_warning: Option<String>,
}

#[derive(Debug)]
struct UploadPreview {
    video_id: String,
    title: String,
}

#[derive(Clone, Debug)]
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
    existing_videos: u32,
    existing_chunks: u32,
    audio_transcribed_videos: u32,
    indexed_chunks: u32,
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let _ = dotenvy::dotenv();
    init_tracing();

    match Cli::parse().command {
        Command::Validate(command) => validate_command(command).await,
        Command::InitTypesense(command) => init_typesense_command(command).await,
        Command::IndexYoutuber(command) => index_youtuber_command(command).await,
        Command::IndexYoutubers(command) => index_youtubers_command(command).await,
        Command::BenchmarkIndexYoutuber(command) => benchmark_index_youtuber_command(command).await,
        Command::RefreshYoutuber(command) => refresh_youtuber_command(command).await,
        Command::RefreshYoutubers(command) => refresh_youtubers_command(command).await,
        Command::RefreshYoutuberCounts(command) => refresh_youtuber_counts_command(command).await,
        Command::RefreshYoutuberCountsList(command) => {
            refresh_youtuber_counts_list_command(command).await
        }
        Command::RemoveYoutuber(command) => remove_youtuber_command(command).await,
        Command::CopyYoutuberChunks(command) => copy_youtuber_chunks_command(command).await,
        Command::IndexVideo(command) => index_video_command(command).await,
        Command::Search(command) => search_command(command).await,
    }
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "youtube_cli=info,youtube_sources=info,youtube_transcribe=info".into());
    let subscriber = tracing_subscriber::fmt().with_env_filter(env_filter);

    if std::env::var("LOG_FORMAT").is_ok_and(|value| value.eq_ignore_ascii_case("json")) {
        subscriber.json().init();
    } else {
        subscriber.init();
    }
}

async fn validate_command(command: ValidateCommand) -> Result<(), CliError> {
    let youtube = command.youtube.client()?;
    let ytdlp = YtDlpTranscriptSource::new();
    let youtuber = validate_youtuber(
        Some(&youtube),
        &ytdlp,
        &command.youtuber.handle,
        command.max_videos,
    )
    .await?;
    print_validated_youtuber(&youtuber);
    Ok(())
}

async fn init_typesense_command(command: InitTypesenseCommand) -> Result<(), CliError> {
    let youtube = command.youtube.client()?;
    let ytdlp = YtDlpTranscriptSource::new();
    let youtuber = validate_youtuber(
        Some(&youtube),
        &ytdlp,
        &command.youtuber.handle,
        command.max_videos,
    )
    .await?;
    print_validated_youtuber(&youtuber);

    let collection = command.collection.collection;
    let search = command.search.client(collection.clone())?;
    match search.create_collection_if_missing().await? {
        CollectionOutcome::Created => {
            println!("Created search backend collection: {collection}");
        }
        CollectionOutcome::AlreadyExists => {
            println!("search backend collection already exists: {collection}");
        }
    }

    println!("Ready to import transcript chunks for {}.", youtuber.handle);
    Ok(())
}

async fn refresh_youtuber_command(command: RefreshYoutuberCommand) -> Result<(), CliError> {
    let youtube = command.youtube.optional_client()?;
    let ytdlp = YtDlpTranscriptSource::new();
    let youtuber = validate_youtuber(youtube.as_ref(), &ytdlp, &command.youtuber.handle, 5).await?;
    print_validated_youtuber(&youtuber);

    let search = command.search.youtuber_client()?;
    match search.create_youtubers_collection_if_missing().await? {
        CollectionOutcome::Created => {
            println!("Created search backend collection: youtube_youtubers")
        }
        CollectionOutcome::AlreadyExists => {
            println!("search backend collection already exists: youtube_youtubers")
        }
    }

    let youtuber_id = youtuber_id_from_handle(&youtuber.handle, &youtuber.channel_id);
    let existing = search.get_youtuber(&youtuber_id).await?;
    let public_video_count = match discover_videos_for_youtuber(youtube.as_ref(), &ytdlp, &youtuber)
        .await
    {
        Ok(videos) => saturating_u32(videos.len()),
        Err(error) => {
            tracing::warn!(handle = %youtuber.handle, %error, "failed to refresh public video count");
            existing.as_ref().map_or(0, |doc| doc.public_video_count)
        }
    };
    let youtuber_doc = build_youtuber_document_from_chunks(
        &search,
        &youtuber,
        &youtuber_id,
        public_video_count,
        existing.as_ref(),
    )
    .await?;
    search.upsert_youtuber(&youtuber_doc).await?;
    println!("Refreshed YouTuber document: {}", youtuber_doc.id);
    Ok(())
}

async fn build_youtuber_document_from_chunks(
    search: &SearchAdminClient,
    youtuber: &ValidatedYoutuber,
    youtuber_id: &str,
    public_video_count: u32,
    existing: Option<&YoutuberDocument>,
) -> Result<YoutuberDocument, CliError> {
    let chunk_stats = search.chunk_stats_by_youtuber_id(youtuber_id).await?;
    let indexed_video_count = saturating_u32(chunk_stats.indexed_videos);
    Ok(YoutuberDocument {
        id: youtuber_id.to_string(),
        handle: youtuber.handle.clone(),
        name: preserved_youtuber_name(youtuber, existing),
        channel_id: preserved_non_empty(&youtuber.channel_id, existing.map(|doc| &doc.channel_id)),
        uploads_playlist: youtuber
            .uploads_playlist
            .clone()
            .or_else(|| existing.and_then(|doc| doc.uploads_playlist.clone())),
        public_video_count: public_video_count.max(indexed_video_count),
        indexed_video_count,
        indexed_chunk_count: saturating_u32(chunk_stats.indexed_chunks),
        avatar_url: youtuber
            .avatar_url
            .clone()
            .or_else(|| existing.and_then(|doc| doc.avatar_url.clone())),
    })
}

async fn upsert_youtuber_document_from_chunks(
    search: &SearchAdminClient,
    youtuber: &ValidatedYoutuber,
    youtuber_id: &str,
    public_video_count: u32,
) -> Result<YoutuberDocument, CliError> {
    let existing = search.get_youtuber(youtuber_id).await?;
    let youtuber_doc = build_youtuber_document_from_chunks(
        search,
        youtuber,
        youtuber_id,
        public_video_count,
        existing.as_ref(),
    )
    .await?;
    search.upsert_youtuber(&youtuber_doc).await?;
    Ok(youtuber_doc)
}

fn preserved_youtuber_name(
    youtuber: &ValidatedYoutuber,
    existing: Option<&YoutuberDocument>,
) -> String {
    let candidate = youtuber.title.trim();
    let handle_name = youtuber.handle.trim_start_matches('@');
    if !candidate.is_empty() && !candidate.eq_ignore_ascii_case(handle_name) {
        return candidate.to_string();
    }

    existing
        .map(|doc| doc.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| youtuber.title.clone())
}

fn preserved_non_empty(candidate: &str, existing: Option<&String>) -> String {
    if candidate.trim().is_empty() {
        existing.cloned().unwrap_or_default()
    } else {
        candidate.to_string()
    }
}

async fn refresh_youtubers_command(command: RefreshYoutubersCommand) -> Result<(), CliError> {
    let handles = read_youtuber_list(&command.list_file)?;
    if handles.is_empty() {
        return Err(CliError::Message(format!(
            "no YouTube handles found in {}",
            command.list_file.display()
        )));
    }

    for handle in handles {
        refresh_youtuber_command(RefreshYoutuberCommand {
            youtuber: YoutuberArg { handle },
            youtube: command.youtube.clone(),
            search: command.search.clone(),
        })
        .await?;
    }

    Ok(())
}

async fn refresh_youtuber_counts_command(
    command: RefreshYoutuberCountsCommand,
) -> Result<(), CliError> {
    let handle = normalize_handle(&command.youtuber.handle)?;
    let youtuber_id = youtuber_id_from_handle(&handle, "");
    let search = command.search.client(command.collection.collection)?;
    refresh_youtuber_counts(&search, &handle, &youtuber_id).await
}

async fn refresh_youtuber_counts_list_command(
    command: RefreshYoutuberCountsListCommand,
) -> Result<(), CliError> {
    let handles = read_youtuber_list(&command.list_file)?;
    if handles.is_empty() {
        return Err(CliError::Message(format!(
            "no YouTube handles found in {}",
            command.list_file.display()
        )));
    }

    let search = command.search.client(command.collection.collection)?;
    for raw_handle in handles {
        let handle = normalize_handle(&raw_handle)?;
        let youtuber_id = youtuber_id_from_handle(&handle, "");
        refresh_youtuber_counts(&search, &handle, &youtuber_id).await?;
    }
    Ok(())
}

async fn refresh_youtuber_counts(
    search: &SearchAdminClient,
    handle: &str,
    youtuber_id: &str,
) -> Result<(), CliError> {
    let Some(mut youtuber) = search.get_youtuber(youtuber_id).await? else {
        println!("Skipped count refresh for {handle}: profile document is missing");
        return Ok(());
    };
    let stats = search.chunk_stats_by_youtuber_id(youtuber_id).await?;
    apply_backend_chunk_stats(&mut youtuber, stats);
    search.upsert_youtuber(&youtuber).await?;
    println!(
        "Refreshed counts for {handle}: {} videos / {} chunks",
        youtuber.indexed_video_count, youtuber.indexed_chunk_count
    );
    Ok(())
}

fn apply_backend_chunk_stats(youtuber: &mut YoutuberDocument, stats: BackendChunkStats) {
    youtuber.indexed_video_count = u32::try_from(stats.indexed_videos).unwrap_or(u32::MAX);
    youtuber.indexed_chunk_count = u32::try_from(stats.indexed_chunks).unwrap_or(u32::MAX);
    youtuber.public_video_count = youtuber
        .public_video_count
        .max(youtuber.indexed_video_count);
}

async fn remove_youtuber_command(command: RemoveYoutuberCommand) -> Result<(), CliError> {
    let handle = normalize_handle(&command.youtuber.handle)?;
    let youtuber_id = youtuber_id_from_handle(&handle, "");
    let transcript_search = command.search.client(command.collection.collection)?;
    let youtuber_search = command.search.youtuber_client()?;

    let delete_response = transcript_search
        .delete_chunks_by_youtuber_id(&youtuber_id)
        .await?;
    println!(
        "Deleted transcript chunks for {handle}: {}",
        serde_json::to_string(&delete_response).unwrap_or_else(|_| delete_response.to_string())
    );

    match youtuber_search.delete_youtuber(&youtuber_id).await? {
        Some(response) => println!(
            "Deleted YouTuber document for {handle}: {}",
            serde_json::to_string(&response).unwrap_or_else(|_| response.to_string())
        ),
        None => println!("No YouTuber document found for {handle}"),
    }

    Ok(())
}

async fn copy_youtuber_chunks_command(command: CopyYoutuberChunksCommand) -> Result<(), CliError> {
    if command.batch_size == 0 {
        return Err(CliError::Message(
            "copy batch size must be greater than zero".to_string(),
        ));
    }

    let handle = normalize_handle(&command.youtuber.handle)?;
    let youtuber_id = youtuber_id_from_handle(&handle, "");
    let collection = command.collection.collection;
    let source = SearchAdminArgs {
        backend: command.source_backend,
        api_url: command.source_api_url,
        admin_api_key: command.source_admin_api_key,
    }
    .client(collection.clone())?;
    let target = SearchAdminArgs {
        backend: command.target_backend,
        api_url: command.target_api_url,
        admin_api_key: command.target_admin_api_key,
    }
    .client(collection)?;

    target.create_collection_if_missing().await?;
    target.create_youtubers_collection_if_missing().await?;
    if command.replace {
        target.delete_chunks_by_youtuber_id(&youtuber_id).await?;
    }

    let copied = match &source {
        SearchAdminClient::Meilisearch(client) => {
            copy_chunks_from_source(client, &target, &youtuber_id, command.batch_size).await?
        }
        SearchAdminClient::Typesense(client) => {
            copy_chunks_from_source(client, &target, &youtuber_id, command.batch_size).await?
        }
    };

    let stats = target.chunk_stats_by_youtuber_id(&youtuber_id).await?;
    if let Some(mut youtuber) = source.get_youtuber(&youtuber_id).await? {
        youtuber.indexed_video_count = u32::try_from(stats.indexed_videos).unwrap_or(u32::MAX);
        youtuber.indexed_chunk_count = u32::try_from(stats.indexed_chunks).unwrap_or(u32::MAX);
        target.upsert_youtuber(&youtuber).await?;
    }

    println!(
        "Copied {copied} chunks for {handle}: {} -> {}; target has {} videos / {} chunks",
        command.source_backend.name(),
        command.target_backend.name(),
        stats.indexed_videos,
        stats.indexed_chunks
    );
    Ok(())
}

async fn copy_chunks_from_source<S: TranscriptChunkSource + Sync>(
    source: &S,
    target: &SearchAdminClient,
    youtuber_id: &str,
    batch_size: usize,
) -> Result<usize, CliError> {
    let mut offset = 0;
    let mut copied = 0;
    let mut pending = Vec::with_capacity(batch_size);

    loop {
        let mut page = source
            .chunks_page_by_youtuber_id(youtuber_id, offset, batch_size)
            .await?;
        if page.chunks.is_empty() {
            break;
        }

        for chunk in &mut page.chunks {
            chunk.youtuber_id = youtuber_id.to_string();
        }

        let page_len = page.chunks.len();
        pending.append(&mut page.chunks);
        copied += page_len;
        offset = page.offset + page_len;
        let finished = offset >= page.total;
        if pending.len() >= batch_size || finished {
            target.import_chunks_batched(&pending).await?;
            pending.clear();
            println!("Copied {copied}/{} chunks for {youtuber_id}", page.total);
        }
        if finished {
            break;
        }
    }

    if !pending.is_empty() {
        target.import_chunks_batched(&pending).await?;
    }

    Ok(copied)
}

async fn index_youtuber_command(command: IndexYoutuberCommand) -> Result<(), CliError> {
    validate_rewipe_safety(
        command.mode,
        command.no_wipe,
        command.allow_destructive_rewipe,
    )?;
    let youtube = command.youtube.optional_client()?;
    let caption_source = transcript_source(
        command.no_transcript_cache,
        command.transcript_cache_dir.clone(),
    );
    let youtuber = validate_youtuber(
        youtube.as_ref(),
        &caption_source,
        &command.youtuber.handle,
        5,
    )
    .await?;
    print_validated_youtuber(&youtuber);

    let public_videos =
        discover_videos_for_youtuber(youtube.as_ref(), &caption_source, &youtuber).await?;
    tracing::info!(
        handle = %youtuber.handle,
        channel_id = %youtuber.channel_id,
        public_videos = public_videos.len(),
        "discovered public videos"
    );
    println!("  public videos discovered: {}", public_videos.len());
    for video in public_videos.iter().take(5) {
        println!("    - {} {}", video.video_id, video.title);
    }

    let collection = command.collection.collection;
    let transcript_search = command.search.client(collection.clone())?;
    match transcript_search.create_collection_if_missing().await? {
        CollectionOutcome::Created => {
            println!("Created search backend collection: {collection}")
        }
        CollectionOutcome::AlreadyExists => {
            println!("search backend collection already exists: {collection}")
        }
    }

    let youtuber_search = command.search.youtuber_client()?;
    match youtuber_search
        .create_youtubers_collection_if_missing()
        .await?
    {
        CollectionOutcome::Created => {
            println!("Created search backend collection: youtube_youtubers")
        }
        CollectionOutcome::AlreadyExists => {
            println!("search backend collection already exists: youtube_youtubers")
        }
    }

    let youtuber_id = youtuber_id_from_handle(&youtuber.handle, &youtuber.channel_id);
    let no_wipe = command.no_wipe || command.mode == IndexMode::Update;
    let skip_existing = command.skip_existing || command.mode == IndexMode::Update;

    if no_wipe {
        println!("Keeping existing transcript chunks for {}", youtuber.handle);
    } else {
        println!(
            "Deleting existing transcript chunks for {} before reindex",
            youtuber.handle
        );
        let delete_response = transcript_search
            .delete_chunks_by_youtuber_id(&youtuber_id)
            .await?;
        println!(
            "Deleted existing chunks response: {}",
            serde_json::to_string(&delete_response).unwrap_or_else(|_| delete_response.to_string())
        );
    }

    let metadata_source = metadata_source(youtube.as_ref(), &caption_source);
    let transcribe_source = command.transcribe_missing.then(|| {
        VoxtypeTranscriptSource::with_config(
            TranscriptionConfig::default()
                .with_cache_dir(command.transcription_cache_dir.clone())
                .with_chunk_seconds(command.transcription_chunk_seconds)
                .with_voxtype_engine(command.voxtype_engine.clone()),
        )
    });
    let chunker = Chunker::new(command.chunk_seconds);
    let mut stats = IndexStats {
        discovered_videos: public_videos.len(),
        ..IndexStats::default()
    };
    let stop_after_existing = normalized_stop_after_existing(command.stop_after_existing);
    let videos_to_index = prepare_videos_to_index(
        &public_videos,
        &transcript_search,
        skip_existing,
        command.max_videos,
        stop_after_existing,
        &mut stats,
    )
    .await?;

    if stats.attempted_videos == 0 {
        println!(
            "No candidate videos selected for {}. No transcript chunks were imported.",
            youtuber.handle
        );
    }
    let jobs = command.jobs.clamp(1, 16);
    println!("Indexing with {jobs} concurrent job(s)");
    let use_worker_existing_checks = should_use_worker_existing_checks(
        skip_existing,
        stop_after_existing,
        videos_to_index.len(),
    );
    let existing_video_ids = if skip_existing && !use_worker_existing_checks {
        if videos_to_index.is_empty() {
            HashSet::new()
        } else {
            println!("Loading existing indexed video ids for {}", youtuber.handle);
            let existing = transcript_search
                .indexed_video_ids_by_youtuber_id(&youtuber_id)
                .await?;
            println!(
                "Skipping videos that already have transcript chunks ({} cached ids)",
                existing.len()
            );
            existing
        }
    } else {
        HashSet::new()
    };
    let existing_video_count = existing_video_ids.len();
    let videos_to_index = videos_to_index
        .into_iter()
        .filter(|video| {
            if existing_video_ids.contains(&video.video_id) {
                stats.existing_videos = stats.existing_videos.saturating_add(1);
                false
            } else {
                true
            }
        })
        .collect::<Vec<_>>();
    if existing_video_count > 0 {
        println!(
            "Skipped {} already-indexed video(s) before starting workers",
            stats.existing_videos
        );
        match upsert_youtuber_document_from_chunks(
            &youtuber_search,
            &youtuber,
            &youtuber_id,
            saturating_u32(stats.discovered_videos),
        )
        .await
        {
            Ok(youtuber_doc) => {
                println!(
                    "Updated YouTuber document: {} ({} videos, {} chunks)",
                    youtuber_doc.id,
                    youtuber_doc.indexed_video_count,
                    youtuber_doc.indexed_chunk_count
                );
            }
            Err(error) => {
                tracing::warn!(handle = %youtuber.handle, %error, "failed to update YouTuber document after existing-video scan");
                println!("Skipped YouTuber document refresh after existing-video scan: {error}");
            }
        }
    } else if use_worker_existing_checks {
        println!(
            "Using per-video existing checks for {} recent candidate video(s)",
            videos_to_index.len()
        );
    }

    let mut tasks = JoinSet::new();
    let mut next_video = 0;
    let mut active_jobs = 0usize;
    let mut completed_videos = 0usize;
    let mut last_youtuber_doc_refresh_completed = 0usize;
    let public_video_count = saturating_u32(stats.discovered_videos);

    while next_video < videos_to_index.len() || active_jobs > 0 {
        while active_jobs < jobs && next_video < videos_to_index.len() {
            let video = videos_to_index[next_video].clone();
            let index = next_video + 1;
            next_video += 1;
            active_jobs += 1;

            let timeout_seconds = command.video_timeout_seconds;
            let attempted_videos = stats.attempted_videos;
            let metadata_source = metadata_source.clone();
            let caption_source = caption_source.clone();
            let transcribe_source = transcribe_source.clone();
            let chunker = chunker.clone();
            let transcript_search = transcript_search.clone();
            let worker_youtuber_id = youtuber_id.clone();
            let content_kind = command.content_kind;
            let timeout_video_id = video.video_id.clone();
            let timeout_title = video.title.clone();
            tasks.spawn(async move {
                tokio::time::timeout(
                    Duration::from_secs(timeout_seconds),
                    index_discovered_video(
                        index,
                        attempted_videos,
                        video,
                        worker_youtuber_id,
                        metadata_source,
                        caption_source,
                        transcribe_source,
                        chunker,
                        transcript_search,
                        content_kind,
                        use_worker_existing_checks,
                    ),
                )
                .await
                .unwrap_or_else(|_| {
                    Ok(IndexVideoOutcome::Skipped {
                        video_id: timeout_video_id,
                        title: timeout_title,
                        reason: "timeout".to_string(),
                    })
                })
            });
        }

        let Some(result) = tasks.join_next().await else {
            break;
        };
        active_jobs = active_jobs.saturating_sub(1);
        completed_videos += 1;
        let attempted_videos = stats.attempted_videos;
        match result {
            Ok(Ok(outcome)) => {
                apply_index_outcome(&mut stats, outcome, completed_videos, attempted_videos);
            }
            Ok(Err(error)) => {
                stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                tracing::warn!(%error, "skipped video after indexing task failed");
                println!(
                    "Done {completed_videos}/{attempted_videos}: skipped indexing error: {error}"
                );
            }
            Err(error) => {
                stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                tracing::warn!(%error, "skipped video after indexing task panicked");
                println!("Done {completed_videos}/{attempted_videos}: skipped task error: {error}");
            }
        }

        let run_indexed_videos = stats.indexed_videos.saturating_add(stats.existing_videos);
        let run_indexed_chunks = stats.indexed_chunks.saturating_add(stats.existing_chunks);
        let should_refresh_youtuber_doc = run_indexed_videos > 0
            && run_indexed_chunks > 0
            && (last_youtuber_doc_refresh_completed == 0
                || completed_videos.saturating_sub(last_youtuber_doc_refresh_completed) >= 25);
        if should_refresh_youtuber_doc {
            match upsert_youtuber_document_from_chunks(
                &youtuber_search,
                &youtuber,
                &youtuber_id,
                public_video_count,
            )
            .await
            {
                Ok(youtuber_doc) => {
                    last_youtuber_doc_refresh_completed = completed_videos;
                    println!(
                        "Updated YouTuber document: {} ({} videos, {} chunks)",
                        youtuber_doc.id,
                        youtuber_doc.indexed_video_count,
                        youtuber_doc.indexed_chunk_count
                    );
                }
                Err(error) => {
                    tracing::warn!(handle = %youtuber.handle, %error, "failed to update YouTuber document during indexing");
                    println!("Skipped YouTuber document refresh during indexing: {error}");
                }
            }
        }
    }

    let run_indexed_videos = stats.indexed_videos.saturating_add(stats.existing_videos);
    let run_indexed_chunks = stats.indexed_chunks.saturating_add(stats.existing_chunks);
    print_index_summary(&stats);

    if stats.attempted_videos > 0 && (run_indexed_videos == 0 || run_indexed_chunks == 0) {
        if skip_existing && stats.existing_videos > 0 {
            let youtuber_doc = upsert_youtuber_document_from_chunks(
                &youtuber_search,
                &youtuber,
                &youtuber_id,
                public_video_count,
            )
            .await?;
            println!(
                "Update pass found no new searchable chunks for {}; kept existing indexed data ({} videos, {} chunks)",
                youtuber.handle, youtuber_doc.indexed_video_count, youtuber_doc.indexed_chunk_count
            );
            return Ok(());
        }

        if let Err(error) = youtuber_search.delete_youtuber(&youtuber_id).await {
            tracing::warn!(handle = %youtuber.handle, %error, "failed to remove unsearchable YouTuber document");
        } else {
            println!("Removed unsearchable YouTuber document: {youtuber_id}");
        }
        return Err(CliError::Message(format!(
            "No searchable transcript chunks were imported for {}. attempted={}, skipped={}, kind_filtered={}. Check yt-dlp/caption errors above.",
            youtuber.handle,
            stats.attempted_videos,
            stats.skipped_videos,
            stats.kind_filtered_videos
        )));
    }

    let youtuber_doc = upsert_youtuber_document_from_chunks(
        &youtuber_search,
        &youtuber,
        &youtuber_id,
        public_video_count,
    )
    .await?;
    println!("Upserted YouTuber document: {}", youtuber_doc.id);

    Ok(())
}

async fn index_youtubers_command(command: IndexYoutubersCommand) -> Result<(), CliError> {
    let handles = read_youtuber_list(&command.list_file)?;
    if handles.is_empty() {
        return Err(CliError::Message(format!(
            "no YouTube handles found in {}",
            command.list_file.display()
        )));
    }

    for handle in handles {
        let command = IndexYoutuberCommand {
            youtuber: YoutuberArg { handle },
            youtube: command.youtube.clone(),
            search: command.search.clone(),
            collection: command.collection.clone(),
            max_videos: command.max_videos,
            mode: command.mode,
            allow_destructive_rewipe: command.allow_destructive_rewipe,
            jobs: command.jobs,
            video_timeout_seconds: command.video_timeout_seconds,
            stop_after_existing: command.stop_after_existing,
            skip_existing: command.mode == IndexMode::Update,
            no_wipe: command.mode == IndexMode::Update,
            content_kind: command.content_kind,
            chunk_seconds: command.chunk_seconds,
            transcribe_missing: command.transcribe_missing,
            transcription_cache_dir: command.transcription_cache_dir.clone(),
            transcript_cache_dir: command.transcript_cache_dir.clone(),
            no_transcript_cache: command.no_transcript_cache,
            transcription_chunk_seconds: command.transcription_chunk_seconds,
            voxtype_engine: command.voxtype_engine.clone(),
        };
        index_youtuber_command(command).await?;
    }

    Ok(())
}

fn validate_rewipe_safety(
    mode: IndexMode,
    no_wipe: bool,
    allow_destructive_rewipe: bool,
) -> Result<(), CliError> {
    if mode == IndexMode::Rewipe && !no_wipe && !allow_destructive_rewipe {
        return Err(CliError::Message(
            "rewipe deletes existing chunks before replacement captions are confirmed; use update or backend copy when possible, or pass --allow-destructive-rewipe after verifying a recoverable source"
                .to_string(),
        ));
    }
    Ok(())
}

async fn benchmark_index_youtuber_command(
    command: BenchmarkIndexYoutuberCommand,
) -> Result<(), CliError> {
    let youtube = command.youtube.optional_client()?;
    let caption_source = transcript_source(
        command.no_transcript_cache,
        command.transcript_cache_dir.clone(),
    );
    let validate_started = Instant::now();
    let youtuber = validate_youtuber(
        youtube.as_ref(),
        &caption_source,
        &command.youtuber.handle,
        5,
    )
    .await?;
    let validate_elapsed = validate_started.elapsed();
    print_validated_youtuber(&youtuber);

    let discover_started = Instant::now();
    let public_videos =
        discover_videos_for_youtuber(youtube.as_ref(), &caption_source, &youtuber).await?;
    let discover_elapsed = discover_started.elapsed();
    let videos = public_videos
        .iter()
        .take(command.max_videos)
        .cloned()
        .collect::<Vec<_>>();

    let search = command.search.client(command.collection.collection)?;
    let metadata_source = metadata_source(youtube.as_ref(), &caption_source);
    let transcribe_source = command.transcribe_missing.then(|| {
        VoxtypeTranscriptSource::with_config(
            TranscriptionConfig::default()
                .with_cache_dir(command.transcription_cache_dir.clone())
                .with_chunk_seconds(command.transcription_chunk_seconds)
                .with_voxtype_engine(command.voxtype_engine.clone()),
        )
    });
    let chunker = Chunker::new(command.chunk_seconds);
    let youtuber_id = youtuber_id_from_handle(&youtuber.handle, &youtuber.channel_id);

    println!(
        "Index benchmark: handle={} discovered={} sampled={} import={}",
        youtuber.handle,
        public_videos.len(),
        videos.len(),
        command.import
    );
    println!(
        "validate_ms\tdiscover_ms\t{}",
        BenchmarkIndexVideoTiming::header()
    );

    let mut summary = BenchmarkIndexSummary::default();
    for (offset, video) in videos.into_iter().enumerate() {
        let timing = benchmark_index_video(
            offset + 1,
            video,
            &youtuber_id,
            &metadata_source,
            &caption_source,
            transcribe_source.as_ref(),
            &chunker,
            &search,
            command.content_kind,
            command.skip_existing,
            command.import,
        )
        .await;
        summary.record(&timing);
        println!(
            "{}\t{}\t{}",
            millis(validate_elapsed),
            millis(discover_elapsed),
            timing.row()
        );
    }

    println!("{}", summary.row());
    Ok(())
}

fn read_youtuber_list(path: &PathBuf) -> Result<Vec<String>, CliError> {
    let body = std::fs::read_to_string(path).map_err(|error| {
        CliError::Message(format!("failed to read {}: {error}", path.display()))
    })?;
    let handles = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split('|').next())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    Ok(handles)
}

async fn prepare_videos_to_index(
    public_videos: &[DiscoveredVideo],
    search: &SearchAdminClient,
    skip_existing: bool,
    max_videos: Option<usize>,
    stop_after_existing: Option<usize>,
    stats: &mut IndexStats,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let limit = max_videos.unwrap_or(public_videos.len());
    let candidates = public_videos.iter().take(limit);

    if !skip_existing || stop_after_existing.is_none() {
        let videos = candidates.cloned().collect::<Vec<_>>();
        stats.attempted_videos = videos.len();
        return Ok(videos);
    }

    let stop_after_existing = stop_after_existing.expect("checked above");
    let mut videos = Vec::new();
    let mut consecutive_existing = 0usize;

    for video in candidates.cloned() {
        let existing_chunks = search.count_chunks_by_video_id(&video.video_id).await?;
        if select_missing_video_until_existing_tail(
            video,
            existing_chunks,
            stop_after_existing,
            stats,
            &mut videos,
            &mut consecutive_existing,
        ) {
            break;
        }
    }

    println!(
        "Selected {} missing video(s) after checking {} newest upload(s)",
        videos.len(),
        stats.attempted_videos
    );
    Ok(videos)
}

fn normalized_stop_after_existing(stop_after_existing: Option<usize>) -> Option<usize> {
    stop_after_existing.filter(|count| *count > 0)
}

fn should_use_worker_existing_checks(
    skip_existing: bool,
    stop_after_existing: Option<usize>,
    candidate_count: usize,
) -> bool {
    skip_existing && stop_after_existing.is_none() && (1..=100).contains(&candidate_count)
}

fn select_missing_video_until_existing_tail(
    video: DiscoveredVideo,
    existing_chunks: u64,
    stop_after_existing: usize,
    stats: &mut IndexStats,
    videos: &mut Vec<DiscoveredVideo>,
    consecutive_existing: &mut usize,
) -> bool {
    stats.attempted_videos = stats.attempted_videos.saturating_add(1);
    if existing_chunks > 0 {
        stats.existing_videos = stats.existing_videos.saturating_add(1);
        stats.existing_chunks = stats
            .existing_chunks
            .saturating_add(saturating_u32(existing_chunks));
        *consecutive_existing = (*consecutive_existing).saturating_add(1);
        if *consecutive_existing >= stop_after_existing {
            let stopped_after = *consecutive_existing;
            println!(
                "Stopped update scan after {stopped_after} consecutive already-indexed video(s)"
            );
            return true;
        }
    } else {
        *consecutive_existing = 0;
        videos.push(video);
    }

    false
}

fn transcript_source(no_cache: bool, cache_dir: PathBuf) -> YtDlpTranscriptSource {
    let source = YtDlpTranscriptSource::new();
    if no_cache {
        source
    } else {
        source.with_cache_dir(cache_dir)
    }
}

fn metadata_source(
    youtube: Option<&YouTubeClient>,
    caption_source: &YtDlpTranscriptSource,
) -> Arc<FallbackMetadataSource> {
    let mut source = FallbackMetadataSource::new().with_source(caption_source.clone());
    if let Some(youtube) = youtube.cloned() {
        source.push(YouTubeMetadataSource::new(youtube));
    }
    Arc::new(source)
}

#[derive(Debug)]
enum IndexVideoOutcome {
    Indexed {
        video_id: String,
        title: String,
        chunks: usize,
        audio_transcribed: bool,
    },
    Existing {
        video_id: String,
        title: String,
        chunks: u64,
    },
    KindFiltered {
        video_id: String,
        title: String,
        content_kind: ContentKind,
    },
    Skipped {
        video_id: String,
        title: String,
        reason: String,
    },
}

#[derive(Debug)]
struct BenchmarkIndexVideoTiming {
    index: usize,
    video_id: String,
    title: String,
    existing_chunks: u64,
    chunks: usize,
    skipped: Option<String>,
    metadata_ms: u128,
    existing_ms: u128,
    transcript_ms: u128,
    chunk_ms: u128,
    import_ms: u128,
    total_ms: u128,
}

impl BenchmarkIndexVideoTiming {
    fn header() -> &'static str {
        "index\tvideo_id\texisting_chunks\tchunks\tmetadata_ms\texisting_ms\ttranscript_ms\tchunk_ms\timport_ms\ttotal_ms\tskipped\ttitle"
    }

    fn row(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.index,
            self.video_id,
            self.existing_chunks,
            self.chunks,
            self.metadata_ms,
            self.existing_ms,
            self.transcript_ms,
            self.chunk_ms,
            self.import_ms,
            self.total_ms,
            self.skipped.as_deref().unwrap_or(""),
            self.title.replace('\t', " ")
        )
    }
}

#[derive(Default)]
struct BenchmarkIndexSummary {
    videos: usize,
    skipped: usize,
    chunks: usize,
    existing_chunks: u64,
    metadata_ms: u128,
    existing_ms: u128,
    transcript_ms: u128,
    chunk_ms: u128,
    import_ms: u128,
    total_ms: u128,
}

impl BenchmarkIndexSummary {
    fn record(&mut self, timing: &BenchmarkIndexVideoTiming) {
        self.videos = self.videos.saturating_add(1);
        if timing.skipped.is_some() {
            self.skipped = self.skipped.saturating_add(1);
        }
        self.chunks = self.chunks.saturating_add(timing.chunks);
        self.existing_chunks = self.existing_chunks.saturating_add(timing.existing_chunks);
        self.metadata_ms = self.metadata_ms.saturating_add(timing.metadata_ms);
        self.existing_ms = self.existing_ms.saturating_add(timing.existing_ms);
        self.transcript_ms = self.transcript_ms.saturating_add(timing.transcript_ms);
        self.chunk_ms = self.chunk_ms.saturating_add(timing.chunk_ms);
        self.import_ms = self.import_ms.saturating_add(timing.import_ms);
        self.total_ms = self.total_ms.saturating_add(timing.total_ms);
    }

    fn row(&self) -> String {
        format!(
            "summary\tvideos={}\tskipped={}\tchunks={}\texisting_chunks={}\tmetadata_ms={}\texisting_ms={}\ttranscript_ms={}\tchunk_ms={}\timport_ms={}\ttotal_ms={}",
            self.videos,
            self.skipped,
            self.chunks,
            self.existing_chunks,
            self.metadata_ms,
            self.existing_ms,
            self.transcript_ms,
            self.chunk_ms,
            self.import_ms,
            self.total_ms
        )
    }
}

#[allow(clippy::too_many_arguments)]
async fn benchmark_index_video(
    index: usize,
    video: DiscoveredVideo,
    youtuber_id: &str,
    metadata_source: &Arc<FallbackMetadataSource>,
    caption_source: &YtDlpTranscriptSource,
    transcribe_source: Option<&VoxtypeTranscriptSource>,
    chunker: &Chunker,
    search: &SearchAdminClient,
    content_kind_filter: ContentKindFilter,
    skip_existing: bool,
    import: bool,
) -> BenchmarkIndexVideoTiming {
    let total_started = Instant::now();
    let mut timing = BenchmarkIndexVideoTiming {
        index,
        video_id: video.video_id.clone(),
        title: video.title.clone(),
        existing_chunks: 0,
        chunks: 0,
        skipped: None,
        metadata_ms: 0,
        existing_ms: 0,
        transcript_ms: 0,
        chunk_ms: 0,
        import_ms: 0,
        total_ms: 0,
    };

    if skip_existing {
        let started = Instant::now();
        match search.count_chunks_by_video_id(&video.video_id).await {
            Ok(existing_chunks) => {
                timing.existing_chunks = existing_chunks;
                timing.existing_ms = millis(started.elapsed());
                if existing_chunks > 0 {
                    timing.skipped = Some("existing".to_string());
                    timing.total_ms = millis(total_started.elapsed());
                    return timing;
                }
            }
            Err(error) => {
                timing.existing_ms = millis(started.elapsed());
                timing.skipped = Some(format!("existing: {error}"));
                timing.total_ms = millis(total_started.elapsed());
                return timing;
            }
        }
    }

    let started = Instant::now();
    let mut metadata = match metadata_source.metadata_for_video(&video.video_id).await {
        Ok(metadata) => {
            timing.metadata_ms = millis(started.elapsed());
            metadata
        }
        Err(error) => {
            timing.metadata_ms = millis(started.elapsed());
            timing.skipped = Some(format!("metadata: {error}"));
            timing.total_ms = millis(total_started.elapsed());
            return timing;
        }
    };
    metadata.youtuber.id = youtuber_id.to_string();

    let content_kind = content_kind_for_duration(metadata.duration_ms);
    if !content_kind_filter.includes(content_kind) {
        timing.skipped = Some(format!("kind: {content_kind:?}"));
        timing.total_ms = millis(total_started.elapsed());
        return timing;
    }

    let started = Instant::now();
    let (transcript, _) = match fetch_transcript_with_optional_fallback(
        &video.video_id,
        caption_source,
        transcribe_source,
    )
    .await
    {
        Ok(result) => {
            timing.transcript_ms = millis(started.elapsed());
            result
        }
        Err(error) => {
            timing.transcript_ms = millis(started.elapsed());
            timing.skipped = Some(format!("transcript: {error}"));
            timing.total_ms = millis(total_started.elapsed());
            return timing;
        }
    };

    let started = Instant::now();
    let chunks = match chunker.chunks(&metadata, &transcript) {
        Ok(chunks) => {
            timing.chunk_ms = millis(started.elapsed());
            chunks
        }
        Err(error) => {
            timing.chunk_ms = millis(started.elapsed());
            timing.skipped = Some(format!("chunking: {error}"));
            timing.total_ms = millis(total_started.elapsed());
            return timing;
        }
    };
    timing.chunks = chunks.len();

    if import {
        let started = Instant::now();
        if let Err(error) = search.import_chunks_batched(&chunks).await {
            timing.import_ms = millis(started.elapsed());
            timing.skipped = Some(format!("import: {error}"));
            timing.total_ms = millis(total_started.elapsed());
            return timing;
        }
        timing.import_ms = millis(started.elapsed());
    }

    timing.total_ms = millis(total_started.elapsed());
    timing
}

fn millis(duration: Duration) -> u128 {
    duration.as_millis()
}

#[allow(clippy::too_many_arguments)]
async fn index_discovered_video(
    index: usize,
    total: usize,
    video: DiscoveredVideo,
    youtuber_id: String,
    metadata_source: Arc<FallbackMetadataSource>,
    caption_source: YtDlpTranscriptSource,
    transcribe_source: Option<VoxtypeTranscriptSource>,
    chunker: Chunker,
    search: SearchAdminClient,
    content_kind_filter: ContentKindFilter,
    skip_existing: bool,
) -> Result<IndexVideoOutcome, CliError> {
    tracing::info!(
        video_id = %video.video_id,
        title = %video.title,
        index,
        total,
        "indexing video"
    );
    println!(
        "Indexing video {index}/{total}: {} {}",
        video.video_id, video.title
    );

    if skip_existing {
        let existing_chunks = search.count_chunks_by_video_id(&video.video_id).await?;
        if existing_chunks > 0 {
            return Ok(IndexVideoOutcome::Existing {
                video_id: video.video_id,
                title: video.title,
                chunks: existing_chunks,
            });
        }
    }

    let mut metadata = match metadata_source.metadata_for_video(&video.video_id).await {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::warn!(video_id = %video.video_id, %error, "skipped video metadata");
            return Ok(IndexVideoOutcome::Skipped {
                video_id: video.video_id,
                title: video.title,
                reason: format!("metadata: {error}"),
            });
        }
    };
    metadata.youtuber.id = youtuber_id;

    let content_kind = content_kind_for_duration(metadata.duration_ms);
    if !content_kind_filter.includes(content_kind) {
        tracing::info!(video_id = %video.video_id, ?content_kind, "skipped by content kind");
        return Ok(IndexVideoOutcome::KindFiltered {
            video_id: video.video_id,
            title: video.title,
            content_kind,
        });
    }

    let (transcript, audio_transcribed) = match fetch_transcript_with_optional_fallback(
        &video.video_id,
        &caption_source,
        transcribe_source.as_ref(),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(video_id = %video.video_id, %error, "skipped transcript");
            return Ok(IndexVideoOutcome::Skipped {
                video_id: video.video_id,
                title: video.title,
                reason: format!("transcript: {error}"),
            });
        }
    };

    let chunks = match chunker.chunks(&metadata, &transcript) {
        Ok(chunks) => chunks,
        Err(error) => {
            tracing::warn!(video_id = %video.video_id, %error, "skipped chunking");
            return Ok(IndexVideoOutcome::Skipped {
                video_id: video.video_id,
                title: video.title,
                reason: format!("chunking: {error}"),
            });
        }
    };

    let chunk_count = chunks.len();
    if let Err(error) = search.import_chunks_batched(&chunks).await {
        tracing::warn!(video_id = %video.video_id, %error, "skipped import");
        return Ok(IndexVideoOutcome::Skipped {
            video_id: video.video_id,
            title: video.title,
            reason: format!("import: {error}"),
        });
    }
    tracing::info!(video_id = %video.video_id, chunk_count, "imported transcript chunks");

    Ok(IndexVideoOutcome::Indexed {
        video_id: video.video_id,
        title: video.title,
        chunks: chunk_count,
        audio_transcribed,
    })
}

fn apply_index_outcome(
    stats: &mut IndexStats,
    outcome: IndexVideoOutcome,
    completed: usize,
    total: usize,
) {
    match outcome {
        IndexVideoOutcome::Indexed {
            video_id,
            title,
            chunks,
            audio_transcribed,
        } => {
            stats.indexed_videos = stats.indexed_videos.saturating_add(1);
            stats.indexed_chunks = stats.indexed_chunks.saturating_add(saturating_u32(chunks));
            if audio_transcribed {
                stats.audio_transcribed_videos = stats.audio_transcribed_videos.saturating_add(1);
            }
            println!("Done {completed}/{total}: {video_id} {title} — imported {chunks} chunks");
        }
        IndexVideoOutcome::Existing {
            video_id,
            title,
            chunks,
        } => {
            stats.existing_videos = stats.existing_videos.saturating_add(1);
            stats.existing_chunks = stats.existing_chunks.saturating_add(saturating_u32(chunks));
            println!(
                "Done {completed}/{total}: {video_id} {title} — already indexed ({chunks} chunks)"
            );
        }
        IndexVideoOutcome::KindFiltered {
            video_id,
            title,
            content_kind,
        } => {
            stats.kind_filtered_videos = stats.kind_filtered_videos.saturating_add(1);
            println!(
                "Done {completed}/{total}: {video_id} {title} — skipped by kind ({content_kind:?})"
            );
        }
        IndexVideoOutcome::Skipped {
            video_id,
            title,
            reason,
        } => {
            stats.skipped_videos = stats.skipped_videos.saturating_add(1);
            println!("Done {completed}/{total}: {video_id} {title} — skipped {reason}");
        }
    }
}

async fn index_video_command(command: IndexVideoCommand) -> Result<(), CliError> {
    let video_id = parse_video_id(&command.video)?;
    let youtube = command.youtube.optional_client()?;
    let collection = command.collection.collection;
    let search = command.search.client(collection.clone())?;

    match search.create_collection_if_missing().await? {
        CollectionOutcome::Created => {
            println!("Created search backend collection: {collection}")
        }
        CollectionOutcome::AlreadyExists => {
            println!("search backend collection already exists: {collection}")
        }
    }

    if command.no_wipe {
        println!("Keeping existing transcript chunks for video {video_id}");
    } else {
        println!("Deleting existing transcript chunks for video {video_id} before reindex");
        let delete_response = search.delete_chunks_by_video_id(&video_id).await?;
        println!(
            "Deleted existing chunks response: {}",
            serde_json::to_string(&delete_response).unwrap_or_else(|_| delete_response.to_string())
        );
    }

    let caption_source = transcript_source(
        command.no_transcript_cache,
        command.transcript_cache_dir.clone(),
    );
    let metadata_source = metadata_source(youtube.as_ref(), &caption_source);
    let transcribe_source = command.transcribe_missing.then(|| {
        VoxtypeTranscriptSource::with_config(
            TranscriptionConfig::default()
                .with_cache_dir(command.transcription_cache_dir.clone())
                .with_chunk_seconds(command.transcription_chunk_seconds)
                .with_voxtype_engine(command.voxtype_engine.clone()),
        )
    });
    let chunker = Chunker::new(command.chunk_seconds);

    println!("Indexing video: {video_id}");
    let metadata = metadata_source.metadata_for_video(&video_id).await?;
    println!("  title: {}", metadata.title);
    println!(
        "  creator: {} ({})",
        metadata.youtuber.name, metadata.youtuber.handle
    );

    let (transcript, audio_transcribed) = fetch_transcript_with_optional_fallback(
        &video_id,
        &caption_source,
        transcribe_source.as_ref(),
    )
    .await?;
    let chunks = chunker.chunks(&metadata, &transcript)?;
    let chunk_count = chunks.len();
    search.import_chunks_batched(&chunks).await?;

    println!("  imported {chunk_count} transcript chunks");
    println!(
        "Index summary: video_id={}, chunks={}, audio_transcribed={}",
        video_id, chunk_count, audio_transcribed
    );

    Ok(())
}

async fn search_command(command: SearchCommand) -> Result<(), CliError> {
    let search_client = command.search.client(command.collection.collection)?;
    let mut search = TextSearchQuery::new(command.query).per_page(command.per_page);

    if let Some(youtuber) = command.youtuber {
        search = search.youtuber_handle(normalize_handle(&youtuber)?);
    }

    let response = search_client.search(&search).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&response).unwrap_or(response.to_string())
    );
    Ok(())
}

async fn validate_youtuber(
    youtube: Option<&YouTubeClient>,
    ytdlp: &YtDlpTranscriptSource,
    raw_handle: &str,
    max_videos: u8,
) -> Result<ValidatedYoutuber, CliError> {
    let handle = normalize_handle(raw_handle)?;
    match validate_youtuber_with_ytdlp(ytdlp, &handle, max_videos).await {
        Ok(mut youtuber) => {
            println!("  validation source: yt-dlp channel feed");
            if let Some(youtube) = youtube {
                match enrich_youtuber_channel_metadata(youtube, &handle, 0).await {
                    Ok(channel) => {
                        // Keep yt-dlp as the discovery source, but persist the
                        // canonical channel identity and image from YouTube's API.
                        youtuber.handle = channel.handle;
                        youtuber.channel_id = channel.channel_id;
                        youtuber.title = channel.title;
                        youtuber.uploads_playlist = channel.uploads_playlist;
                        youtuber.avatar_url = channel.avatar_url;
                        println!("  channel profile source: YouTube Data API");
                    }
                    Err(error) => {
                        tracing::warn!(handle = %handle, %error, "YouTube API profile enrichment failed; retaining yt-dlp metadata");
                        println!(
                            "  YouTube API profile enrichment failed ({error}); retaining yt-dlp metadata"
                        );
                    }
                }
            }
            return Ok(youtuber);
        }
        Err(error) => {
            tracing::warn!(handle = %handle, %error, "yt-dlp validation failed; falling back to YouTube API");
            println!("  yt-dlp validation failed ({error}); falling back to YouTube API");
        }
    }

    let youtube = youtube.ok_or_else(|| {
        CliError::Message(format!(
            "Could not resolve YouTube handle {handle} with yt-dlp and no YOUTUBE_API_KEY is configured"
        ))
    })?;
    validate_youtuber_with_youtube_api(youtube, &handle, max_videos).await
}

async fn validate_youtuber_with_youtube_api(
    youtube: &YouTubeClient,
    handle: &str,
    max_videos: u8,
) -> Result<ValidatedYoutuber, CliError> {
    let mut youtuber = enrich_youtuber_channel_metadata(youtube, handle, max_videos).await?;
    if youtuber.recent_uploads.is_empty() {
        if let Some(uploads_playlist) = youtuber.uploads_playlist.as_deref() {
            match fetch_recent_uploads(youtube, uploads_playlist, max_videos).await {
                Ok(recent_uploads) => {
                    youtuber.recent_uploads = recent_uploads;
                    youtuber.uploads_warning = None;
                }
                Err(error) => {
                    youtuber.uploads_warning = Some(format!(
                        "could not list recent uploads from playlist {uploads_playlist}: {error}"
                    ));
                }
            }
        } else {
            youtuber.uploads_warning =
                Some("channel did not expose an uploads playlist".to_string());
        }
    }
    Ok(youtuber)
}

async fn enrich_youtuber_channel_metadata(
    youtube: &YouTubeClient,
    handle: &str,
    max_videos: u8,
) -> Result<ValidatedYoutuber, CliError> {
    let channel_page = youtube
        .list(ListResource::Channels)
        .query(
            Query::new()
                .part(["id", "snippet", "contentDetails"])
                .set("forHandle", handle)
                .fields(
                    "items(id,snippet(title,customUrl,thumbnails),contentDetails/relatedPlaylists/uploads)",
                ),
        )
        .page()
        .await?;

    let channel = first_item(&channel_page).ok_or_else(|| {
        CliError::Message(format!("No YouTube channel found for handle {handle}"))
    })?;
    let channel_id = required_string(channel, "/id", "channel id")?;
    let title = required_string(channel, "/snippet/title", "channel title")?;
    let resolved_handle =
        optional_string(channel, "/snippet/customUrl").unwrap_or_else(|| handle.to_string());
    let uploads_playlist = optional_string(channel, "/contentDetails/relatedPlaylists/uploads");
    let avatar_url = best_thumbnail(channel.pointer("/snippet/thumbnails"));
    let recent_uploads = match uploads_playlist.as_deref() {
        Some(uploads_playlist) if max_videos > 0 => {
            fetch_recent_uploads(youtube, uploads_playlist, max_videos)
                .await
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };

    Ok(ValidatedYoutuber {
        handle: resolved_handle,
        channel_id,
        title,
        uploads_playlist,
        avatar_url,
        recent_uploads,
        uploads_warning: None,
    })
}

async fn discover_videos_for_youtuber(
    youtube: Option<&YouTubeClient>,
    ytdlp: &YtDlpTranscriptSource,
    youtuber: &ValidatedYoutuber,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    match discover_videos_for_youtuber_with_ytdlp(ytdlp, &youtuber.handle).await {
        Ok(videos) => {
            println!("  discovery source: yt-dlp channel feed");
            return Ok(videos);
        }
        Err(error) => {
            tracing::warn!(handle = %youtuber.handle, %error, "yt-dlp discovery failed; falling back to YouTube API");
            println!("  yt-dlp discovery failed ({error}); falling back to YouTube API");
        }
    }

    let youtube = youtube.ok_or_else(|| {
        CliError::Message(format!(
            "Could not discover videos for {} with yt-dlp and no YOUTUBE_API_KEY is configured",
            youtuber.handle
        ))
    })?;
    if let Some(uploads_playlist) = youtuber.uploads_playlist.as_deref() {
        match discover_upload_playlist_videos(youtube, uploads_playlist).await {
            Ok(videos) => {
                println!("  discovery source: uploads playlist ({uploads_playlist})");
                return Ok(videos);
            }
            Err(error) => {
                tracing::warn!(
                    playlist = uploads_playlist,
                    %error,
                    "uploads playlist discovery failed; falling back to search.list"
                );
                println!(
                    "  uploads playlist discovery failed ({error}); falling back to search.list"
                );
            }
        }
    } else {
        println!("  uploads playlist unavailable; falling back to search.list");
    }

    discover_public_videos(youtube, &youtuber.channel_id).await
}

async fn validate_youtuber_with_ytdlp(
    ytdlp: &YtDlpTranscriptSource,
    handle: &str,
    max_videos: u8,
) -> Result<ValidatedYoutuber, CliError> {
    let feed = ytdlp
        .channel_feed(handle, Some(usize::from(max_videos.max(1))))
        .await?;
    if feed.videos.is_empty() {
        return Err(CliError::Message(format!(
            "No YouTube channel found for handle {handle}"
        )));
    }
    let uploads_playlist = uploads_playlist_from_channel_id(&feed.channel_id);

    Ok(ValidatedYoutuber {
        handle: feed.handle,
        channel_id: feed.channel_id,
        title: feed.title,
        uploads_playlist,
        avatar_url: feed.avatar_url,
        recent_uploads: feed
            .videos
            .into_iter()
            .take(usize::from(max_videos))
            .map(|video| UploadPreview {
                video_id: video.video_id,
                title: video.title,
            })
            .collect(),
        uploads_warning: None,
    })
}

fn uploads_playlist_from_channel_id(channel_id: &str) -> Option<String> {
    let suffix = channel_id.strip_prefix("UC")?;
    if suffix.is_empty() {
        return None;
    }
    Some(format!("UU{suffix}"))
}

async fn discover_videos_for_youtuber_with_ytdlp(
    ytdlp: &YtDlpTranscriptSource,
    handle: &str,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let mut videos = ytdlp
        .channel_videos(handle, None)
        .await?
        .into_iter()
        .map(discovered_video_from_ytdlp_video)
        .collect::<Vec<_>>();

    match ytdlp.channel_streams(handle, None).await {
        Ok(streams) => {
            println!(
                "  discovery source: yt-dlp streams tab ({} entries)",
                streams.len()
            );
            videos.extend(streams.into_iter().map(discovered_video_from_ytdlp_video));
        }
        Err(error) => {
            tracing::warn!(handle = %handle, %error, "yt-dlp stream-tab discovery failed; continuing with uploads");
            println!("  stream-tab discovery skipped ({error})");
        }
    }

    Ok(dedupe_discovered_videos(videos))
}

async fn discover_upload_playlist_videos(
    youtube: &YouTubeClient,
    uploads_playlist: &str,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let mut videos = Vec::new();
    let mut page_token = None;

    loop {
        let mut query = Query::new()
            .part(["snippet", "contentDetails"])
            .set("playlistId", uploads_playlist)
            .max_results(50)
            .fields("nextPageToken,items(snippet(title),contentDetails/videoId)");

        if let Some(token) = page_token {
            query = query.page_token(token);
        }

        let page = youtube
            .list(ListResource::PlaylistItems)
            .query(query)
            .page()
            .await?;
        for item in &page.items {
            if let Some(video) = discovered_video_from_playlist_item(item) {
                videos.push(video);
            }
        }

        page_token = page.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    Ok(dedupe_discovered_videos(videos))
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

    Ok(dedupe_discovered_videos(videos))
}

fn discovered_video_from_playlist_item(item: &Value) -> Option<DiscoveredVideo> {
    Some(DiscoveredVideo {
        video_id: optional_string(item, "/contentDetails/videoId")?,
        title: optional_string(item, "/snippet/title")?,
    })
}

fn discovered_video_from_search_item(item: &Value) -> Option<DiscoveredVideo> {
    Some(DiscoveredVideo {
        video_id: optional_string(item, "/id/videoId")?,
        title: optional_string(item, "/snippet/title")?,
    })
}

fn discovered_video_from_ytdlp_video(video: YtDlpVideo) -> DiscoveredVideo {
    DiscoveredVideo {
        video_id: video.video_id,
        title: video.title,
    }
}

fn dedupe_discovered_videos(videos: Vec<DiscoveredVideo>) -> Vec<DiscoveredVideo> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::with_capacity(videos.len());

    for video in videos {
        if seen.insert(video.video_id.clone()) {
            deduped.push(video);
        }
    }

    deduped
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

async fn fetch_transcript_with_optional_fallback(
    video_id: &str,
    caption_source: &YtDlpTranscriptSource,
    transcribe_source: Option<&VoxtypeTranscriptSource>,
) -> Result<(Vec<TranscriptSegment>, bool), CliError> {
    match caption_source.transcript_for_video(video_id).await {
        Ok(transcript) => Ok((transcript, false)),
        Err(caption_error) => {
            let Some(transcribe_source) = transcribe_source else {
                return Err(caption_error.into());
            };

            tracing::info!(video_id, error = %caption_error, "captions unavailable, trying local audio transcription");
            println!("  captions unavailable, trying local audio transcription: {caption_error}");
            let transcript = transcribe_source.transcript_for_video(video_id).await?;
            Ok((transcript, true))
        }
    }
}

fn parse_video_id(input: &str) -> Result<String, CliError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(CliError::Message(
            "video ID or URL cannot be empty".to_string(),
        ));
    }

    if looks_like_video_id(trimmed) {
        return Ok(trimmed.to_string());
    }

    if let Some(video_id) = query_param(trimmed, "v").filter(|value| looks_like_video_id(value)) {
        return Ok(video_id.to_string());
    }

    for marker in ["youtu.be/", "/shorts/", "/embed/"] {
        if let Some(after_marker) = trimmed.split_once(marker).map(|(_, value)| value) {
            let candidate = after_marker
                .split(['?', '&', '/', '#'])
                .next()
                .unwrap_or_default();
            if looks_like_video_id(candidate) {
                return Ok(candidate.to_string());
            }
        }
    }

    Err(CliError::Message(format!(
        "Could not parse a YouTube video ID from {input}"
    )))
}

fn query_param<'a>(input: &'a str, key: &str) -> Option<&'a str> {
    let query = input
        .split_once('?')?
        .1
        .split('#')
        .next()
        .unwrap_or_default();
    query.split('&').find_map(|pair| {
        let (pair_key, value) = pair.split_once('=')?;
        (pair_key == key).then_some(value)
    })
}

fn looks_like_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
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

fn saturating_u32(value: impl TryInto<u32>) -> u32 {
    value.try_into().unwrap_or(u32::MAX)
}

fn print_index_summary(stats: &IndexStats) {
    println!(
        "Index summary: discovered={}, attempted={}, indexed={}, existing={}, skipped={}, kind_filtered={}, audio_transcribed={}, chunks={}, existing_chunks={}",
        stats.discovered_videos,
        stats.attempted_videos,
        stats.indexed_videos,
        stats.existing_videos,
        stats.skipped_videos,
        stats.kind_filtered_videos,
        stats.audio_transcribed_videos,
        stats.indexed_chunks,
        stats.existing_chunks
    );
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

fn best_thumbnail(thumbnails: Option<&Value>) -> Option<String> {
    let thumbnails = thumbnails?.as_object()?;
    ["high", "medium", "default"]
        .iter()
        .find_map(|quality| thumbnails.get(*quality)?.get("url")?.as_str())
        .map(ToOwned::to_owned)
}

fn print_validated_youtuber(youtuber: &ValidatedYoutuber) {
    println!("Validated YouTuber: {}", youtuber.handle);
    println!("  title: {}", youtuber.title);
    println!("  channel_id: {}", youtuber.channel_id);
    if youtuber.avatar_url.is_some() {
        println!("  avatar: available");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str) -> DiscoveredVideo {
        DiscoveredVideo {
            video_id: id.to_string(),
            title: id.to_string(),
        }
    }

    fn run_existing_tail_plan(
        checks: &[(&str, u64)],
        stop_after_existing: usize,
    ) -> (Vec<String>, IndexStats, usize, bool) {
        let mut stats = IndexStats::default();
        let mut selected = Vec::new();
        let mut consecutive_existing = 0usize;
        let mut stopped = false;

        for (id, existing_chunks) in checks {
            stopped = select_missing_video_until_existing_tail(
                video(id),
                *existing_chunks,
                stop_after_existing,
                &mut stats,
                &mut selected,
                &mut consecutive_existing,
            );
            if stopped {
                break;
            }
        }

        (
            selected.into_iter().map(|video| video.video_id).collect(),
            stats,
            consecutive_existing,
            stopped,
        )
    }

    #[test]
    fn existing_tail_scan_stops_after_consecutive_existing_videos() {
        let (selected, stats, consecutive_existing, stopped) =
            run_existing_tail_plan(&[("new-1", 0), ("old-1", 3), ("old-2", 7), ("new-2", 0)], 2);

        assert_eq!(selected, vec!["new-1"]);
        assert_eq!(stats.attempted_videos, 3);
        assert_eq!(stats.existing_videos, 2);
        assert_eq!(stats.existing_chunks, 10);
        assert_eq!(consecutive_existing, 2);
        assert!(stopped);
    }

    #[test]
    fn existing_tail_scan_resets_after_missing_video() {
        let (selected, stats, consecutive_existing, stopped) =
            run_existing_tail_plan(&[("old-1", 3), ("new-1", 0), ("old-2", 4), ("old-3", 5)], 2);

        assert_eq!(selected, vec!["new-1"]);
        assert_eq!(stats.attempted_videos, 4);
        assert_eq!(stats.existing_videos, 3);
        assert_eq!(stats.existing_chunks, 12);
        assert_eq!(consecutive_existing, 2);
        assert!(stopped);
    }

    #[test]
    fn zero_stop_after_existing_is_normalized_to_disabled() {
        assert_eq!(normalized_stop_after_existing(Some(0)), None);
        assert_eq!(normalized_stop_after_existing(Some(50)), Some(50));
        assert_eq!(normalized_stop_after_existing(None), None);
    }

    #[test]
    fn worker_existing_checks_only_run_for_small_uncapped_update_batches() {
        assert!(should_use_worker_existing_checks(true, None, 1));
        assert!(should_use_worker_existing_checks(true, None, 100));
        assert!(!should_use_worker_existing_checks(true, None, 0));
        assert!(!should_use_worker_existing_checks(true, None, 101));
        assert!(!should_use_worker_existing_checks(false, None, 10));
        assert!(!should_use_worker_existing_checks(true, Some(50), 10));
    }

    #[test]
    fn canonical_youtuber_id_does_not_include_handle_prefix() {
        assert_eq!(
            youtuber_id_from_handle("@t3dotgg", "UCbRP3c757lWg9M-U7TyEkXA"),
            "t3dotgg"
        );
    }

    #[test]
    fn backend_counts_replace_stale_profile_counts() {
        let mut youtuber = YoutuberDocument {
            id: "t3dotgg".to_string(),
            handle: "@t3dotgg".to_string(),
            name: "Theo".to_string(),
            channel_id: "channel".to_string(),
            uploads_playlist: None,
            public_video_count: 500,
            indexed_video_count: 1,
            indexed_chunk_count: 114,
            avatar_url: None,
        };

        apply_backend_chunk_stats(
            &mut youtuber,
            BackendChunkStats {
                indexed_videos: 576,
                indexed_chunks: 165_451,
            },
        );

        assert_eq!(youtuber.indexed_video_count, 576);
        assert_eq!(youtuber.indexed_chunk_count, 165_451);
        assert_eq!(youtuber.public_video_count, 576);
    }

    #[test]
    fn destructive_rewipe_requires_explicit_acknowledgement() {
        assert!(validate_rewipe_safety(IndexMode::Update, false, false).is_ok());
        assert!(validate_rewipe_safety(IndexMode::Rewipe, true, false).is_ok());
        assert!(validate_rewipe_safety(IndexMode::Rewipe, false, true).is_ok());
        assert!(validate_rewipe_safety(IndexMode::Rewipe, false, false).is_err());
    }

    #[test]
    fn derives_uploads_playlist_from_canonical_channel_id() {
        assert_eq!(
            uploads_playlist_from_channel_id("UCbRP3c757lWg9M-U7TyEkXA").as_deref(),
            Some("UUbRP3c757lWg9M-U7TyEkXA")
        );
        assert_eq!(uploads_playlist_from_channel_id("not-a-channel"), None);
    }
}
