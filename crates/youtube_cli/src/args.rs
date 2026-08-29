use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use youtube_client::YouTubeClient;
use youtube_index_store::{RawIndexAdminClient, RawIndexAdminConfig};
use youtube_search_client::{DEFAULT_COLLECTION, SearchClient, SearchClientConfig};

use super::{CliError, ContentKindFilter, RAW_PUBLICATION_TIMEOUT_ENV, raw_publication_timeout};

const DEFAULT_TRANSCRIPT_FAILURE_RETRY_SECONDS: u64 = 21_600;

#[derive(Debug, Parser)]
#[command(name = "ytsearch")]
#[command(about = "YouTube transcript search indexing utilities")]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Debug, Subcommand)]
pub(super) enum Command {
    #[command(about = "Verify that the mutable Raw search backend is ready")]
    InitBackend(InitBackendCommand),
    #[command(about = "Validate that a YouTube handle resolves and show recent uploads")]
    Validate(ValidateCommand),
    #[command(about = "Create/update the YouTuber document and report discoverable public videos")]
    IndexYoutuber(IndexYoutuberCommand),
    #[command(about = "Index YouTubers from a text file")]
    IndexYoutubers(IndexYoutubersCommand),
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
    #[command(about = "Index one video by ID or URL")]
    IndexVideo(IndexVideoCommand),
    #[command(about = "Search transcript chunks using the selected search backend")]
    Search(SearchCommand),
    #[command(about = "Build and activate pending Raw mutations on the external builder")]
    Publish(PublishCommand),
    #[command(about = "Summarize classified video failures by creator as JSON")]
    FailureReport(FailureReportCommand),
}

#[derive(Clone, Debug, Args)]
pub(super) struct FailureReportCommand {
    #[arg(
        value_name = "TRANSCRIPT_CACHE_DIRECTORY",
        default_value = "data/transcript-cache"
    )]
    pub(super) transcript_cache_directory: PathBuf,
    #[arg(
        long,
        default_value_t = DEFAULT_TRANSCRIPT_FAILURE_RETRY_SECONDS,
        help = "Retry policy used by the indexing run whose failures are being reported"
    )]
    pub(super) transcript_failure_retry_seconds: u64,
}

#[derive(Clone, Debug, Args)]
pub(super) struct PublishCommand {
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
pub(super) struct InitBackendCommand {
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
pub(super) struct ValidateCommand {
    #[command(flatten)]
    pub(super) youtuber: YoutuberArg,
    #[arg(long, default_value_t = 5, help = "Number of recent uploads to list")]
    pub(super) max_videos: u8,
    #[command(flatten)]
    pub(super) youtube: YouTubeAuthArgs,
}

#[derive(Clone, Debug, Args)]
pub(super) struct RefreshYoutuberCommand {
    #[command(flatten)]
    pub(super) youtuber: YoutuberArg,
    #[command(flatten)]
    pub(super) youtube: YouTubeAuthArgs,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
}

#[derive(Clone, Debug, Args)]
pub(super) struct RefreshYoutubersCommand {
    #[arg(value_name = "LIST_FILE")]
    pub(super) list_file: PathBuf,
    #[command(flatten)]
    pub(super) youtube: YouTubeAuthArgs,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
}

#[derive(Clone, Debug, Args)]
pub(super) struct RefreshYoutuberCountsCommand {
    #[command(flatten)]
    pub(super) youtuber: YoutuberArg,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
pub(super) struct RefreshYoutuberCountsListCommand {
    #[arg(value_name = "LIST_FILE")]
    pub(super) list_file: PathBuf,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
}

#[derive(Clone, Debug, Args)]
pub(super) struct RemoveYoutuberCommand {
    #[command(flatten)]
    pub(super) youtuber: YoutuberArg,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
    #[arg(
        long,
        help = "Acknowledge permanent deletion of the channel profile and all transcript chunks"
    )]
    pub(super) allow_destructive_remove: bool,
}

#[derive(Clone, Debug, Args)]
pub(super) struct IndexYoutuberCommand {
    #[command(flatten)]
    pub(super) youtuber: YoutuberArg,
    #[command(flatten)]
    pub(super) youtube: YouTubeAuthArgs,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
    #[arg(long, help = "Optional cap for a test indexing run")]
    pub(super) max_videos: Option<usize>,
    #[arg(
        long,
        help = "After existing-video filtering, index at most this many missing videos"
    )]
    pub(super) max_new_videos: Option<usize>,
    #[arg(
        long,
        help = "In update mode, stop scanning newest-first after this many consecutive already-indexed videos. Use 0 to disable"
    )]
    pub(super) stop_after_existing: Option<usize>,
    #[arg(long, value_enum, default_value_t = IndexMode::Update)]
    pub(super) mode: IndexMode,
    #[arg(
        long,
        help = "Acknowledge that rewipe deletes existing chunks before replacement data is imported"
    )]
    pub(super) allow_destructive_rewipe: bool,
    #[arg(long, default_value_t = 4, help = "Concurrent videos to index")]
    pub(super) jobs: usize,
    #[arg(
        long,
        default_value_t = 300,
        help = "Maximum seconds to spend on one video"
    )]
    pub(super) video_timeout_seconds: u64,
    #[arg(
        long,
        help = "Skip videos that already have chunks in the search backend"
    )]
    pub(super) skip_existing: bool,
    #[arg(
        long,
        help = "Keep existing chunks for this YouTuber instead of wiping before import"
    )]
    pub(super) no_wipe: bool,
    #[arg(long, value_enum, default_value_t = ContentKindFilter::All)]
    pub(super) content_kind: ContentKindFilter,
    #[command(flatten)]
    pub(super) transcript: TranscriptOptions,
    #[arg(
        long,
        default_value_t = DEFAULT_TRANSCRIPT_FAILURE_RETRY_SECONDS,
        help = "Seconds before retrying a video whose transcript fetch previously failed"
    )]
    pub(super) transcript_failure_retry_seconds: u64,
    #[arg(
        long,
        help = "Write durable mutations without building a generation; a later `ytsearch publish` makes them searchable"
    )]
    pub(super) defer_publish: bool,
}

