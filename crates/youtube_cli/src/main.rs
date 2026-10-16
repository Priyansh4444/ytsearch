use clap::{Parser, ValueEnum};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    num::NonZeroU16,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use thiserror::Error;
use tokio::task::JoinSet;
use youtube_client::{ListResource, Query, YouTubeClient};
use youtube_index_store::{
    BackendReadiness, ChannelDeletionScope, ChannelDocument, ChunkStats, IndexOperation,
    IndexStoreError, RAW_GENERATION_FILES, RAW_PUBLICATION_API_VERSION, RawGenerationManifest,
    RawIndexAdminClient, RawPublicationJob, RawPublicationStatus, VideoReplacement,
};
use youtube_ingest::{
    Chunker, TranscriptSegment, TranscriptSource, VideoMetadataSource,
    stable_youtuber_id as youtuber_id_from_handle,
};
use youtube_raw_search::{GenerationBuilder, MutationStore};
use youtube_search_client::TextSearchQuery;
use youtube_sources::{FallbackMetadataSource, YouTubeMetadataSource};
use youtube_transcribe::{
    AudioTranscriptSource, AudioTranscriptionError, TranscriptionBackend, TranscriptionConfig,
};
use youtube_ytdlp::{YtDlpTranscriptSource, YtDlpVideo};

mod args;
mod atomic_file;
mod failure_ledger;
mod publication_receipt;
mod reconciliation_manifest;

use args::*;
use failure_ledger::{FailureCategory, FailureLedger, FailureStage};
use reconciliation_manifest::{begin_catalog_run, write_complete_discovery};

const DEFAULT_RAW_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const MAX_RAW_PUBLICATION_TIMEOUT_SECONDS: u64 = 2 * 60 * 60;
const RAW_PUBLICATION_TIMEOUT_ENV: &str = "RAW_PUBLICATION_TIMEOUT_SECONDS";
const RAW_PUBLICATION_WORK_DIRECTORY_ENV: &str = "RAW_PUBLICATION_WORK_DIRECTORY";
const RAW_PUBLICATION_MIN_FREE_BYTES_ENV: &str = "RAW_PUBLICATION_MIN_FREE_BYTES";
const RAW_PUBLICATION_ACTIVATION_READY_FILE_ENV: &str = "RAW_PUBLICATION_ACTIVATION_READY_FILE";
const RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE_ENV: &str =
    "RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE";
const RAW_PUBLICATION_ACTIVATION_STARTED_FILE_ENV: &str = "RAW_PUBLICATION_ACTIVATION_STARTED_FILE";
const RAW_PUBLICATION_ACTIVATION_FINISHED_FILE_ENV: &str =
    "RAW_PUBLICATION_ACTIVATION_FINISHED_FILE";
const RAW_PUBLICATION_ACTIVATION_WAIT_SECONDS: u64 = 300;
const DEFAULT_RAW_PUBLICATION_MIN_FREE_BYTES: u64 = 8_000_000_000;

