use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex as BudgetMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};
use tokio::{fs, process::Command, sync::Mutex, time::timeout};
use youtube_ingest::{IngestError, TimestampMs, TranscriptSegment};
use youtube_ytdlp::YtDlpTranscriptSource;

const MAX_AUDIO_BYTES: u64 = 1_500_000_000;
const MAX_DURATION_MS: u32 = 12 * 60 * 60 * 1000;
const CHUNK_TIMEOUT: Duration = Duration::from_secs(120);
static CACHE_WRITES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TranscriptionBackend {
    Voxtype,
    WhisperCpp {
        binary: PathBuf,
        model: PathBuf,
        threads: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionConfig {
    cache_dir: PathBuf,
    caption_cache_dir: PathBuf,
    chunk_seconds: u32,
    ffmpeg_binary: String,
    ffprobe_binary: String,
    voxtype_binary: String,
    voxtype_engine: Option<String>,
    backend: TranscriptionBackend,
    budget: Duration,
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            cache_dir: PathBuf::from("data/transcription-cache"),
            caption_cache_dir: PathBuf::from("data/transcript-cache"),
            chunk_seconds: 30,
            ffmpeg_binary: "ffmpeg".into(),
            ffprobe_binary: "ffprobe".into(),
            voxtype_binary: "voxtype".into(),
            voxtype_engine: None,
            backend: TranscriptionBackend::Voxtype,
            budget: Duration::from_secs(480),
        }
    }
}

impl TranscriptionConfig {
    pub fn with_cache_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.cache_dir = directory.into();
        self
    }
    pub fn with_caption_cache_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.caption_cache_dir = directory.into();
        self
    }
    pub fn with_chunk_seconds(mut self, seconds: u32) -> Self {
        self.chunk_seconds = seconds.clamp(1, 300);
        self
    }
    pub fn with_voxtype_engine(mut self, engine: Option<impl Into<String>>) -> Self {
        self.voxtype_engine = engine.map(Into::into);
        self
    }
    pub fn with_backend(mut self, backend: TranscriptionBackend) -> Self {
        self.backend = backend;
        self
    }
    pub fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }
}

#[derive(Debug)]
pub enum AudioTranscriptionError {
    Deferred,
    Failed(IngestError),
}
impl fmt::Display for AudioTranscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deferred => {
                formatter.write_str("audio transcription budget exhausted; resume on a later pass")
            }
            Self::Failed(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for AudioTranscriptionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Failed(error) => Some(error),
            Self::Deferred => None,
        }
    }
}
impl From<IngestError> for AudioTranscriptionError {
    fn from(error: IngestError) -> Self {
        Self::Failed(error)
    }
}

/// Source clones share one serialized inference slot and one active-work budget.
#[derive(Clone, Debug)]
pub struct AudioTranscriptSource {
    config: TranscriptionConfig,
    downloader: YtDlpTranscriptSource,
    slot: Arc<Mutex<()>>,
    remaining: Arc<BudgetMutex<Duration>>,
}

struct ActiveBudget {
    remaining: Arc<BudgetMutex<Duration>>,
    started: Instant,
    available: Duration,
}
impl ActiveBudget {
    fn remaining(&self) -> Duration {
        self.available.saturating_sub(self.started.elapsed())
    }
}
impl Drop for ActiveBudget {
    fn drop(&mut self) {
        let mut remaining = self
            .remaining
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *remaining = remaining.saturating_sub(self.started.elapsed());
    }
}

#[derive(Serialize, Deserialize)]
struct ChunkCheckpoint {
    identity: String,
    start_ms: TimestampMs,
    end_ms: TimestampMs,
    segments: Vec<TranscriptSegment>,
}

impl AudioTranscriptSource {
    pub fn with_config(config: TranscriptionConfig) -> Self {
        Self {
            downloader: YtDlpTranscriptSource::new()
                .with_cache_dir(config.caption_cache_dir.clone()),
            remaining: Arc::new(BudgetMutex::new(config.budget)),
            slot: Arc::new(Mutex::new(())),
            config,
        }
    }