#[derive(Clone, Debug, Args)]
pub(super) struct IndexYoutubersCommand {
    #[arg(value_name = "LIST_FILE")]
    pub(super) list_file: PathBuf,
    #[command(flatten)]
    pub(super) youtube: YouTubeAuthArgs,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
    #[arg(long, value_enum, default_value_t = IndexMode::Update)]
    pub(super) mode: IndexMode,
    #[arg(
        long,
        help = "Acknowledge that rewipe deletes existing chunks before replacement data is imported"
    )]
    pub(super) allow_destructive_rewipe: bool,
    #[arg(long, help = "Optional cap per creator for a test indexing run")]
    pub(super) max_videos: Option<usize>,
    #[arg(
        long,
        help = "In update mode, stop scanning newest-first after this many consecutive already-indexed videos. Use 0 to disable"
    )]
    pub(super) stop_after_existing: Option<usize>,
    #[arg(
        long,
        default_value_t = 4,
        help = "Concurrent videos to index per creator"
    )]
    pub(super) jobs: usize,
    #[arg(
        long,
        default_value_t = 1,
        help = "Concurrent creators; automatically bounded so total video workers never exceed 16"
    )]
    pub(super) channel_jobs: usize,
    #[arg(
        long,
        default_value_t = 300,
        help = "Maximum seconds to spend on one video"
    )]
    pub(super) video_timeout_seconds: u64,
    #[arg(long, value_enum, default_value_t = ContentKindFilter::All)]
    pub(super) content_kind: ContentKindFilter,
    #[command(flatten)]
    pub(super) transcript: TranscriptOptions,
    #[arg(
        long,
        default_value_t = DEFAULT_TRANSCRIPT_FAILURE_RETRY_SECONDS,
        help = "Seconds before retrying a video whose transcript fetch previously failed"
    )]
    pub(super) transcript_failure_retry_seconds: u64,
    #[arg(
        long,
        help = "Write durable mutations without building a generation; a later `ytsearch publish` makes them searchable"
    )]
    pub(super) defer_publish: bool,
}

#[derive(Clone, Debug, Args)]
pub(super) struct TranscriptOptions {
    #[arg(
        long,
        default_value_t = 10,
        help = "Transcript chunk window in seconds"
    )]
    pub(super) chunk_seconds: u32,
    #[arg(
        long,
        help = "When captions are missing, extract audio with yt-dlp and transcribe locally with voxtype"
    )]
    pub(super) transcribe_missing: bool,
    #[arg(long, default_value = "data/transcription-cache")]
    pub(super) transcription_cache_dir: PathBuf,
    #[arg(long, default_value = "data/transcript-cache")]
    pub(super) transcript_cache_dir: PathBuf,
    #[arg(long, help = "Disable the yt-dlp transcript cache")]
    pub(super) no_transcript_cache: bool,
    #[arg(long, default_value_t = 30)]
    pub(super) transcription_chunk_seconds: u32,
    #[arg(
        long,
        help = "Optional voxtype engine override, e.g. whisper or parakeet"
    )]
    pub(super) voxtype_engine: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub(super) struct IndexVideoCommand {
    #[arg(value_name = "VIDEO_ID_OR_URL")]
    pub(super) video: String,
    #[command(flatten)]
    pub(super) youtube: YouTubeAuthArgs,
    #[command(flatten)]
    pub(super) search: SearchAdminArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
    #[arg(
        long,
        help = "Keep existing chunks for this video instead of wiping before import"
    )]
    pub(super) no_wipe: bool,
    #[command(flatten)]
    pub(super) transcript: TranscriptOptions,
}