#[derive(Debug, Error)]
enum CliError {
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("youtube api failed: {0}")]
    YouTube(#[from] youtube_client::YouTubeError),
    #[error("search query failed: {0}")]
    Search(#[from] youtube_search_client::SearchError),
    #[error("search index administration failed: {0}")]
    IndexStore(#[from] IndexStoreError),
    #[error("ingest failed: {0}")]
    Ingest(#[from] youtube_ingest::IngestError),
    #[error("audio transcription fallback {source}; original caption failure: {caption}")]
    TranscriptFallback {
        caption: String,
        source: AudioTranscriptionError,
    },
    #[error("upstream indexing paused after {0}; retry after the provider cooldown")]
    UpstreamBlocked(&'static str),
    #[error("{0}")]
    RetryableGaps(String),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug)]
struct ValidatedYoutuber {
    handle: String,
    channel_id: String,
    title: String,
    uploads_playlist: Option<String>,
    avatar_url: Option<String>,
    recent_uploads: Vec<DiscoveredVideo>,
    uploads_warning: Option<String>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationPolicy {
    PublishAfterRun,
    DeferVisibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExistingChannelPolicy {
    Retain,
    Rewipe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExistingVideoPolicy {
    Reindex,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExistingDataPolicy {
    channel: ExistingChannelPolicy,
    videos: ExistingVideoPolicy,
}

impl ExistingDataPolicy {
    fn from_cli(mode: IndexMode, no_wipe: bool, skip_existing: bool) -> Self {
        if mode == IndexMode::Update {
            return Self {
                channel: ExistingChannelPolicy::Retain,
                videos: ExistingVideoPolicy::Skip,
            };
        }
        Self {
            channel: if no_wipe {
                ExistingChannelPolicy::Retain
            } else {
                ExistingChannelPolicy::Rewipe
            },
            videos: if skip_existing {
                ExistingVideoPolicy::Skip
            } else {
                ExistingVideoPolicy::Reindex
            },
        }
    }

    const fn skips_existing_videos(self) -> bool {
        matches!(self.videos, ExistingVideoPolicy::Skip)
    }
}

impl PublicationPolicy {
    const fn from_defer_flag(defer: bool) -> Self {
        if defer {
            Self::DeferVisibility
        } else {
            Self::PublishAfterRun
        }
    }

    const fn publishes_after_run(self) -> bool {
        matches!(self, Self::PublishAfterRun)
    }
}

#[derive(Debug, Default)]
struct IndexStats {
    discovered_videos: usize,
    attempted_videos: usize,
    indexed_videos: u32,
    skipped_videos: u32,
    deferred_transcript_failures: u32,
    deferred_failure_categories: BTreeMap<String, u32>,
    failure_categories: BTreeMap<String, u32>,
    retryable_failures: u32,
    kind_filtered_videos: u32,
    existing_videos: u32,
    existing_chunks: u32,
    audio_transcribed_videos: u32,
    indexed_chunks: u32,
}

#[tokio::main]
async fn main() -> ExitCode {
    let _ = dotenvy::dotenv();
    init_tracing();
    // The CLI never serves a generation, so its publication builds keep their
    // pages cached instead of protecting a reader's mapped index.
    youtube_raw_search::retain_build_page_cache();

    let result = match Cli::parse().command {
        Command::InitBackend(command) => init_backend_command(command).await,
        Command::Validate(command) => validate_command(command).await,
        Command::IndexYoutuber(command) => index_youtuber(command).await,
        Command::IndexYoutubers(command) => index_youtubers_command(command).await,
        Command::RemoveYoutuber(command) => remove_youtuber_command(command).await,
        Command::IndexVideo(command) => index_video_command(command).await,
        Command::Search(command) => search_command(command).await,
        Command::Publish(command) => publish_command(command).await,
        Command::FailureReport(command) => failure_report_command(command),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(cli_error_exit_code(&error))
        }
    }
}

fn cli_error_exit_code(error: &CliError) -> u8 {
    match error {
        CliError::RetryableGaps(_) => EXIT_RETRYABLE_GAPS,
        CliError::UpstreamBlocked(_) => EXIT_UPSTREAM_BLOCKED,
        CliError::IndexStore(IndexStoreError::Unavailable { .. }) => EXIT_BACKEND_UNAVAILABLE,
        _ => 1,
    }
}

fn failure_report_command(command: FailureReportCommand) -> Result<(), CliError> {
    let ledger = FailureLedger::new(
        command.transcript_cache_directory.join(".failures"),
        Duration::from_secs(command.transcript_failure_retry_seconds),
    );
    let report = ledger.report()?;
    let encoded = serde_json::to_string_pretty(&report)
        .map_err(|error| CliError::Message(format!("failed to encode failure report: {error}")))?;
    println!("{encoded}");
    Ok(())
}

async fn init_backend_command(command: InitBackendCommand) -> Result<(), CliError> {
    let search = command.search.client(command.collection.collection)?;
    let readiness = search.readiness().await?;
    require_remote_publication_api(&readiness)?;
    println!(
        "Raw backend ready: generation={}, durable_sequence={}",
        readiness.generation.as_deref().unwrap_or("base"),
        readiness.last_sequence.unwrap_or_default()
    );
    Ok(())
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

fn build_youtuber_document_from_chunks(
    youtuber: &ValidatedYoutuber,
    youtuber_id: &str,
    public_video_count: u32,
    chunk_stats: ChunkStats,
    existing: Option<&ChannelDocument>,
) -> ChannelDocument {
    let indexed_video_count = saturating_u32(chunk_stats.indexed_videos);
    ChannelDocument {
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
    }
}

async fn upsert_youtuber_document_from_chunks(
    search: &RawIndexAdminClient,
    youtuber: &ValidatedYoutuber,
    youtuber_id: &str,
    public_video_count: u32,
) -> Result<ChannelDocument, CliError> {
    let (existing, chunk_stats) = tokio::try_join!(
        search.channel(youtuber_id),
        search.channel_stats(youtuber_id),
    )?;
    let youtuber_doc = build_youtuber_document_from_chunks(
        youtuber,
        youtuber_id,
        public_video_count,
        chunk_stats,
        existing.as_ref(),
    );
    upsert_channel_document(search, &youtuber_doc).await?;
    Ok(youtuber_doc)
}

fn preserved_youtuber_name(
    youtuber: &ValidatedYoutuber,
    existing: Option<&ChannelDocument>,
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

async fn upsert_channel_document(
    search: &RawIndexAdminClient,
    document: &ChannelDocument,
) -> Result<IndexOperation, CliError> {
    let payload = serde_json::to_vec(document).map_err(|error| {
        CliError::Message(format!("failed to encode channel mutation: {error}"))
    })?;
    let idempotency_key = content_operation_key("upsert-channel", &document.id, &payload);
    Ok(search.upsert_channel(document, &idempotency_key).await?)
}

async fn replace_video_document(
    search: &RawIndexAdminClient,
    video_id: &str,
    chunks: &[youtube_ingest::TranscriptChunk],
) -> Result<IndexOperation, CliError> {
    let payload = serde_json::to_vec(chunks)
        .map_err(|error| CliError::Message(format!("failed to encode video mutation: {error}")))?;
    let idempotency_key = content_operation_key("replace-video", video_id, &payload);
    let replacement = VideoReplacement::new(video_id, chunks)?;
    Ok(search.replace_video(replacement, &idempotency_key).await?)
}

fn content_operation_key(kind: &str, target: &str, payload: &[u8]) -> String {
    let digest = blake3::hash(payload).to_hex();
    format!("{kind}-{target}-{}", &digest[..20])
}

fn unique_operation_key(kind: &str, target: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{kind}-{target}-{timestamp}")
}

async fn publish_and_wait(search: &RawIndexAdminClient) -> Result<(), CliError> {
    let readiness = search.readiness().await?;
    require_remote_publication_api(&readiness)?;
    let mut active = readiness
        .generation
        .as_deref()
        .and_then(|generation| generation.parse::<u64>().ok())
        .unwrap_or_default();
    let required_sequence = readiness.last_sequence.unwrap_or_default();
    let work_root = std::env::var_os(RAW_PUBLICATION_WORK_DIRECTORY_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/ytsearch/raw-generation-work"));
    validate_publication_work_root(&work_root)?;
    fs::create_dir_all(&work_root)?;
    let _work_lock = PublicationWorkLock::acquire(&work_root)?;
    for job_id in publication_receipt::recover_cleanup_and_list_completed_jobs(&work_root)? {
        publication_receipt::reconcile_cleanup(
            &work_root,
            search.publication_status(&job_id).await,
        )?;
    }
    if required_sequence <= active {
        println!("Raw has no pending mutations to publish");
        return Ok(());
    }
    let base_chunks = required_publication_path("RAW_SEARCH_BASE_CHUNKS_GZIP")?;
    let base_channels = required_publication_path("RAW_SEARCH_BASE_CHANNELS_GZIP")?;
    let started = Instant::now();
    while active < required_sequence {
        let job = search.prepare_publication().await?;
        let published =
            publish_generation(search, job, &base_chunks, &base_channels, &work_root).await?;
        if published <= active {
            return Err(CliError::Message(format!(
                "Raw publication made no progress toward required sequence {required_sequence}; active={active}, returned={published}"
            )));
        }
        active = published;
    }
    // Durable per-publication evidence in the worker log: total time and the
    // process's peak resident memory.
    println!(
        "Published required Raw sequence {required_sequence} in {} s, peak memory {}",
        started.elapsed().as_secs(),
        peak_resident_memory().unwrap_or_else(|| "unknown".to_string())
    );
    Ok(())
}

/// This process's peak resident set size (`VmHWM`), on Linux.
fn peak_resident_memory() -> Option<String> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let kilobytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(format!("{} MB", kilobytes / 1024))
}

fn require_publication_build_space(work_root: &Path) -> Result<(), CliError> {
    let minimum_free = publication_minimum_free_bytes(
        std::env::var(RAW_PUBLICATION_MIN_FREE_BYTES_ENV)
            .ok()
            .as_deref(),
    )?;
    let available = available_space(work_root)?;
    if available < minimum_free {
        return Err(CliError::Message(format!(
            "Raw publication builder has {available} free bytes; at least {minimum_free} are required"
        )));
    }

    Ok(())
}

fn require_remote_publication_api(readiness: &BackendReadiness) -> Result<(), CliError> {
    // A reader that reports its accepted generation format must match the
    // format this worker builds; otherwise every publication would build for
    // an hour and then fail its compatibility check.
    if let Some(format) = readiness.generation_format.as_deref()
        && format != youtube_raw_search::RAW_GENERATION_FORMAT
    {
        return Err(CliError::Message(format!(
            "Raw reader accepts generation format {format}, but this worker builds {}; deploy matching reader and worker images before indexing",
            youtube_raw_search::RAW_GENERATION_FORMAT
        )));
    }
    if readiness.publication_api_version == Some(RAW_PUBLICATION_API_VERSION) {
        return Ok(());
    }
    let actual = readiness
        .publication_api_version
        .map_or_else(|| "missing".to_string(), |version| version.to_string());
    Err(CliError::Message(format!(
        "Raw reader publication API is incompatible: expected version {RAW_PUBLICATION_API_VERSION}, got {actual}; deploy a matching Raw reader before indexing"
    )))
}

async fn publish_generation(
    search: &RawIndexAdminClient,
    job: RawPublicationJob,
    base_chunks: &Path,
    base_channels: &Path,
    work_root: &Path,
) -> Result<u64, CliError> {
    if job.status == RawPublicationStatus::Activated {
        println!("Raw generation {} is already active", job.target_sequence);
        return Ok(job.target_sequence);
    }

    if !is_publication_job_id(&job.job_id) {
        return Err(CliError::Message(
            "Raw publication returned an invalid job ID".into(),
        ));
    }
    let job_root = work_root.join(&job.job_id);
    let generation_path = job_root.join("generation");
    let manifest = if matches!(
        publication_receipt::prepare_workspace(&job_root, &job)?,
        publication_receipt::Workspace::Resume
    ) {
        let root = job_root.clone();
        let job = job.clone();
        let builder =
            GenerationBuilder::new(base_chunks.to_path_buf(), base_channels.to_path_buf());
        tokio::task::spawn_blocking(move || {
            publication_receipt::resume_verified_build(&root, &job, &builder)
        })
        .await
        .map_err(|error| CliError::Message(format!("Raw resume task failed: {error}")))??
    } else {
        require_publication_build_space(work_root)?;
        let snapshot_path = job_root.join("mutations.sqlite3");
        println!(
            "Downloading Raw mutation snapshot through sequence {}",
            job.target_sequence
        );
        search
            .download_publication_snapshot(&job, &snapshot_path)
            .await?;

        let generation_path = generation_path.clone();
        let compatibility = job.compatibility.clone();
        let base_chunks = base_chunks.to_path_buf();
        let base_channels = base_channels.to_path_buf();
        let target = job.target_sequence;
        let build_started = Instant::now();
        let detached = tokio::task::spawn_blocking(move || {
            let snapshot = MutationStore::open_snapshot_read_only(&snapshot_path)?;
            GenerationBuilder::new(base_chunks, base_channels).build(
                &snapshot,
                &compatibility,
                &generation_path,
            )
        })
        .await
        .map_err(|error| {
            CliError::Message(format!("Raw publication builder task failed: {error}"))
        })??;
        println!(
            "Built Raw generation {target} in {} s",
            build_started.elapsed().as_secs()
        );
        if detached.sequence != target {
            return Err(CliError::Message(format!(
                "Raw detached generation sequence {} does not match job {target}",
                detached.sequence
            )));
        }

        let manifest: RawGenerationManifest = serde_json::from_reader(fs::File::open(
            detached.path.join("manifest.json"),
        )?)
        .map_err(|error| {
            CliError::Message(format!(
                "failed to decode detached generation manifest: {error}"
            ))
        })?;
        publication_receipt::record_verified_build(&job_root, &job)?;
        manifest
    };
    let target = job.target_sequence;
    let mut state = search.accept_publication_manifest(&job, manifest).await?;
    for file_name in RAW_GENERATION_FILES {
        if state.completed_files.contains(file_name) {
            continue;
        }
        println!("Uploading Raw generation file {file_name}");
        state = search
            .upload_publication_file(&job.job_id, file_name, &generation_path.join(file_name))
            .await?;
    }
    let activation_gate = PublicationActivationGate::from_environment()?;
    if let Some(gate) = &activation_gate {
        gate.wait(Duration::from_secs(RAW_PUBLICATION_ACTIVATION_WAIT_SECONDS))
            .await?;
        gate.record_started()?;
    }
    state = search.activate_publication(&job.job_id).await?;
    if let Some(gate) = &activation_gate {
        gate.record_finished()?;
    }
    if state.status != RawPublicationStatus::Activated || state.target_sequence != target {
        return Err(CliError::Message(
            "Raw publication activation returned an inconsistent job state".to_string(),
        ));
    }
    publication_receipt::cleanup_activated_build(work_root, &state)?;
    println!("Published Raw generation {target}");
    Ok(target)
}

#[derive(Debug)]
struct PublicationActivationGate {
    ready: PathBuf,
    proceed: PathBuf,
    started: PathBuf,
    finished: PathBuf,
}

impl PublicationActivationGate {
    fn from_environment() -> Result<Option<Self>, CliError> {
        Self::from_paths([
            std::env::var_os(RAW_PUBLICATION_ACTIVATION_READY_FILE_ENV).map(PathBuf::from),
            std::env::var_os(RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE_ENV).map(PathBuf::from),
            std::env::var_os(RAW_PUBLICATION_ACTIVATION_STARTED_FILE_ENV).map(PathBuf::from),
            std::env::var_os(RAW_PUBLICATION_ACTIVATION_FINISHED_FILE_ENV).map(PathBuf::from),
        ])
    }

    fn from_paths(paths: [Option<PathBuf>; 4]) -> Result<Option<Self>, CliError> {
        if paths.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [Some(ready), Some(proceed), Some(started), Some(finished)] = paths else {
            return Err(CliError::Message(
                "all Raw publication activation handshake paths must be set together".to_string(),
            ));
        };
        if [&ready, &proceed, &started, &finished]
            .iter()
            .any(|path| !path.is_absolute())
        {
            return Err(CliError::Message(
                "Raw publication activation handshake paths must be absolute".to_string(),
            ));
        }
        Ok(Some(Self {
            ready,
            proceed,
            started,
            finished,
        }))
    }

    async fn wait(&self, maximum: Duration) -> Result<(), CliError> {
        fs::write(&self.ready, b"ready\n")?;
        let deadline = Instant::now() + maximum;
        while !self.proceed.is_file() {
            if Instant::now() >= deadline {
                return Err(CliError::Message(
                    "Raw publication activation handshake timed out".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }

    fn record_started(&self) -> Result<(), CliError> {
        self.record_timestamp(&self.started)
    }

    fn record_finished(&self) -> Result<(), CliError> {
        self.record_timestamp(&self.finished)
    }

    fn record_timestamp(&self, path: &Path) -> Result<(), CliError> {
        let milliseconds = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|error| {
                CliError::Message(format!("system clock precedes Unix epoch: {error}"))
            })?
            .as_millis();
        fs::write(path, format!("{milliseconds}\n"))?;
        Ok(())
    }
}

async fn publish_command(command: PublishCommand) -> Result<(), CliError> {
    let search = command.search.client(command.collection.collection)?;
    publish_and_wait(&search).await
}

fn required_publication_path(name: &str) -> Result<PathBuf, CliError> {
    std::env::var_os(name).map(PathBuf::from).ok_or_else(|| {
        CliError::Message(format!(
            "{name} is required for external Raw generation publication"
        ))
    })
}

fn validate_publication_work_root(path: &Path) -> Result<(), CliError> {
    if !path.is_absolute() || path.parent().is_none() || path == Path::new("/") {
        return Err(CliError::Message(format!(
            "{RAW_PUBLICATION_WORK_DIRECTORY_ENV} must be an absolute, non-root directory"
        )));
    }
    Ok(())
}

struct PublicationWorkLock {
    _file: fs::File,
}

impl PublicationWorkLock {
    fn acquire(root: &Path) -> Result<Self, CliError> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("builder.lock"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(CliError::Message(
                    "another Raw generation builder already owns the publication workspace"
                        .to_string(),
                ));
            }
        }
        Ok(Self { _file: file })
    }
}

fn is_publication_job_id(value: &str) -> bool {
    value.len() == 37
        && value.as_bytes()[..20].iter().all(u8::is_ascii_digit)
        && value.as_bytes()[20] == b'-'
        && value.as_bytes()[21..].iter().all(u8::is_ascii_hexdigit)
}

fn publication_minimum_free_bytes(value: Option<&str>) -> Result<u64, CliError> {
    value
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                CliError::Message(format!(
                    "{RAW_PUBLICATION_MIN_FREE_BYTES_ENV} must be an integer number of bytes"
                ))
            })
        })
        .transpose()
        .map(|value| value.unwrap_or(DEFAULT_RAW_PUBLICATION_MIN_FREE_BYTES))
}

#[cfg(unix)]
fn available_space(path: &Path) -> Result<u64, CliError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| CliError::Message("publication path contains a NUL byte".to_string()))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` points to writable storage.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful `statvfs` initialized the structure.
    let stats = unsafe { stats.assume_init() };
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn available_space(_path: &Path) -> Result<u64, CliError> {
    Ok(u64::MAX)
}

fn raw_publication_timeout(value: Option<&str>) -> Result<Duration, CliError> {
    let Some(value) = value else {
        return Ok(DEFAULT_RAW_PUBLICATION_TIMEOUT);
    };
    let seconds = value.parse::<u64>().map_err(|_| {
        CliError::Message(format!(
            "{RAW_PUBLICATION_TIMEOUT_ENV} must be an integer number of seconds"
        ))
    })?;
    if !(60..=MAX_RAW_PUBLICATION_TIMEOUT_SECONDS).contains(&seconds) {
        return Err(CliError::Message(format!(
            "{RAW_PUBLICATION_TIMEOUT_ENV} must be between 60 and {MAX_RAW_PUBLICATION_TIMEOUT_SECONDS} seconds"
        )));
    }
    Ok(Duration::from_secs(seconds))
}

async fn all_indexed_video_ids(
    search: &RawIndexAdminClient,
    channel_id: &str,
) -> Result<HashSet<String>, CliError> {
    let mut video_ids = HashSet::new();
    let mut cursor = None;
    let limit = NonZeroU16::new(10_000).expect("non-zero page size");
    loop {
        let page = search
            .indexed_video_ids(channel_id, cursor.as_deref(), limit)
            .await?;
        video_ids.extend(page.video_ids);
        let Some(next_cursor) = page.next_cursor else {
            return Ok(video_ids);
        };
        cursor = Some(next_cursor);
    }
}

async fn remove_youtuber_command(command: RemoveYoutuberCommand) -> Result<(), CliError> {
    validate_remove_safety(command.allow_destructive_remove)?;
    let handle = normalize_handle(&command.youtuber.handle)?;
    let youtuber_id = youtuber_id_from_handle(&handle, "");
    let transcript_search = command.search.client(command.collection.collection)?;
    let operation = transcript_search
        .delete_channel(
            &youtuber_id,
            ChannelDeletionScope::ChannelAndVideos,
            &unique_operation_key("delete-channel", &youtuber_id),
        )
        .await?;
    publish_and_wait(&transcript_search).await?;
    println!(
        "Deleted channel {handle} and its transcript chunks (operation {})",
        operation.id
    );

    Ok(())
}

fn validate_remove_safety(allow_destructive_remove: bool) -> Result<(), CliError> {
    if !allow_destructive_remove {
        return Err(CliError::Message(
            "removing a YouTuber permanently deletes its profile and transcript chunks; pass --allow-destructive-remove to confirm"
                .to_string(),
        ));
    }
    Ok(())
}

async fn index_youtuber(command: IndexYoutuberCommand) -> Result<(), CliError> {
    let transcribe_source = command.run.transcript.transcriber().await?;
    index_youtuber_with_transcriber(command, transcribe_source).await
}

async fn index_youtuber_with_transcriber(
    command: IndexYoutuberCommand,
    transcribe_source: Option<AudioTranscriptSource>,
) -> Result<(), CliError> {
    let run = &command.run;
    let publication = PublicationPolicy::from_defer_flag(run.defer_publish);
    let existing_data =
        ExistingDataPolicy::from_cli(run.mode, command.no_wipe, command.skip_existing);
    validate_existing_data_policy(existing_data, run.allow_destructive_rewipe)?;
    let transcript_search = run.search.client(run.collection.collection.clone())?;
    transcript_search.readiness().await?;

    let youtube = run.youtube.optional_client()?;
    let caption_source = run.transcript.caption_source();
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

    let youtuber_id = youtuber_id_from_handle(&youtuber.handle, &youtuber.channel_id);
    let discovery_manifest = write_complete_discovery(
        &run.transcript.transcript_cache_dir,
        &youtuber_id,
        &youtuber.handle,
        &public_videos,
    )?;
    println!(
        "Persisted complete discovery evidence: {}",
        discovery_manifest.display()
    );

    let initial_youtuber_doc = upsert_youtuber_document_from_chunks(
        &transcript_search,
        &youtuber,
        &youtuber_id,
        saturating_u32(public_videos.len()),
    )
    .await?;
    println!(
        "Published in-progress YouTuber profile: {}",
        initial_youtuber_doc.id
    );
    let refresh_after_first_completion = existing_data.channel == ExistingChannelPolicy::Rewipe
        || initial_youtuber_doc.indexed_chunk_count == 0;

    match existing_data.channel {
        ExistingChannelPolicy::Retain => {
            println!("Keeping existing transcript chunks for {}", youtuber.handle);
        }
        ExistingChannelPolicy::Rewipe => {
            println!(
                "Deleting existing transcript chunks for {} before reindex",
                youtuber.handle
            );
            let operation = transcript_search
                .delete_channel(
                    &youtuber_id,
                    ChannelDeletionScope::ChannelAndVideos,
                    &unique_operation_key("rewipe-channel", &youtuber_id),
                )
                .await?;
            println!("Queued Raw rewipe operation: {}", operation.id);
        }
    }

    let metadata_source = metadata_source(youtube.as_ref(), &caption_source);
    let chunker = run.transcript.chunker();
    let mut stats = IndexStats {
        discovered_videos: public_videos.len(),
        ..IndexStats::default()
    };
    let stop_after_existing = normalized_stop_after_existing(run.stop_after_existing);
    let videos_to_index = prepare_videos_to_index(
        &public_videos,
        &transcript_search,
        existing_data.skips_existing_videos(),
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
        existing_data.skips_existing_videos(),
        stop_after_existing,
        videos_to_index.len(),
    );
    let existing_video_ids = if existing_data.skips_existing_videos() && !use_worker_existing_checks
    {
        if videos_to_index.is_empty() {
            HashSet::new()
        } else {
            println!("Loading existing indexed video ids for {}", youtuber.handle);
            let existing = all_indexed_video_ids(&transcript_search, &youtuber_id).await?;
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
    let mut videos_to_index = videos_to_index
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
    let failure_ledger = FailureLedger::new(
        run.transcript.transcript_cache_dir.join(".failures"),
        Duration::from_secs(run.transcript_failure_retry_seconds),
    );
    defer_recent_transcript_failures(
        &mut videos_to_index,
        &failure_ledger,
        &youtuber_id,
        &mut stats,
        Some(&caption_source),
        transcribe_source.is_some(),
    );
    if let Some(limit) = command.max_new_videos {
        videos_to_index.truncate(limit);
        println!(
            "Capped this update pass at {} missing video(s)",
            videos_to_index.len()
        );
    }
    if existing_video_count > 0 {
        println!(
            "Skipped {} already-indexed video(s) before starting workers",
            stats.existing_videos
        );
    } else if use_worker_existing_checks {
        println!(
            "Using per-video existing checks for {} recent candidate video(s)",
            videos_to_index.len()
        );
    }

    let mut tasks = JoinSet::new();
    let worker_total = videos_to_index.len();
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

            let worker = index_discovered_video(
                index,
                worker_total,
                video.clone(),
                youtuber_id.clone(),
                metadata_source.clone(),
                caption_source.clone(),
                transcribe_source.clone(),
                chunker.clone(),
                transcript_search.clone(),
                run.content_kind,
                use_worker_existing_checks,
                failure_ledger.clone(),
            );
            let timeout = Duration::from_secs(run.video_timeout_seconds);
            let failure_ledger = failure_ledger.clone();
            let youtuber_id = youtuber_id.clone();
            tasks.spawn(async move {
                tokio::time::timeout(timeout, worker)
                    .await
                    .unwrap_or_else(|_| {
                        let category = failure_ledger.record(
                            &video.video_id,
                            &youtuber_id,
                            FailureStage::Timeout,
                            "video indexing exceeded its timeout",
                        );
                        Ok(IndexVideoOutcome::Skipped {
                            video_id: video.video_id,
                            title: video.title,
                            reason: "timeout".to_string(),
                            category,
                        })
                    })
            });
        }

        let Some(result) = tasks.join_next().await else {
            break;
        };
        active_jobs = active_jobs.saturating_sub(1);
        completed_videos += 1;
        match result {
            Ok(Ok(outcome)) => {
                let blocking_category = outcome.upstream_blocking_category();
                apply_index_outcome(&mut stats, outcome, completed_videos, worker_total);
                if let Some(category) = blocking_category {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    print_index_summary(&stats);
                    return Err(CliError::UpstreamBlocked(category.as_str()));
                }
            }
            Ok(Err(error)) => {
                stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                stats.retryable_failures = stats.retryable_failures.saturating_add(1);
                tracing::warn!(%error, "skipped video after indexing task failed");
                println!("Done {completed_videos}/{worker_total}: skipped indexing error: {error}");
            }
            Err(error) => {
                stats.skipped_videos = stats.skipped_videos.saturating_add(1);
                stats.retryable_failures = stats.retryable_failures.saturating_add(1);
                tracing::warn!(%error, "skipped video after indexing task panicked");
                println!("Done {completed_videos}/{worker_total}: skipped task error: {error}");
            }
        }

        let run_indexed_videos = stats.indexed_videos.saturating_add(stats.existing_videos);
        let run_indexed_chunks = stats.indexed_chunks.saturating_add(stats.existing_chunks);
        let should_refresh_youtuber_doc = run_indexed_videos > 0
            && run_indexed_chunks > 0
            && ((refresh_after_first_completion && last_youtuber_doc_refresh_completed == 0)
                || completed_videos.saturating_sub(last_youtuber_doc_refresh_completed) >= 25);
        if should_refresh_youtuber_doc {
            match upsert_youtuber_document_from_chunks(
                &transcript_search,
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

    let retained_existing = existing_data.skips_existing_videos() && stats.existing_videos > 0;
    if stats.attempted_videos > 0
        && (run_indexed_videos == 0 || run_indexed_chunks == 0)
        && !retained_existing
    {
        if let Err(error) = transcript_search
            .delete_channel(
                &youtuber_id,
                ChannelDeletionScope::ChannelAndVideos,
                &unique_operation_key("remove-empty-channel", &youtuber_id),
            )
            .await
        {
            tracing::warn!(handle = %youtuber.handle, %error, "failed to remove unsearchable YouTuber document");
        } else {
            println!("Removed unsearchable YouTuber document: {youtuber_id}");
        }
        if publication.publishes_after_run() {
            publish_and_wait(&transcript_search).await?;
        }
        let message = format!(
            "No searchable transcript chunks were imported for {}. attempted={}, skipped={}, kind_filtered={}. Check yt-dlp/caption errors above.",
            youtuber.handle,
            stats.attempted_videos,
            stats.skipped_videos,
            stats.kind_filtered_videos
        );
        return if stats.retryable_failures > 0 {
            Err(CliError::RetryableGaps(message))
        } else {
            Err(CliError::Message(message))
        };
    }

    let youtuber_doc = upsert_youtuber_document_from_chunks(
        &transcript_search,
        &youtuber,
        &youtuber_id,
        public_video_count,
    )
    .await?;
    println!("Upserted YouTuber document: {}", youtuber_doc.id);
    if publication.publishes_after_run() {
        publish_and_wait(&transcript_search).await?;
    }

    completed_index_result(&youtuber.handle, &stats)
}

fn completed_index_result(handle: &str, stats: &IndexStats) -> Result<(), CliError> {
    if stats.retryable_failures == 0 {
        return Ok(());
    }
    Err(CliError::RetryableGaps(format!(
        "Indexing {handle} retained {} retryable video gap(s); the queue request must remain for retry",
        stats.retryable_failures
    )))
}

/// The run finished; only per-video gaps with a scheduled retry remain.
const EXIT_RETRYABLE_GAPS: u8 = 75;
/// A bot challenge or rate limit stopped the run before every video was tried.
const EXIT_UPSTREAM_BLOCKED: u8 = 69;
/// A transient index backend failure stopped the run; retry without a YouTube cooldown.
const EXIT_BACKEND_UNAVAILABLE: u8 = 70;

/// A catalog run is complete when every creator was attempted and the only
/// failures are retryable per-video gaps: those stay in the failure ledger
/// with their own backoff, and a later pass retries them. Any other creator
/// failure fails the run.
fn catalog_outcome(failures: Vec<(String, CliError)>) -> Result<(), CliError> {
    if failures.is_empty() {
        return Ok(());
    }
    let only_gaps = failures
        .iter()
        .all(|(_, error)| matches!(error, CliError::RetryableGaps(_)));
    let only_backend_or_gaps = failures.iter().all(|(_, error)| {
        matches!(
            error,
            CliError::RetryableGaps(_) | CliError::IndexStore(IndexStoreError::Unavailable { .. })
        )
    });
    let summary = failures
        .iter()
        .map(|(handle, error)| format!("{handle}: {error}"))
        .collect::<Vec<_>>()
        .join(" | ");
    if only_gaps {
        Err(CliError::RetryableGaps(format!(
            "{} YouTuber(s) retained retryable video gaps after every requested creator was attempted: {summary}",
            failures.len()
        )))
    } else if only_backend_or_gaps {
        Err(IndexStoreError::unavailable(
            "raw",
            format!(
                "{} YouTuber(s) could not complete because the backend was unavailable: {summary}",
                failures.len()
            ),
        )
        .into())
    } else {
        Err(CliError::Message(format!(
            "{} YouTuber(s) failed after every requested creator was attempted: {summary}",
            failures.len()
        )))
    }
}

async fn index_youtubers_command(command: IndexYoutubersCommand) -> Result<(), CliError> {
    // Construct and preflight once: clones share the invocation-wide ASR budget.
    let transcribe_source = command.run.transcript.transcriber().await?;
    let cache_dir = &command.run.transcript.transcript_cache_dir;
    let publication_search = command
        .run
        .search
        .client(command.run.collection.collection.clone())?;
    let handles =
        reconciliation_manifest::resume_order(cache_dir, read_youtuber_list(&command.list_file)?)?;
    let reconciliation_run_id = begin_catalog_run(cache_dir)?;
    println!("Started catalog reconciliation run: {reconciliation_run_id}");
    let channel_jobs = effective_channel_jobs(command.channel_jobs, command.jobs);
    println!(
        "Indexing creators with {channel_jobs} concurrent creator job(s) and up to {} video worker(s) each",
        command.jobs.clamp(1, 16)
    );
    let mut handles = handles.into_iter();
    let mut tasks = JoinSet::new();
    let mut failures = Vec::new();

    loop {
        while tasks.len() < channel_jobs {
            let Some(handle) = handles.next() else {
                break;
            };
            let failed_handle = handle.clone();
            // Update mode ignores the per-video flags; rewipe reindexes everything.
            let child = IndexYoutuberCommand {
                youtuber: YoutuberArg { handle },
                run: IndexRunArgs {
                    defer_publish: true,
                    ..command.run.clone()
                },
                max_videos: command.max_videos,
                max_new_videos: None,
                jobs: command.jobs,
                skip_existing: false,
                no_wipe: false,
            };
            let transcribe_source = transcribe_source.clone();
            tasks.spawn(async move {
                (
                    failed_handle,
                    index_youtuber_with_transcriber(child, transcribe_source).await,
                )
            });
        }

        let Some(result) = tasks.join_next().await else {
            break;
        };
        match result {
            Ok((_, Ok(()))) => {}
            Ok((blocked_handle, Err(error @ CliError::UpstreamBlocked(_)))) => {
                tasks.abort_all();
                reconciliation_manifest::remember_blocked_creator(
                    cache_dir,
                    Some(&blocked_handle),
                )?;
                println!(
                    "Catalog pass blocked at {blocked_handle}; the next pass resumes after it"
                );
                return Err(error);
            }
            Ok((failed_handle, Err(error))) => {
                tracing::error!(handle = %failed_handle, %error, "YouTuber indexing failed; continuing with remaining creators");
                failures.push((failed_handle, error));
            }
            Err(error) => {
                tracing::error!(%error, "YouTuber indexing task failed; continuing with remaining creators");
                failures.push((
                    "indexing task".to_string(),
                    CliError::Message(error.to_string()),
                ));
            }
        }
    }

    // Every creator was attempted, so the next pass starts from the top.
    reconciliation_manifest::remember_blocked_creator(cache_dir, None)?;
    if PublicationPolicy::from_defer_flag(command.run.defer_publish).publishes_after_run() {
        publish_and_wait(&publication_search).await?;
    }
    catalog_outcome(failures)
}

fn effective_channel_jobs(requested: usize, video_jobs: usize) -> usize {
    let video_jobs = video_jobs.clamp(1, 16);
    requested.clamp(1, 4).min(16 / video_jobs).max(1)
}

fn validate_existing_data_policy(
    policy: ExistingDataPolicy,
    allow_destructive_rewipe: bool,
) -> Result<(), CliError> {
    if policy.channel == ExistingChannelPolicy::Rewipe && policy.videos == ExistingVideoPolicy::Skip
    {
        return Err(CliError::Message(
            "cannot combine rewipe with skip-existing: the pending channel deletion would hide videos that the active generation still reports as indexed"
                .to_string(),
        ));
    }
    if policy.channel == ExistingChannelPolicy::Rewipe && !allow_destructive_rewipe {
        return Err(CliError::Message(
            "rewipe deletes existing chunks before replacement captions are confirmed; use update or backend copy when possible, or pass --allow-destructive-rewipe after verifying a recoverable source"
                .to_string(),
        ));
    }
    Ok(())
}

fn read_youtuber_list(path: &Path) -> Result<Vec<String>, CliError> {
    let body = std::fs::read_to_string(path).map_err(|error| {
        CliError::Message(format!("failed to read {}: {error}", path.display()))
    })?;
    let handles: Vec<_> = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split('|').next())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if handles.is_empty() {
        Err(CliError::Message(format!(
            "no YouTube handles found in {}",
            path.display()
        )))
    } else {
        Ok(handles)
    }
}

async fn prepare_videos_to_index(
    public_videos: &[DiscoveredVideo],
    search: &RawIndexAdminClient,
    skip_existing: bool,
    max_videos: Option<usize>,
    stop_after_existing: Option<usize>,
    stats: &mut IndexStats,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let limit = max_videos.unwrap_or(public_videos.len());
    let candidates = public_videos.iter().take(limit);

    let (true, Some(stop_after_existing)) = (skip_existing, stop_after_existing) else {
        let videos = candidates.cloned().collect::<Vec<_>>();
        stats.attempted_videos = videos.len();
        return Ok(videos);
    };
    let mut videos = Vec::new();
    let mut consecutive_existing = 0usize;

    for video in candidates.cloned() {
        let existing_chunks = search.video_chunk_count(&video.video_id).await?;
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

fn defer_recent_transcript_failures(
    videos: &mut Vec<DiscoveredVideo>,
    failure_ledger: &FailureLedger,
    creator_id: &str,
    stats: &mut IndexStats,
    caption_source: Option<&YtDlpTranscriptSource>,
    fallback_ready: bool,
) {
    videos.retain(|video| {
        failure_ledger.attribute(&video.video_id, creator_id);
        let cached = caption_source.is_some_and(|source| {
            match source.cached_transcript(&video.video_id) {
                Ok(transcript) => transcript.is_some(),
                Err(error) => {
                    tracing::warn!(video_id = %video.video_id, %error, "cannot inspect recovery transcript cache");
                    false
                }
            }
        });
        let decision = failure_ledger.decision_for(&video.video_id, fallback_ready, cached);
        if decision.deferred {
            stats.deferred_transcript_failures =
                stats.deferred_transcript_failures.saturating_add(1);
            if let Some(category) = decision.category {
                increment_category(&mut stats.deferred_failure_categories, category);
                if category.retryable() {
                    stats.retryable_failures = stats.retryable_failures.saturating_add(1);
                }
            }
        }
        !decision.deferred
    });

    if stats.deferred_transcript_failures > 0 {
        println!(
            "Deferred {} classified video failure(s): {}",
            stats.deferred_transcript_failures,
            category_summary(&stats.deferred_failure_categories)
        );
    }
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

impl TranscriptOptions {
    fn caption_source(&self) -> YtDlpTranscriptSource {
        let source = YtDlpTranscriptSource::new();
        if self.no_transcript_cache {
            source
        } else {
            source.with_cache_dir(self.transcript_cache_dir.clone())
        }
    }

    async fn transcriber(&self) -> Result<Option<AudioTranscriptSource>, CliError> {
        if !self.transcribe_missing {
            return Ok(None);
        }
        let backend = match self.transcription_backend {
            TranscriptionBackendArg::Voxtype => TranscriptionBackend::Voxtype,
            TranscriptionBackendArg::WhisperCpp => TranscriptionBackend::WhisperCpp {
                binary: self.whisper_binary.clone(),
                model: self.whisper_model.clone().ok_or_else(|| {
                    CliError::Message("--whisper-model is required for whisper-cpp".to_string())
                })?,
                threads: usize::from(self.whisper_threads),
            },
        };
        let source = AudioTranscriptSource::with_config(
            TranscriptionConfig::default()
                .with_cache_dir(self.transcription_cache_dir.clone())
                .with_caption_cache_dir(self.transcript_cache_dir.clone())
                .with_chunk_seconds(self.transcription_chunk_seconds)
                .with_voxtype_engine(self.voxtype_engine.clone())
                .with_backend(backend)
                .with_budget(Duration::from_secs(self.transcription_budget_seconds)),
        );
        source.preflight().await?;
        Ok(Some(source))
    }

    fn chunker(&self) -> Chunker {
        Chunker::new(self.chunk_seconds)
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
        category: FailureCategory,
    },
}

impl IndexVideoOutcome {
    fn upstream_blocking_category(&self) -> Option<FailureCategory> {
        match self {
            Self::Skipped { category, .. } if category.blocks_run() => Some(*category),
            _ => None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn index_discovered_video(
    index: usize,
    total: usize,
    video: DiscoveredVideo,
    youtuber_id: String,
    metadata_source: Arc<FallbackMetadataSource>,
    caption_source: YtDlpTranscriptSource,
    transcribe_source: Option<AudioTranscriptSource>,
    chunker: Chunker,
    search: RawIndexAdminClient,
    content_kind_filter: ContentKindFilter,
    skip_existing: bool,
    failure_ledger: FailureLedger,
) -> Result<IndexVideoOutcome, CliError> {
    let indexing_started = Instant::now();
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

    let existing_check_started = Instant::now();
    if skip_existing {
        let existing_chunks = search.video_chunk_count(&video.video_id).await?;
        if existing_chunks > 0 {
            tracing::info!(
                video_id = %video.video_id,
                existing_check_ms = elapsed_ms(existing_check_started),
                total_ms = elapsed_ms(indexing_started),
                "indexing video already exists"
            );
            return Ok(IndexVideoOutcome::Existing {
                video_id: video.video_id,
                title: video.title,
                chunks: existing_chunks,
            });
        }
    }
    let existing_check_ms = elapsed_ms(existing_check_started);

    let metadata_started = Instant::now();
    let mut metadata = match metadata_source.metadata_for_video(&video.video_id).await {
        Ok(metadata) => metadata,
        Err(error) => {
            tracing::warn!(
                video_id = %video.video_id,
                %error,
                metadata_ms = elapsed_ms(metadata_started),
                total_ms = elapsed_ms(indexing_started),
                "skipped video metadata"
            );
            return Ok(skipped_video(
                video,
                &failure_ledger,
                &youtuber_id,
                FailureStage::Metadata,
                error,
            ));
        }
    };
    let metadata_ms = elapsed_ms(metadata_started);
    metadata.youtuber.id = youtuber_id;

    let content_kind = content_kind_for_duration(metadata.duration_ms);
    if !content_kind_filter.includes(content_kind) {
        caption_source.discard_video_json(&video.video_id).await;
        failure_ledger.clear(&video.video_id);
        tracing::info!(
            video_id = %video.video_id,
            ?content_kind,
            existing_check_ms,
            metadata_ms,
            total_ms = elapsed_ms(indexing_started),
            "skipped by content kind"
        );
        return Ok(IndexVideoOutcome::KindFiltered {
            video_id: video.video_id,
            title: video.title,
            content_kind,
        });
    }

    let transcript_started = Instant::now();
    let (transcript, audio_transcribed) = match fetch_transcript_with_optional_fallback(
        &video.video_id,
        &caption_source,
        transcribe_source.as_ref(),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(
                video_id = %video.video_id,
                %error,
                transcript_ms = elapsed_ms(transcript_started),
                total_ms = elapsed_ms(indexing_started),
                "skipped transcript"
            );
            let reason = error.to_string();
            let category = match &error {
                CliError::TranscriptFallback { source, .. } => failure_ledger
                    .record_transcription_failure(
                        &video.video_id,
                        &metadata.youtuber.id,
                        &source.to_string(),
                        &reason,
                        matches!(source, AudioTranscriptionError::Deferred),
                    ),
                _ => failure_ledger.record_transcript_failure(
                    &video.video_id,
                    &metadata.youtuber.id,
                    &reason,
                    metadata.published_at.as_deref(),
                ),
            };
            return Ok(IndexVideoOutcome::Skipped {
                video_id: video.video_id,
                title: video.title,
                reason: format!("transcript: {reason}"),
                category,
            });
        }
    };
    let transcript_ms = elapsed_ms(transcript_started);

    let chunking_started = Instant::now();
    let chunks = match chunker.chunks(&metadata, &transcript) {
        Ok(chunks) => chunks,
        Err(error) => {
            tracing::warn!(
                video_id = %video.video_id,
                %error,
                chunking_ms = elapsed_ms(chunking_started),
                total_ms = elapsed_ms(indexing_started),
                "skipped chunking"
            );
            return Ok(skipped_video(
                video,
                &failure_ledger,
                &metadata.youtuber.id,
                FailureStage::Chunking,
                error,
            ));
        }
    };
    let chunking_ms = elapsed_ms(chunking_started);

    let chunk_count = chunks.len();
    let import_started = Instant::now();
    if let Err(error) = replace_video_document(&search, &video.video_id, &chunks).await {
        tracing::warn!(
            video_id = %video.video_id,
            %error,
            import_ms = elapsed_ms(import_started),
            total_ms = elapsed_ms(indexing_started),
            "skipped import"
        );
        return Ok(skipped_video(
            video,
            &failure_ledger,
            &metadata.youtuber.id,
            FailureStage::Import,
            error,
        ));
    }
    let import_ms = elapsed_ms(import_started);
    failure_ledger.clear(&video.video_id);
    tracing::info!(
        video_id = %video.video_id,
        chunk_count,
        audio_transcribed,
        existing_check_ms,
        metadata_ms,
        transcript_ms,
        chunking_ms,
        import_ms,
        total_ms = elapsed_ms(indexing_started),
        "indexed video performance"
    );

    Ok(IndexVideoOutcome::Indexed {
        video_id: video.video_id,
        title: video.title,
        chunks: chunk_count,
        audio_transcribed,
    })
}

/// Records a classified failure for `video` and reports it as skipped.
fn skipped_video(
    video: DiscoveredVideo,
    failure_ledger: &FailureLedger,
    youtuber_id: &str,
    stage: FailureStage,
    error: impl std::fmt::Display,
) -> IndexVideoOutcome {
    let error = error.to_string();
    let category = failure_ledger.record(&video.video_id, youtuber_id, stage, &error);
    IndexVideoOutcome::Skipped {
        video_id: video.video_id,
        title: video.title,
        reason: format!("{}: {error}", stage.as_str()),
        category,
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
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
            category,
        } => {
            stats.skipped_videos = stats.skipped_videos.saturating_add(1);
            increment_category(&mut stats.failure_categories, category);
            if category.retryable() {
                stats.retryable_failures = stats.retryable_failures.saturating_add(1);
            }
            println!("Done {completed}/{total}: {video_id} {title} — skipped {reason}");
        }
    }
}

async fn index_video_command(command: IndexVideoCommand) -> Result<(), CliError> {
    let video_id = parse_video_id(&command.video)?;
    let youtube = command.youtube.optional_client()?;
    let collection = command.collection.collection;
    let search = command.search.client(collection.clone())?;

    search.readiness().await?;
    if command.no_wipe {
        println!("Raw replacement keeps unrelated videos and atomically replaces this video");
    }

    let caption_source = command.transcript.caption_source();
    let metadata_source = metadata_source(youtube.as_ref(), &caption_source);
    let transcribe_source = command.transcript.transcriber().await?;
    let chunker = command.transcript.chunker();

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
    replace_video_document(&search, &video_id, &chunks).await?;
    publish_and_wait(&search).await?;

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
    let ytdlp_validation = validate_youtuber_with_ytdlp(ytdlp, &handle, max_videos);
    let (ytdlp_result, profile_result) = if let Some(youtube) = youtube {
        let (ytdlp_result, profile_result) = tokio::join!(
            ytdlp_validation,
            enrich_youtuber_channel_metadata(youtube, &handle),
        );
        (ytdlp_result, Some(profile_result))
    } else {
        (ytdlp_validation.await, None)
    };

    match ytdlp_result {
        Ok(mut youtuber) => {
            println!("  validation source: yt-dlp channel feed");
            if let Some(profile_result) = profile_result {
                match profile_result {
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
    let mut youtuber =
        profile_result.expect("profile lookup runs when the YouTube client is configured")?;
    populate_recent_uploads(youtube, &mut youtuber, max_videos).await;
    Ok(youtuber)
}

async fn populate_recent_uploads(
    youtube: &YouTubeClient,
    youtuber: &mut ValidatedYoutuber,
    max_videos: u8,
) {
    if max_videos == 0 {
        return;
    }
    let Some(uploads_playlist) = youtuber.uploads_playlist.as_deref() else {
        youtuber.uploads_warning = Some("channel did not expose an uploads playlist".to_string());
        return;
    };
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
}

async fn enrich_youtuber_channel_metadata(
    youtube: &YouTubeClient,
    handle: &str,
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

    let channel = channel_page.items.first().ok_or_else(|| {
        CliError::Message(format!("No YouTube channel found for handle {handle}"))
    })?;
    let channel_id = required_string(channel, "/id", "channel id")?;
    let title = required_string(channel, "/snippet/title", "channel title")?;
    let resolved_handle =
        optional_string(channel, "/snippet/customUrl").unwrap_or_else(|| handle.to_string());
    let uploads_playlist = optional_string(channel, "/contentDetails/relatedPlaylists/uploads");
    let avatar_url = best_thumbnail(channel.pointer("/snippet/thumbnails"));
    Ok(ValidatedYoutuber {
        handle: resolved_handle,
        channel_id,
        title,
        uploads_playlist,
        avatar_url,
        recent_uploads: Vec::new(),
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
    let limit = Some(usize::from(max_videos.max(1)));
    let mut selected_feed = None;
    let mut first_error = None;
    for tab in ["uploads", "streams", "shorts"] {
        let result = match tab {
            "uploads" => ytdlp.channel_feed(handle, limit).await,
            "streams" => ytdlp.channel_stream_feed(handle, limit).await,
            _ => ytdlp.channel_shorts_feed(handle, limit).await,
        };
        match result {
            Ok(feed) if !feed.videos.is_empty() => {
                selected_feed = Some(feed);
                break;
            }
            Ok(_) => println!("  {tab} tab is empty; checking another tab for validation"),
            Err(error) => {
                println!("  {tab} tab failed ({error}); checking another tab for validation");
                first_error.get_or_insert(error);
            }
        }
    }
    let feed = selected_feed.ok_or_else(|| match first_error {
        Some(error) => CliError::from(error),
        None => CliError::Message(format!(
            "No public uploads, streams, or Shorts found for handle {handle}"
        )),
    })?;
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
            .map(discovered_video_from_ytdlp_video)
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
    let (uploads, streams, shorts) = tokio::join!(
        ytdlp.channel_feed(handle, None),
        ytdlp.channel_stream_feed(handle, None),
        ytdlp.channel_shorts_feed(handle, None),
    );
    let uploads = uploads.map_err(|error| {
        tracing::warn!(handle = %handle, %error, "yt-dlp uploads-tab discovery failed; refusing a partial catalog");
        error
    })?;
    let streams = streams.map_err(|error| {
        tracing::warn!(handle = %handle, %error, "yt-dlp streams-tab discovery failed; refusing a partial catalog");
        error
    })?;
    let shorts = shorts.map_err(|error| {
        tracing::warn!(handle = %handle, %error, "yt-dlp shorts-tab discovery failed; refusing a partial catalog");
        error
    })?;
    let streams = streams.videos;
    let mut videos = uploads
        .videos
        .into_iter()
        .map(discovered_video_from_ytdlp_video)
        .collect::<Vec<_>>();
    println!(
        "  discovery source: yt-dlp streams tab ({} entries)",
        streams.len()
    );
    videos.extend(streams.into_iter().map(discovered_video_from_ytdlp_video));
    println!(
        "  discovery source: yt-dlp shorts tab ({} entries)",
        shorts.videos.len()
    );
    videos.extend(
        shorts
            .videos
            .into_iter()
            .map(discovered_video_from_ytdlp_video),
    );

    Ok(dedupe_discovered_videos(videos))
}

async fn discover_upload_playlist_videos(
    youtube: &YouTubeClient,
    uploads_playlist: &str,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let query = Query::new()
        .part(["snippet", "contentDetails"])
        .set("playlistId", uploads_playlist)
        .max_results(50)
        .fields("nextPageToken,items(snippet(title),contentDetails/videoId)");
    list_all_videos(
        youtube,
        ListResource::PlaylistItems,
        query,
        PLAYLIST_VIDEO_ID,
    )
    .await
}

async fn discover_public_videos(
    youtube: &YouTubeClient,
    channel_id: &str,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let query = Query::new()
        .part(["snippet"])
        .set("channelId", channel_id)
        .set("type", "video")
        .set("order", "date")
        .max_results(50)
        .fields("nextPageToken,items(id/videoId,snippet/title)");
    list_all_videos(youtube, ListResource::Search, query, "/id/videoId").await
}

const PLAYLIST_VIDEO_ID: &str = "/contentDetails/videoId";

/// Follows every `nextPageToken` and returns the deduplicated videos.
async fn list_all_videos(
    youtube: &YouTubeClient,
    resource: ListResource,
    query: Query,
    video_id_pointer: &str,
) -> Result<Vec<DiscoveredVideo>, CliError> {
    let mut videos = Vec::new();
    let mut page_token = None;
    loop {
        let mut page_query = query.clone();
        if let Some(token) = page_token {
            page_query = page_query.page_token(token);
        }
        let page = youtube.list(resource).query(page_query).page().await?;
        videos.extend(
            page.items
                .iter()
                .filter_map(|item| discovered_video_from_item(item, video_id_pointer)),
        );
        page_token = page.next_page_token;
        if page_token.is_none() {
            return Ok(dedupe_discovered_videos(videos));
        }
    }
}

fn discovered_video_from_item(item: &Value, video_id_pointer: &str) -> Option<DiscoveredVideo> {
    Some(DiscoveredVideo {
        video_id: optional_string(item, video_id_pointer)?,
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
) -> Result<Vec<DiscoveredVideo>, CliError> {
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
        .filter_map(|item| discovered_video_from_item(item, PLAYLIST_VIDEO_ID))
        .collect())
}

async fn fetch_transcript_with_optional_fallback(
    video_id: &str,
    caption_source: &YtDlpTranscriptSource,
    transcribe_source: Option<&AudioTranscriptSource>,
) -> Result<(Vec<TranscriptSegment>, bool), CliError> {
    match caption_source.transcript_for_video(video_id).await {
        Ok(transcript) => Ok((transcript, false)),
        Err(caption_error) => {
            let Some(transcribe_source) = transcribe_source else {
                return Err(caption_error.into());
            };

            if !failure_ledger::allows_audio_fallback(&caption_error.to_string()) {
                return Err(caption_error.into());
            }
            tracing::info!(video_id, error = %caption_error, "captions unavailable, trying audio transcription");
            println!("  captions unavailable, trying audio transcription: {caption_error}");
            let transcript = transcribe_source
                .transcribe(video_id)
                .await
                .map_err(|source| CliError::TranscriptFallback {
                    caption: caption_error.to_string(),
                    source,
                })?;
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
        "Index summary: discovered={}, attempted={}, indexed={}, existing={}, skipped={}, retryable_failures={}, deferred_transcript_failures={}, kind_filtered={}, audio_transcribed={}, chunks={}, existing_chunks={}",
        stats.discovered_videos,
        stats.attempted_videos,
        stats.indexed_videos,
        stats.existing_videos,
        stats.skipped_videos,
        stats.retryable_failures,
        stats.deferred_transcript_failures,
        stats.kind_filtered_videos,
        stats.audio_transcribed_videos,
        stats.indexed_chunks,
        stats.existing_chunks
    );
    if !stats.failure_categories.is_empty() {
        println!(
            "Index failure categories: {}",
            category_summary(&stats.failure_categories)
        );
    }
}

fn increment_category(categories: &mut BTreeMap<String, u32>, category: FailureCategory) {
    let count = categories.entry(category.as_str().to_string()).or_default();
    *count = count.saturating_add(1);
}

fn category_summary(categories: &BTreeMap<String, u32>) -> String {
    serde_json::to_string(categories).unwrap_or_else(|_| "{}".to_string())
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

    #[cfg(unix)]
    #[tokio::test]
    async fn channel_discovery_includes_shorts_and_rejects_partial_tabs() {
        use std::os::unix::fs::PermissionsExt;

        const CHILD: &str = "YTSEARCH_CHANNEL_DISCOVERY_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // The source reads its binary and session from the environment. Run
            // in a child so parallel tests cannot observe the fixture settings.
            let fixture = tempfile::tempdir().unwrap();
            let binary = fixture.path().join("yt-dlp");
            fs::write(
                &binary,
                r#"#!/bin/sh
set -eu
url=
for arg in "$@"; do url="$arg"; done
case "$url" in
  */@fail-videos/videos|*/@fail-streams/streams|*/@fail-shorts/shorts)
    echo 'fixture tab discovery failed' >&2
    exit 1 ;;
  */@shorts-only/videos|*/@shorts-only/streams)
    entries='[]' ;;
  */videos)
    entries='[{"id":"upload","title":"Upload"},{"id":"duplicate","title":"Duplicate"}]' ;;
  */streams)
    entries='[{"id":"stream","title":"Stream"},{"id":"duplicate","title":"Duplicate"}]' ;;
  */shorts)
    entries='[{"id":"short","title":"Short"},{"id":"duplicate","title":"Duplicate"}]' ;;
  *) exit 2 ;;
esac
printf '{"id":"UCfixture","channel_id":"UCfixture","channel":"Fixture","entries":%s}' "$entries"
"#,
            )
            .unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::channel_discovery_includes_shorts_and_rejects_partial_tabs",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("YT_DLP_BINARY", binary)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr),
            );
            return;
        }

        let source = YtDlpTranscriptSource::new();
        let videos = discover_videos_for_youtuber_with_ytdlp(&source, "@fixture")
            .await
            .unwrap();
        assert_eq!(
            videos
                .iter()
                .map(|video| video.video_id.as_str())
                .collect::<Vec<_>>(),
            ["upload", "duplicate", "stream", "short"],
        );
        for handle in ["@fail-videos", "@fail-streams", "@fail-shorts"] {
            assert!(
                discover_videos_for_youtuber_with_ytdlp(&source, handle)
                    .await
                    .is_err(),
                "{handle} must fall back instead of returning a partial catalog",
            );
        }
        let youtuber = validate_youtuber_with_ytdlp(&source, "@shorts-only", 1)
            .await
            .unwrap();
        assert_eq!(youtuber.channel_id, "UCfixture");
        assert_eq!(youtuber.recent_uploads.len(), 1);
        assert_eq!(youtuber.recent_uploads[0].video_id, "short");
    }

    #[test]
    fn admin_client_rejects_collections_raw_does_not_serve() {
        let args = args::SearchAdminArgs {
            backend: args::SearchBackend::Raw,
            api_url: Some("http://127.0.0.1:1".to_string()),
            admin_api_key: Some("test-admin-key".to_string()),
        };
        assert!(
            args.client(youtube_search_client::DEFAULT_COLLECTION)
                .is_ok()
        );
        let Err(error) = args.client("other-collection") else {
            panic!("a collection Raw does not serve must be rejected");
        };
        assert!(error.to_string().contains("other-collection"), "{error}");
    }

    #[test]
    fn an_update_with_only_existing_chunks_keeps_retryable_gaps_queued() {
        let mut stats = IndexStats {
            existing_videos: 1,
            existing_chunks: 10,
            skipped_videos: 1,
            retryable_failures: 1,
            ..IndexStats::default()
        };
        assert!(matches!(
            completed_index_result("@creator", &stats),
            Err(CliError::RetryableGaps(_))
        ));
        stats.retryable_failures = 0;
        assert!(completed_index_result("@creator", &stats).is_ok());
    }

    // A new upload whose captions are not published yet must not fail the
    // daily catalog pass: every creator was attempted and the gap is retried
    // on its own backoff.
    #[test]
    fn catalog_with_only_retryable_gaps_completes_with_gaps() {
        assert!(catalog_outcome(Vec::new()).is_ok());
        let gaps = vec![
            (
                "@one".to_string(),
                CliError::RetryableGaps("1 gap".to_string()),
            ),
            (
                "@two".to_string(),
                CliError::RetryableGaps("2 gaps".to_string()),
            ),
        ];
        assert!(matches!(
            catalog_outcome(gaps),
            Err(CliError::RetryableGaps(message)) if message.contains("@one") && message.contains("@two")
        ));
    }

    #[test]
    fn catalog_with_any_hard_failure_fails() {
        let mixed = vec![
            (
                "@one".to_string(),
                CliError::RetryableGaps("1 gap".to_string()),
            ),
            (
                "@two".to_string(),
                CliError::Message("reader unavailable".to_string()),
            ),
        ];
        assert!(matches!(catalog_outcome(mixed), Err(CliError::Message(_))));
    }

    #[test]
    fn backend_unavailability_exits_retryably_without_a_provider_cooldown_message() {
        let error = CliError::IndexStore(IndexStoreError::unavailable(
            "raw",
            "channel stats request timed out",
        ));
        assert_eq!(cli_error_exit_code(&error), EXIT_BACKEND_UNAVAILABLE);
        assert!(
            error
                .to_string()
                .contains("channel stats request timed out")
        );
        assert!(!error.to_string().contains("provider cooldown"));
        assert_eq!(
            cli_error_exit_code(&CliError::RetryableGaps("caption gap".to_string())),
            EXIT_RETRYABLE_GAPS,
        );
        assert_eq!(
            cli_error_exit_code(&CliError::UpstreamBlocked("rate_limited")),
            EXIT_UPSTREAM_BLOCKED,
        );
    }

    #[test]
    fn catalog_preserves_backend_unavailability_as_a_retryable_outcome() {
        for include_gap in [false, true] {
            let mut failures = vec![(
                "@offline".to_string(),
                CliError::IndexStore(IndexStoreError::unavailable(
                    "raw",
                    "channel stats request timed out",
                )),
            )];
            if include_gap {
                failures.push((
                    "@gap".to_string(),
                    CliError::RetryableGaps("caption gap".to_string()),
                ));
            }
            let error = catalog_outcome(failures).unwrap_err();
            assert_eq!(cli_error_exit_code(&error), EXIT_BACKEND_UNAVAILABLE);
            assert!(error.to_string().contains("@offline"));
            assert!(
                error
                    .to_string()
                    .contains("channel stats request timed out")
            );
            assert!(!error.to_string().contains("provider cooldown"));
            if include_gap {
                assert!(error.to_string().contains("@gap"));
            }
        }
    }

    #[test]
    fn backend_errors_other_than_unavailable_remain_fatal_in_a_mixed_catalog() {
        for fatal in [
            IndexStoreError::InvalidInput("invalid collection".to_string()),
            IndexStoreError::Conflict("operation conflict".to_string()),
            IndexStoreError::internal("raw", "unauthorized"),
            IndexStoreError::internal("raw", "invalid response"),
        ] {
            let error = CliError::IndexStore(fatal);
            assert_eq!(cli_error_exit_code(&error), 1);
            let result = catalog_outcome(vec![
                ("@fatal".to_string(), error),
                (
                    "@gap".to_string(),
                    CliError::RetryableGaps("caption gap".to_string()),
                ),
                (
                    "@offline".to_string(),
                    CliError::IndexStore(IndexStoreError::unavailable("raw", "timeout")),
                ),
            ]);
            assert!(matches!(result, Err(CliError::Message(_))));
            assert_eq!(cli_error_exit_code(&result.unwrap_err()), 1);
        }
    }

    #[test]
    fn gaps_and_upstream_blocks_use_distinct_exit_codes() {
        assert_ne!(EXIT_RETRYABLE_GAPS, EXIT_UPSTREAM_BLOCKED);
        assert_ne!(EXIT_BACKEND_UNAVAILABLE, EXIT_UPSTREAM_BLOCKED);
        assert_ne!(EXIT_BACKEND_UNAVAILABLE, EXIT_RETRYABLE_GAPS);
        assert_eq!(EXIT_RETRYABLE_GAPS, 75);
    }

    #[tokio::test]
    async fn publication_activation_gate_validates_and_unblocks() {
        assert!(
            PublicationActivationGate::from_paths([None, None, None, None])
                .unwrap()
                .is_none()
        );
        assert!(
            PublicationActivationGate::from_paths([Some(PathBuf::from("ready")), None, None, None])
                .is_err()
        );

        let directory = tempfile::tempdir().unwrap();
        let gate = PublicationActivationGate::from_paths([
            Some(directory.path().join("ready")),
            Some(directory.path().join("continue")),
            Some(directory.path().join("started")),
            Some(directory.path().join("finished")),
        ])
        .unwrap()
        .unwrap();
        assert!(gate.wait(Duration::from_millis(20)).await.is_err());
        fs::write(&gate.proceed, b"continue\n").unwrap();
        gate.wait(Duration::from_secs(1)).await.unwrap();
        gate.record_started().unwrap();
        gate.record_finished().unwrap();
        assert!(gate.ready.is_file());
        assert!(gate.started.is_file());
        assert!(gate.finished.is_file());
    }

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
    fn creator_concurrency_respects_the_global_video_worker_budget() {
        assert_eq!(effective_channel_jobs(0, 4), 1);
        assert_eq!(effective_channel_jobs(2, 4), 2);
        assert_eq!(effective_channel_jobs(4, 4), 4);
        assert_eq!(effective_channel_jobs(4, 8), 2);
        assert_eq!(effective_channel_jobs(4, 16), 1);
        assert_eq!(effective_channel_jobs(99, 1), 4);
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
    fn raw_publication_timeout_covers_a_full_production_generation() {
        assert_eq!(
            raw_publication_timeout(None).unwrap(),
            Duration::from_secs(45 * 60)
        );
        assert_eq!(
            raw_publication_timeout(Some("3600")).unwrap(),
            Duration::from_secs(3600)
        );
        assert!(raw_publication_timeout(Some("59")).is_err());
        assert!(raw_publication_timeout(Some("7201")).is_err());
        assert!(raw_publication_timeout(Some("forever")).is_err());
    }

    #[test]
    fn publication_workspace_and_free_space_inputs_fail_closed() {
        assert!(validate_publication_work_root(Path::new("/")).is_err());
        assert!(validate_publication_work_root(Path::new("relative")).is_err());
        assert!(validate_publication_work_root(Path::new("/var/lib/ytsearch/work")).is_ok());
        assert_eq!(
            publication_minimum_free_bytes(None).unwrap(),
            DEFAULT_RAW_PUBLICATION_MIN_FREE_BYTES
        );
        assert_eq!(
            publication_minimum_free_bytes(Some("9000000000")).unwrap(),
            9_000_000_000
        );
        assert!(publication_minimum_free_bytes(Some("8gb")).is_err());
        assert!(is_publication_job_id(
            "00000000000000000001-0123456789abcdef"
        ));
        assert!(!is_publication_job_id(
            "../00000000000000000001-0123456789abcdef"
        ));
    }

    #[test]
    fn remote_publication_requires_a_matching_reader_protocol() {
        let mut readiness = BackendReadiness {
            backend: "raw".to_string(),
            ready: true,
            generation: Some("1".to_string()),
            last_sequence: Some(1),
            acknowledged_readers: 1,
            publication_api_version: None,
            source_commit: None,
            generation_format: None,
        };

        let error = require_remote_publication_api(&readiness).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Raw reader publication API is incompatible: expected version 1, got missing; deploy a matching Raw reader before indexing"
        );

        readiness.publication_api_version = Some(RAW_PUBLICATION_API_VERSION);
        require_remote_publication_api(&readiness).unwrap();

        // The 2026-09-27 outage: the reader expected schema-8-v2 while the
        // worker built schema-7-v2, found only after an hour-long build.
        readiness.generation_format = Some("raw-index-schema-8-v2".to_string());
        let error = require_remote_publication_api(&readiness).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("deploy matching reader and worker images")
        );
        readiness.generation_format = Some(youtube_raw_search::RAW_GENERATION_FORMAT.to_string());
        require_remote_publication_api(&readiness).unwrap();
    }

    #[test]
    fn transcription_cli_requires_a_model_and_positive_bounds() {
        let base = [
            "ytsearch",
            "index-youtuber",
            "@creator",
            "--transcribe-missing",
            "--transcription-backend",
            "whisper-cpp",
        ];
        assert!(Cli::try_parse_from(base).is_err());
        let mut configured = base.to_vec();
        configured.extend([
            "--whisper-model",
            "/tmp/base.en.bin",
            "--whisper-threads",
            "2",
            "--transcription-budget-seconds",
            "480",
        ]);
        let parsed = Cli::try_parse_from(configured.clone()).unwrap();
        let Command::IndexYoutuber(command) = parsed.command else {
            panic!("wrong command")
        };
        assert_eq!(
            command.run.transcript.transcription_backend,
            TranscriptionBackendArg::WhisperCpp
        );
        assert_eq!(command.run.transcript.transcription_budget_seconds, 480);
        configured.extend(["--transcription-budget-seconds", "0"]);
        assert!(Cli::try_parse_from(configured).is_err());
        assert!(
            Cli::try_parse_from([
                "ytsearch",
                "index-youtuber",
                "@creator",
                "--whisper-threads",
                "0"
            ])
            .is_err()
        );
    }

    #[test]
    fn validated_portable_cache_reopens_only_its_captionless_video() {
        let root = std::env::temp_dir().join(format!(
            "ytsearch-portable-recovery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ledger = FailureLedger::new(root.join(".failures"), Duration::from_secs(3_600));
        for id in ["ready", "corrupt", "empty", "later"] {
            ledger.record(
                id,
                "creator",
                FailureStage::Transcript,
                "no usable captions",
            );
        }
        fs::write(
            root.join("ready.json"),
            br#"[{"start_ms":0,"end_ms":1000,"text":"working portable transcript"}]"#,
        )
        .unwrap();
        fs::write(root.join("corrupt.json"), b"{").unwrap();
        fs::write(root.join("empty.json"), b"[]").unwrap();
        let source = YtDlpTranscriptSource::new().with_cache_dir(&root);
        let mut videos = ["ready", "corrupt", "empty", "later"]
            .into_iter()
            .map(video)
            .collect::<Vec<_>>();
        let mut stats = IndexStats::default();
        defer_recent_transcript_failures(
            &mut videos,
            &ledger,
            "creator",
            &mut stats,
            Some(&source),
            false,
        );
        assert_eq!(
            videos
                .iter()
                .map(|video| video.video_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ready"]
        );
        assert_eq!(stats.deferred_transcript_failures, 3);
        assert_eq!(ledger.report().unwrap().total_records, 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recent_transcript_failures_do_not_block_later_missing_videos() {
        let failure_dir = std::env::temp_dir().join(format!(
            "ytsearch-transcript-failure-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&failure_dir);
        let ledger = FailureLedger::new(failure_dir.clone(), Duration::from_secs(3_600));
        ledger.record(
            "failed-video",
            "creator",
            FailureStage::Transcript,
            "temporary request failed",
        );
        let mut videos = vec![video("failed-video"), video("next-video")];
        let mut stats = IndexStats::default();

        defer_recent_transcript_failures(&mut videos, &ledger, "creator", &mut stats, None, false);

        assert_eq!(
            videos
                .iter()
                .map(|video| video.video_id.as_str())
                .collect::<Vec<_>>(),
            vec!["next-video"]
        );
        assert_eq!(stats.deferred_transcript_failures, 1);
        assert_eq!(stats.deferred_failure_categories.get("unknown"), Some(&1));
        ledger.clear("failed-video");
        let _ = std::fs::remove_dir_all(&failure_dir);
    }

    #[test]
    fn upstream_protection_outcomes_stop_new_video_work() {
        let rate_limited = IndexVideoOutcome::Skipped {
            video_id: "video".to_string(),
            title: "Video".to_string(),
            reason: "rate limited".to_string(),
            category: FailureCategory::RateLimited,
        };
        let captionless = IndexVideoOutcome::Skipped {
            video_id: "video".to_string(),
            title: "Video".to_string(),
            reason: "no captions".to_string(),
            category: FailureCategory::NoCaptions,
        };

        assert_eq!(
            rate_limited.upstream_blocking_category(),
            Some(FailureCategory::RateLimited)
        );
        assert_eq!(captionless.upstream_blocking_category(), None);
    }

    #[test]
    fn mixed_success_retains_retryable_gaps_but_accepts_terminal_gaps() {
        let mut stats = IndexStats::default();
        apply_index_outcome(
            &mut stats,
            IndexVideoOutcome::Indexed {
                video_id: "indexed".to_string(),
                title: "Indexed".to_string(),
                chunks: 2,
                audio_transcribed: false,
            },
            1,
            3,
        );
        apply_index_outcome(
            &mut stats,
            IndexVideoOutcome::Skipped {
                video_id: "captionless".to_string(),
                title: "Captionless".to_string(),
                reason: "no captions".to_string(),
                category: FailureCategory::NoCaptions,
            },
            2,
            3,
        );
        assert_eq!(stats.retryable_failures, 0);

        apply_index_outcome(
            &mut stats,
            IndexVideoOutcome::Skipped {
                video_id: "timed-out".to_string(),
                title: "Timed out".to_string(),
                reason: "timeout".to_string(),
                category: FailureCategory::Timeout,
            },
            3,
            3,
        );
        assert_eq!(stats.indexed_videos, 1);
        assert_eq!(stats.retryable_failures, 1);
    }

    #[test]
    fn canonical_youtuber_id_does_not_include_handle_prefix() {
        assert_eq!(
            youtuber_id_from_handle("@t3dotgg", "UCbRP3c757lWg9M-U7TyEkXA"),
            "t3dotgg"
        );
    }

    #[test]
    fn existing_data_flags_resolve_to_an_explicit_policy() {
        let update = ExistingDataPolicy {
            channel: ExistingChannelPolicy::Retain,
            videos: ExistingVideoPolicy::Skip,
        };
        let cases = [
            (IndexMode::Update, false, false, update),
            (IndexMode::Update, false, true, update),
            (IndexMode::Update, true, false, update),
            (IndexMode::Update, true, true, update),
            (
                IndexMode::Rewipe,
                false,
                false,
                ExistingDataPolicy {
                    channel: ExistingChannelPolicy::Rewipe,
                    videos: ExistingVideoPolicy::Reindex,
                },
            ),
            (
                IndexMode::Rewipe,
                false,
                true,
                ExistingDataPolicy {
                    channel: ExistingChannelPolicy::Rewipe,
                    videos: ExistingVideoPolicy::Skip,
                },
            ),
            (
                IndexMode::Rewipe,
                true,
                false,
                ExistingDataPolicy {
                    channel: ExistingChannelPolicy::Retain,
                    videos: ExistingVideoPolicy::Reindex,
                },
            ),
            (
                IndexMode::Rewipe,
                true,
                true,
                ExistingDataPolicy {
                    channel: ExistingChannelPolicy::Retain,
                    videos: ExistingVideoPolicy::Skip,
                },
            ),
        ];
        for (mode, no_wipe, skip_existing, expected) in cases {
            assert_eq!(
                ExistingDataPolicy::from_cli(mode, no_wipe, skip_existing),
                expected
            );
        }
    }

    #[test]
    fn destructive_or_contradictory_existing_data_policy_fails_closed() {
        let rewipe = ExistingDataPolicy::from_cli(IndexMode::Rewipe, false, false);
        assert!(validate_existing_data_policy(rewipe, true).is_ok());
        assert!(validate_existing_data_policy(rewipe, false).is_err());

        let contradictory = ExistingDataPolicy::from_cli(IndexMode::Rewipe, false, true);
        assert!(validate_existing_data_policy(contradictory, true).is_err());

        let retain = ExistingDataPolicy::from_cli(IndexMode::Rewipe, true, false);
        assert!(validate_existing_data_policy(retain, false).is_ok());
    }

    #[test]
    fn deferred_visibility_maps_to_an_explicit_publication_policy() {
        assert_eq!(
            PublicationPolicy::from_defer_flag(false),
            PublicationPolicy::PublishAfterRun
        );
        assert!(PublicationPolicy::PublishAfterRun.publishes_after_run());
        assert_eq!(
            PublicationPolicy::from_defer_flag(true),
            PublicationPolicy::DeferVisibility
        );
        assert!(!PublicationPolicy::DeferVisibility.publishes_after_run());
    }

    #[test]
    fn destructive_remove_requires_explicit_acknowledgement() {
        assert!(validate_remove_safety(true).is_ok());
        assert!(validate_remove_safety(false).is_err());
    }

    #[test]
    fn mutation_keys_are_stable_for_retries_and_change_with_content() {
        let first = content_operation_key("replace-video", "video", b"first");
        assert_eq!(
            first,
            content_operation_key("replace-video", "video", b"first")
        );
        assert_ne!(
            first,
            content_operation_key("replace-video", "video", b"second")
        );
        assert!(first.bytes().all(|byte| byte.is_ascii_graphic()));
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