    /// Fail before claiming work when an explicitly enabled runtime is unusable.
    pub async fn preflight(&self) -> youtube_ingest::Result<()> {
        fs::create_dir_all(&self.config.cache_dir)
            .await
            .map_err(|error| failed("<setup>", error))?;
        fs::create_dir_all(&self.config.caption_cache_dir)
            .await
            .map_err(|error| failed("<setup>", error))?;
        for directory in [&self.config.cache_dir, &self.config.caption_cache_dir] {
            let probe = directory.join(format!(
                ".asr-preflight-{}-{}",
                std::process::id(),
                CACHE_WRITES.fetch_add(1, Ordering::Relaxed)
            ));
            fs::write(&probe, b"")
                .await
                .map_err(|error| failed("<setup>", error))?;
            fs::remove_file(probe)
                .await
                .map_err(|error| failed("<setup>", error))?;
        }
        for binary in [&self.config.ffmpeg_binary, &self.config.ffprobe_binary] {
            check_binary(Path::new(binary), "-version").await?;
        }
        match &self.config.backend {
            TranscriptionBackend::Voxtype => {
                check_binary(Path::new(&self.config.voxtype_binary), "--help").await?
            }
            TranscriptionBackend::WhisperCpp {
                binary,
                model,
                threads,
            } => {
                if *threads == 0 {
                    return Err(failed("<setup>", "whisper threads must be positive"));
                }
                let metadata = fs::metadata(model).await.map_err(|error| {
                    failed("<setup>", format!("whisper model is not readable: {error}"))
                })?;
                if !metadata.is_file() || metadata.len() == 0 {
                    return Err(failed("<setup>", "whisper model is empty or not a file"));
                }
                fs::File::open(model)
                    .await
                    .map_err(|error| failed("<setup>", error))?;
                check_binary(binary, "--help").await?;
            }
        }
        Ok(())
    }

    pub async fn transcribe(
        &self,
        video_id: &str,
    ) -> Result<Vec<TranscriptSegment>, AudioTranscriptionError> {
        if let Some(segments) = self.downloader.cached_transcript(video_id)? {
            return Ok(segments);
        }
        let _slot = self.slot.lock().await;
        if let Some(segments) = self.downloader.cached_transcript(video_id)? {
            return Ok(segments);
        }
        let available = *self
            .remaining
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if available.is_zero() {
            return Err(AudioTranscriptionError::Deferred);
        }
        let budget = ActiveBudget {
            remaining: self.remaining.clone(),
            started: Instant::now(),
            available,
        };
        match timeout(available, self.transcribe_with_budget(video_id, &budget)).await {
            Ok(Err(_)) if budget.remaining().is_zero() => Err(AudioTranscriptionError::Deferred),
            Ok(result) => result,
            Err(_) => Err(AudioTranscriptionError::Deferred),
        }
    }