#[derive(Clone, Debug, Args)]
pub(super) struct SearchCommand {
    #[arg(value_name = "QUERY")]
    pub(super) query: String,
    #[arg(long, help = "Optional YouTube handle filter, e.g. @t3dotgg")]
    pub(super) youtuber: Option<String>,
    #[command(flatten)]
    pub(super) search: SearchQueryArgs,
    #[command(flatten)]
    pub(super) collection: TranscriptCollectionArg,
    #[arg(long, default_value_t = 10)]
    pub(super) per_page: u8,
}

#[derive(Clone, Debug, Args)]
pub(super) struct YoutuberArg {
    #[arg(value_name = "HANDLE", help = "YouTube handle, e.g. @t3dotgg")]
    pub(super) handle: String,
}

#[derive(Clone, Debug, Args)]
pub(super) struct YouTubeAuthArgs {
    #[arg(
        long,
        env = "YOUTUBE_API_KEY",
        hide_env_values = true,
        help = "YouTube Data API key"
    )]
    pub(super) youtube_api_key: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub(super) struct SearchAdminArgs {
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_BACKEND",
        default_value = "raw",
        help = "Search backend to use (raw)"
    )]
    pub(super) backend: SearchBackend,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_API_URL",
        hide_env_values = true,
        help = "Raw search backend admin URL. Defaults to RAW_SEARCH_API_URL"
    )]
    pub(super) api_url: Option<String>,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_ADMIN_API_KEY",
        hide_env_values = true,
        help = "Raw search backend admin key. Defaults to RAW_SEARCH_ADMIN_API_KEY"
    )]
    pub(super) admin_api_key: Option<String>,
}

#[derive(Clone, Debug, Args)]
pub(super) struct SearchQueryArgs {
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_BACKEND",
        default_value = "raw",
        help = "Search backend to use (raw)"
    )]
    pub(super) backend: SearchBackend,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_API_URL",
        hide_env_values = true,
        help = "Search backend URL. Defaults to RAW_SEARCH_API_URL"
    )]
    pub(super) api_url: Option<String>,
    #[arg(
        long,
        env = "YTSEARCH_SEARCH_API_KEY",
        hide_env_values = true,
        help = "Search key. Defaults to RAW_SEARCH_API_KEY"
    )]
    pub(super) search_api_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum SearchBackend {
    Raw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum IndexMode {
    Update,
    Rewipe,
}

#[derive(Clone, Debug, Args)]
pub(super) struct TranscriptCollectionArg {
    #[arg(
        long,
        env = "YTSEARCH_TRANSCRIPTS_INDEX",
        hide_env_values = true,
        default_value = DEFAULT_COLLECTION
    )]
    pub(super) collection: String,
}

impl YouTubeAuthArgs {
    pub(super) fn client(self) -> Result<YouTubeClient, CliError> {
        let api_key = self.youtube_api_key.ok_or_else(|| {
            CliError::Message("missing YouTube Data API key (YOUTUBE_API_KEY)".to_string())
        })?;
        Ok(YouTubeClient::with_api_key(api_key)?)
    }

    pub(super) fn optional_client(&self) -> Result<Option<YouTubeClient>, CliError> {
        self.youtube_api_key
            .as_ref()
            .map(|api_key| YouTubeClient::with_api_key(api_key.clone()).map_err(CliError::from))
            .transpose()
    }
}

impl SearchAdminArgs {
    pub(super) fn client(
        &self,
        _collection: impl Into<String>,
    ) -> Result<RawIndexAdminClient, CliError> {
        let api_url = self
            .api_url
            .clone()
            .or_else(|| std::env::var("RAW_SEARCH_API_URL").ok())
            .ok_or_else(|| CliError::Message("missing Raw admin URL".to_string()))?;
        let admin_api_key = self
            .admin_api_key
            .clone()
            .or_else(|| std::env::var("RAW_SEARCH_ADMIN_API_KEY").ok())
            .ok_or_else(|| CliError::Message("missing Raw admin key".to_string()))?;
        let publication_timeout =
            raw_publication_timeout(std::env::var(RAW_PUBLICATION_TIMEOUT_ENV).ok().as_deref())?;
        Ok(RawIndexAdminClient::new(
            RawIndexAdminConfig::new(api_url, admin_api_key)
                .with_publication_timeout(publication_timeout),
        )?)
    }
}

impl SearchQueryArgs {
    pub(super) fn client(&self, collection: impl Into<String>) -> Result<SearchClient, CliError> {
        let api_url = self
            .api_url
            .clone()
            .or_else(|| std::env::var("RAW_SEARCH_API_URL").ok())
            .ok_or_else(|| CliError::Message("missing Raw search URL".to_string()))?;
        let search_api_key = self
            .search_api_key
            .clone()
            .or_else(|| std::env::var("RAW_SEARCH_API_KEY").ok())
            .ok_or_else(|| CliError::Message("missing Raw search key".to_string()))?;
        Ok(SearchClient::new(
            SearchClientConfig::new(api_url, search_api_key).with_collection(collection),
        )?)
    }
}