    async fn transcribe_with_budget(
        &self,
        video_id: &str,
        budget: &ActiveBudget,
    ) -> Result<Vec<TranscriptSegment>, AudioTranscriptionError> {
        let directory = self.config.cache_dir.join(safe_file_stem(video_id));
        fs::create_dir_all(&directory)
            .await
            .map_err(|error| failed(video_id, error))?;
        let audio = directory.join("audio.wav");
        if fs::metadata(&audio).await.is_err() {
            let available_bytes = MAX_AUDIO_BYTES.saturating_sub(
                audio_cache_bytes(&self.config.cache_dir)
                    .map_err(|error| failed(video_id, error))?,
            );
            if available_bytes <= 44 {
                return Err(failed(
                    video_id,
                    "transcription audio cache is full; resume existing videos before downloading more",
                ).into());
            }
            self.downloader
                .download_audio(
                    video_id,
                    &audio,
                    budget.remaining().min(Duration::from_secs(180)),
                    available_bytes,
                )
                .await?;
        }
        let metadata = fs::metadata(&audio)
            .await
            .map_err(|error| failed(video_id, error))?;
        if metadata.len() > MAX_AUDIO_BYTES || metadata.len() <= 44 {
            return Err(failed(video_id, "cached audio exceeds supported size or is empty").into());
        }
        let duration_ms = self.duration(video_id, &audio).await?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let model_identity = match &self.config.backend {
            TranscriptionBackend::WhisperCpp { model, .. } => {
                let metadata = fs::metadata(model)
                    .await
                    .map_err(|error| failed(video_id, error))?;
                format!("{}:{:?}", metadata.len(), metadata.modified().ok())
            }
            TranscriptionBackend::Voxtype => String::new(),
        };
        let identity = format!(
            "v1:{}:{}:{}:{}:{}",
            serde_json::to_string(&(
                &self.config.backend,
                self.config.chunk_seconds,
                &self.config.voxtype_engine
            ))
            .map_err(|error| failed(video_id, error))?,
            metadata.len(),
            modified,
            duration_ms,
            model_identity
        );
        let chunk_ms = self.config.chunk_seconds * 1000;
        let mut start_ms = 0;
        let mut transcript = Vec::new();
        while start_ms < duration_ms {
            if budget.remaining().is_zero() {
                return Err(AudioTranscriptionError::Deferred);
            }
            let end_ms = start_ms.saturating_add(chunk_ms).min(duration_ms);
            let checkpoint_path = directory.join(format!("{start_ms}.json"));
            let cached = fs::read(&checkpoint_path)
                .await
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ChunkCheckpoint>(&bytes).ok())
                .filter(|checkpoint| {
                    checkpoint.identity == identity
                        && checkpoint.start_ms == start_ms
                        && checkpoint.end_ms == end_ms
                        && valid_chunk(&checkpoint.segments, start_ms, end_ms)
                });
            let segments = if let Some(checkpoint) = cached {
                checkpoint.segments
            } else {
                let chunk = directory.join("chunk.wav");
                self.extract_chunk(video_id, &audio, &chunk, start_ms, end_ms)
                    .await?;
                let segments = self
                    .transcribe_chunk(
                        video_id,
                        &chunk,
                        &directory.join("whisper-result"),
                        start_ms,
                        end_ms,
                        budget.remaining(),
                    )
                    .await?;
                let checkpoint = ChunkCheckpoint {
                    identity: identity.clone(),
                    start_ms,
                    end_ms,
                    segments: segments.clone(),
                };
                write_json_atomic(&checkpoint_path, &checkpoint)
                    .await
                    .map_err(|error| failed(video_id, error))?;
                let _ = fs::remove_file(chunk).await;
                segments
            };
            transcript.extend(segments);
            start_ms = end_ms;
        }
        if transcript.is_empty() {
            // Every chunk completed, so no work remains to resume. Keeping
            // terminal speech-free audio would consume the bounded cache.
            let _ = fs::remove_dir_all(directory).await;
            return Err(failed(video_id, "audio transcription produced no usable speech").into());
        }
        self.downloader
            .write_cached_transcript(video_id, &transcript)?;
        let _ = fs::remove_dir_all(directory).await;
        Ok(transcript)
    }

    async fn duration(&self, video_id: &str, audio: &Path) -> youtube_ingest::Result<u32> {
        let mut command = Command::new(&self.config.ffprobe_binary);
        command
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(audio);
        let output = run(video_id, &mut command).await?;
        let seconds = String::from_utf8_lossy(&output)
            .trim()
            .parse::<f64>()
            .map_err(|error| failed(video_id, error))?;
        let milliseconds = (seconds * 1000.0).ceil();
        if !milliseconds.is_finite() || !(1.0..=f64::from(MAX_DURATION_MS)).contains(&milliseconds)
        {
            return Err(failed(
                video_id,
                "audio duration exceeds supported 12-hour limit or is invalid",
            ));
        }
        Ok(milliseconds as u32)
    }

    async fn extract_chunk(
        &self,
        video_id: &str,
        audio: &Path,
        chunk: &Path,
        start: u32,
        end: u32,
    ) -> youtube_ingest::Result<()> {
        let mut command = Command::new(&self.config.ffmpeg_binary);
        command
            .arg("-y")
            .arg("-ss")
            .arg(format_seconds(start))
            .arg("-t")
            .arg(format_seconds(end - start))
            .arg("-i")
            .arg(audio)
            .args(["-vn", "-ac", "1", "-ar", "16000"])
            .arg(chunk);
        run(video_id, &mut command).await?;
        Ok(())
    }

    async fn transcribe_chunk(
        &self,
        video_id: &str,
        chunk: &Path,
        output: &Path,
        start_ms: u32,
        end_ms: u32,
        remaining: Duration,
    ) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
        match &self.config.backend {
            TranscriptionBackend::Voxtype => {
                let mut command = Command::new(&self.config.voxtype_binary);
                command.arg("transcribe");
                if let Some(engine) = &self.config.voxtype_engine {
                    command.arg("--engine").arg(engine);
                }
                command.arg(chunk);
                let bytes = run_with_timeout(video_id, &mut command, remaining).await?;
                let text = normalize_transcriber_output(&String::from_utf8_lossy(&bytes));
                Ok(if text.is_empty() {
                    Vec::new()
                } else {
                    vec![TranscriptSegment {
                        start_ms,
                        end_ms,
                        text,
                    }]
                })
            }
            TranscriptionBackend::WhisperCpp {
                binary,
                model,
                threads,
            } => {
                let output_file = output.with_extension("json");
                let _ = fs::remove_file(&output_file).await;
                let mut command = Command::new(binary);
                command
                    .arg("-m")
                    .arg(model)
                    .arg("-f")
                    .arg(chunk)
                    .args(["-l", "en", "-ng", "-oj", "-of"])
                    .arg(output)
                    .arg("-t")
                    .arg(threads.to_string());
                run_with_timeout(video_id, &mut command, remaining).await?;
                let bytes = fs::read(&output_file)
                    .await
                    .map_err(|error| failed(video_id, error))?;
                let segments = parse_whisper_output(video_id, &bytes, start_ms, end_ms)?;
                let _ = fs::remove_file(output_file).await;
                Ok(segments)
            }
        }
    }
}

// Bound incomplete audio across the invocation's shared work directory. Existing
// videos can still resume when the cap prevents starting another download.
fn audio_cache_bytes(directory: &Path) -> std::io::Result<u64> {
    let mut bytes = 0_u64;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            match std::fs::metadata(entry.path().join("audio.wav")) {
                Ok(metadata) => bytes = bytes.saturating_add(metadata.len()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(bytes)
}

async fn check_binary(binary: &Path, argument: &str) -> youtube_ingest::Result<()> {
    let mut command = Command::new(binary);
    command.arg(argument);
    run("<setup>", &mut command).await.map(|_| ())
}

async fn run(video_id: &str, command: &mut Command) -> youtube_ingest::Result<Vec<u8>> {
    run_with_timeout(video_id, command, CHUNK_TIMEOUT).await
}

async fn run_with_timeout(
    video_id: &str,
    command: &mut Command,
    remaining: Duration,
) -> youtube_ingest::Result<Vec<u8>> {
    let output = timeout(
        remaining,
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| failed(video_id, "audio subprocess timed out"))?
    .map_err(|error| failed(video_id, error))?;
    if !output.status.success() {
        return Err(failed(
            video_id,
            format!(
                "audio subprocess failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(output.stdout)
}

#[derive(Deserialize)]
struct WhisperOutput {
    transcription: Vec<WhisperSegment>,
}
#[derive(Deserialize)]
struct WhisperSegment {
    offsets: WhisperOffsets,
    text: String,
}
#[derive(Deserialize)]
struct WhisperOffsets {
    from: u32,
    to: u32,
}

fn parse_whisper_output(
    video_id: &str,
    bytes: &[u8],
    start_ms: u32,
    end_ms: u32,
) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
    let output: WhisperOutput =
        serde_json::from_slice(bytes).map_err(|error| failed(video_id, error))?;
    let mut segments = Vec::new();
    for segment in output.transcription {
        let text = segment
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() || text.eq_ignore_ascii_case("[BLANK_AUDIO]") {
            continue;
        }
        let duration = end_ms - start_ms;
        // Whisper may extend its final decoded segment beyond the supplied
        // audio (observed 29.600..30.600 on a 30s chunk). Keep its intersection
        // with real audio; padding alone must never become searchable text.
        if segment.offsets.from >= segment.offsets.to {
            return Err(failed(video_id, "whisper returned reversed timestamps"));
        }
        if segment.offsets.from >= duration {
            continue;
        }
        segments.push(TranscriptSegment {
            start_ms: start_ms + segment.offsets.from,
            end_ms: start_ms + segment.offsets.to.min(duration),
            text,
        });
    }
    if !valid_chunk(&segments, start_ms, end_ms) {
        return Err(failed(
            video_id,
            "whisper returned unordered transcript segments",
        ));
    }
    Ok(segments)
}

fn valid_chunk(segments: &[TranscriptSegment], start: u32, end: u32) -> bool {
    segments.iter().all(|segment| {
        segment.start_ms >= start
            && segment.start_ms < segment.end_ms
            && segment.end_ms <= end
            && !segment.text.trim().is_empty()
    }) && segments
        .windows(2)
        .all(|pair| pair[0].start_ms <= pair[1].start_ms)
}

async fn write_json_atomic(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let temporary = path.with_extension(format!(
        "json.tmp.{}.{}",
        std::process::id(),
        CACHE_WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    fs::write(&temporary, bytes).await?;
    if let Err(error) = fs::rename(&temporary, path).await {
        let _ = fs::remove_file(temporary).await;
        return Err(error);
    }
    Ok(())
}
fn failed(video_id: &str, error: impl fmt::Display) -> IngestError {
    IngestError::TranscriptUnavailable {
        video_id: video_id.into(),
        reason: error.to_string(),
    }
}
fn safe_file_stem(value: &str) -> String {
    value
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || value == '-' || value == '_' {
                value
            } else {
                '_'
            }
        })
        .collect()
}
fn format_seconds(milliseconds: u32) -> String {
    format!("{:.3}", f64::from(milliseconds) / 1000.0)
}
fn normalize_transcriber_output(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && ![
                    "Loading audio file:",
                    "Audio format:",
                    "Processing ",
                    "whisper_",
                    "ggml_",
                    "system_info:",
                    "main:",
                    "202",
                    "CPU total size",
                    "model size",
                    "n_vocab",
                    "n_audio_",
                    "n_text_",
                    "n_mels",
                    "ftype",
                    "type",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
        })
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ytsearch-asr-{}-{}",
                std::process::id(),
                CACHE_WRITES.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn script(&self, name: &str, body: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
        fn config(&self) -> TranscriptionConfig {
            let probe = self.script("ffprobe", "echo 70");
            let ffmpeg = self.script(
                "ffmpeg",
                "[ \"$1\" = -version ] && exit 0\nfor last do :; done\nprintf wav > \"$last\"",
            );
            let mut config = TranscriptionConfig::default()
                .with_cache_dir(self.0.join("work"))
                .with_caption_cache_dir(self.0.join("cache"));
            config.ffprobe_binary = probe.to_string_lossy().into_owned();
            config.ffmpeg_binary = ffmpeg.to_string_lossy().into_owned();
            config
        }
        fn audio(&self, video: &str) {
            let directory = self.0.join("work").join(video);
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("audio.wav"), [0; 100]).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn whisper_json_offsets_are_relative_and_invalid_output_is_rejected() {
        let result = parse_whisper_output(
            "id",
            br#"{"transcription":[{"offsets":{"from":50,"to":150},"text":" hello world "}]}"#,
            30_000,
            60_000,
        )
        .unwrap();
        assert_eq!(result[0].start_ms, 30_050);
        assert_eq!(result[0].end_ms, 30_150);
        assert_eq!(result[0].text, "hello world");
        assert!(
            parse_whisper_output(
                "id",
                br#"{"transcription":[{"offsets":{"from":100,"to":10},"text":"bad"}]}"#,
                0,
                30_000
            )
            .is_err()
        );
        let boundary = parse_whisper_output("id", br#"{"transcription":[{"offsets":{"from":29600,"to":30600},"text":" collect the data"},{"offsets":{"from":30600,"to":31000},"text":"padding only"}]}"#, 30_000, 60_000).unwrap();
        assert_eq!(boundary.len(), 1);
        assert_eq!((boundary[0].start_ms, boundary[0].end_ms), (59_600, 60_000));
        assert!(parse_whisper_output("id", b"runtime logs", 0, 30_000).is_err());
        assert!(
            parse_whisper_output("id", br#"{"transcription":[]}"#, 0, 30_000)
                .unwrap()
                .is_empty()
        );
        assert!(parse_whisper_output("id", br#"{"transcription":[{"offsets":{"from":0,"to":30000},"text":" [BLANK_AUDIO] "}]}"#, 0, 30_000).unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_inference_resumes_completed_silence_and_commits_only_full_transcript() {
        let fixture = Fixture::new();
        fixture.audio("video");
        let count = fixture.0.join("count");
        let fail = fixture.0.join("fail");
        std::fs::write(&fail, "yes").unwrap();
        let binary = fixture.script(
            "voxtype",
            &format!(
                r#"
count=$(cat '{}' 2>/dev/null || echo 0)
count=$((count + 1))
printf '%s' "$count" > '{}'
if [ "$count" -eq 1 ]; then exit 0; fi
if [ -f '{}' ]; then echo 'inference failed' >&2; exit 1; fi
echo 'recovered useful speech'
"#,
                count.display(),
                count.display(),
                fail.display()
            ),
        );
        let mut config = fixture.config();
        config.voxtype_binary = binary.to_string_lossy().into_owned();
        let source = AudioTranscriptSource::with_config(config.clone());
        assert!(matches!(
            source.transcribe("video").await,
            Err(AudioTranscriptionError::Failed(_))
        ));
        assert!(!fixture.0.join("cache/video.json").exists());
        assert!(fixture.0.join("work/video/0.json").exists());
        std::fs::remove_file(fail).unwrap();
        let resumed = AudioTranscriptSource::with_config(config);
        let transcript = resumed.transcribe("video").await.unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].start_ms, 30_000);
        assert_eq!(std::fs::read_to_string(count).unwrap(), "4");
        assert!(fixture.0.join("cache/video.json").exists());
        assert!(!fixture.0.join("work/video").exists());
    }

    #[tokio::test]
    async fn completed_speech_free_audio_releases_the_bounded_work_cache() {
        let fixture = Fixture::new();
        fixture.audio("silent");
        let mut config = fixture.config();
        config.voxtype_binary = fixture.script("voxtype", "exit 0").to_string_lossy().into();
        let source = AudioTranscriptSource::with_config(config);
        let error = source.transcribe("silent").await.unwrap_err();
        assert!(error.to_string().contains("no usable speech"));
        assert!(!fixture.0.join("cache/silent.json").exists());
        assert!(!fixture.0.join("work/silent").exists());
        assert_eq!(audio_cache_bytes(&fixture.0.join("work")).unwrap(), 0);
    }

    #[tokio::test]
    async fn slow_inference_uses_the_remaining_invocation_budget() {
        let fixture = Fixture::new();
        fixture.audio("slow");
        let started = fixture.0.join("started");
        let release = fixture.0.join("release");
        let mut config = fixture.config().with_budget(Duration::from_secs(480));
        config.voxtype_binary = fixture
            .script(
                "voxtype",
                &format!(
                    "touch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\necho 'slow useful speech'",
                    started.display(),
                    release.display()
                ),
            )
            .to_string_lossy()
            .into();
        let source = AudioTranscriptSource::with_config(config);
        let task = tokio::spawn(async move { source.transcribe("slow").await });
        let started_wait = Instant::now();
        while !started.exists() {
            assert!(started_wait.elapsed() < Duration::from_secs(5));
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        tokio::time::pause();
        tokio::time::advance(CHUNK_TIMEOUT + Duration::from_secs(1)).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        let still_running = !task.is_finished();
        std::fs::write(release, "resume").unwrap();
        tokio::time::resume();
        let result = task.await.unwrap();
        assert!(
            still_running,
            "inference stopped at the helper subprocess deadline"
        );
        assert_eq!(result.unwrap().len(), 3);
        assert!(fixture.0.join("cache/slow.json").exists());
    }

    #[tokio::test]
    async fn clones_share_budget_and_cancellation_debits_active_work() {
        let fixture = Fixture::new();
        fixture.audio("video");
        let binary = fixture.script("voxtype", "exec sleep 10");
        let mut config = fixture.config().with_budget(Duration::from_millis(60));
        config.voxtype_binary = binary.to_string_lossy().into_owned();
        let source = AudioTranscriptSource::with_config(config);
        let other = source.clone();
        let task = tokio::spawn(async move { other.transcribe("video").await });
        tokio::time::sleep(Duration::from_millis(25)).await;
        task.abort();
        let _ = task.await;
        let remaining = *source.remaining.lock().unwrap();
        assert!(remaining < Duration::from_millis(50));
        assert!(matches!(
            source.transcribe("video").await,
            Err(AudioTranscriptionError::Deferred)
        ));
        assert!(matches!(
            source.clone().transcribe("another").await,
            Err(AudioTranscriptionError::Deferred)
        ));
        assert!(!fixture.0.join("cache/video.json").exists());
    }

    #[tokio::test]
    async fn portable_cache_is_read_without_runtime_or_budget_and_corrupt_entries_are_misses() {
        let fixture = Fixture::new();
        let mut config = fixture.config().with_budget(Duration::ZERO);
        config.voxtype_binary = "/missing/runtime".into();
        let source = AudioTranscriptSource::with_config(config);
        std::fs::create_dir_all(fixture.0.join("cache")).unwrap();
        std::fs::write(
            fixture.0.join("cache/bad.json"),
            r#"[{"start_ms":100,"end_ms":0,"text":"wrong"}]"#,
        )
        .unwrap();
        assert!(matches!(
            source.transcribe("bad").await,
            Err(AudioTranscriptionError::Deferred)
        ));
        source
            .downloader
            .write_cached_transcript(
                "good",
                &[TranscriptSegment {
                    start_ms: 0,
                    end_ms: 1000,
                    text: "cached speech".into(),
                }],
            )
            .unwrap();
        assert_eq!(
            source.transcribe("good").await.unwrap()[0].text,
            "cached speech"
        );
        assert!(source.preflight().await.is_err());
    }
}
