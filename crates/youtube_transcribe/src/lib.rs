use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    fs,
    process::Command,
    time::{Duration, timeout},
};
use youtube_ingest::{IngestError, TimestampMs, TranscriptSegment, TranscriptSource};

const YOUTUBE_WATCH_URL_PREFIX: &str = "https://www.youtube.com/watch?v=";
const DEFAULT_CACHE_DIR: &str = "data/transcription-cache";
const DEFAULT_YT_DLP_BINARY: &str = "yt-dlp";
const DEFAULT_FFMPEG_BINARY: &str = "ffmpeg";
const DEFAULT_FFPROBE_BINARY: &str = "ffprobe";
const DEFAULT_VOXTYPE_BINARY: &str = "voxtype";
const DEFAULT_CHUNK_SECONDS: u32 = 30;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(60 * 20);
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60 * 5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoxtypeTranscriptSource {
    config: TranscriptionConfig,
}

impl VoxtypeTranscriptSource {
    pub fn new() -> Self {
        Self::with_config(TranscriptionConfig::default())
    }

    pub fn with_config(config: TranscriptionConfig) -> Self {
        Self { config }
    }

    async fn cached_or_transcribe(
        &self,
        video_id: &str,
    ) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
        let transcript_path = self.config.transcript_path(video_id);
        if let Ok(bytes) = fs::read(&transcript_path).await {
            tracing::info!(video_id, path = %transcript_path.display(), "using cached transcript");
            return serde_json::from_slice(&bytes)
                .map_err(|error| transcript_error(video_id, error));
        }

        self.config.ensure_directories().await?;
        let audio_path = self.config.audio_path(video_id);
        if fs::metadata(&audio_path).await.is_err() {
            tracing::info!(video_id, path = %audio_path.display(), "extracting audio");
            self.extract_audio(video_id, &audio_path).await?;
        } else {
            tracing::info!(video_id, path = %audio_path.display(), "using cached audio");
        }

        let duration_ms = self.probe_duration_ms(video_id, &audio_path).await?;
        let segments = self
            .transcribe_audio_chunks(video_id, &audio_path, duration_ms)
            .await?;
        let bytes = serde_json::to_vec_pretty(&segments)
            .map_err(|error| transcript_error(video_id, error))?;
        tracing::info!(video_id, segments = segments.len(), path = %transcript_path.display(), "writing transcript cache");
        fs::write(transcript_path, bytes)
            .await
            .map_err(|error| transcript_error(video_id, error))?;

        Ok(segments)
    }

    async fn extract_audio(&self, video_id: &str, audio_path: &Path) -> youtube_ingest::Result<()> {
        let output_template = self.config.audio_output_template(video_id);
        let output = timeout(
            PROCESS_TIMEOUT,
            Command::new(&self.config.yt_dlp_binary)
                .kill_on_drop(true)
                .arg("--no-warnings")
                .arg("--extract-audio")
                .arg("--audio-format")
                .arg("wav")
                .arg("--postprocessor-args")
                .arg("ffmpeg:-ac 1 -ar 16000")
                .arg("-o")
                .arg(output_template)
                .arg(video_url(video_id))
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| transcript_error(video_id, "yt-dlp audio extraction timed out"))?
        .map_err(|error| transcript_error(video_id, error))?;

        if !output.status.success() {
            return Err(transcript_error(
                video_id,
                format!(
                    "yt-dlp audio extraction failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        if fs::metadata(audio_path).await.is_err() {
            return Err(transcript_error(
                video_id,
                format!(
                    "yt-dlp did not create expected audio file {}",
                    audio_path.display()
                ),
            ));
        }

        Ok(())
    }

    async fn probe_duration_ms(
        &self,
        video_id: &str,
        audio_path: &Path,
    ) -> youtube_ingest::Result<TimestampMs> {
        let output = timeout(
            CHUNK_TIMEOUT,
            Command::new(&self.config.ffprobe_binary)
                .kill_on_drop(true)
                .arg("-v")
                .arg("error")
                .arg("-show_entries")
                .arg("format=duration")
                .arg("-of")
                .arg("default=noprint_wrappers=1:nokey=1")
                .arg(audio_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| transcript_error(video_id, "ffprobe timed out"))?
        .map_err(|error| transcript_error(video_id, error))?;

        if !output.status.success() {
            return Err(transcript_error(
                video_id,
                format!(
                    "ffprobe failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        let seconds = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .map_err(|error| transcript_error(video_id, error))?;
        let milliseconds = (seconds * 1000.0).ceil();
        if !(1.0..=f64::from(u32::MAX)).contains(&milliseconds) {
            return Err(transcript_error(
                video_id,
                "audio duration is out of supported range",
            ));
        }

        Ok(milliseconds as TimestampMs)
    }

    async fn transcribe_audio_chunks(
        &self,
        video_id: &str,
        audio_path: &Path,
        duration_ms: TimestampMs,
    ) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
        let chunk_ms = self.config.chunk_seconds.saturating_mul(1000).max(1000);
        let mut start_ms = 0;
        let mut segments = Vec::new();

        while start_ms < duration_ms {
            let end_ms = start_ms.saturating_add(chunk_ms).min(duration_ms);
            let chunk_path = self.config.chunk_path(video_id, start_ms);
            self.extract_chunk(video_id, audio_path, &chunk_path, start_ms, end_ms)
                .await?;

            tracing::debug!(video_id, start_ms, end_ms, path = %chunk_path.display(), "transcribing audio chunk");
            match self.transcribe_chunk(video_id, &chunk_path).await {
                Ok(text) if has_text(&text) => segments.push(TranscriptSegment {
                    start_ms,
                    end_ms,
                    text,
                }),
                Ok(_) => {}
                Err(error) => return Err(error),
            }

            let _ = fs::remove_file(chunk_path).await;
            start_ms = end_ms;
        }

        if segments.is_empty() {
            return Err(IngestError::TranscriptUnavailable {
                video_id: video_id.to_string(),
                reason: "voxtype produced no usable transcript text".to_string(),
            });
        }

        Ok(segments)
    }

    async fn extract_chunk(
        &self,
        video_id: &str,
        audio_path: &Path,
        chunk_path: &Path,
        start_ms: TimestampMs,
        end_ms: TimestampMs,
    ) -> youtube_ingest::Result<()> {
        let start_seconds = format_seconds(start_ms);
        let duration_seconds = format_seconds(end_ms.saturating_sub(start_ms));
        let output = timeout(
            CHUNK_TIMEOUT,
            Command::new(&self.config.ffmpeg_binary)
                .kill_on_drop(true)
                .arg("-y")
                .arg("-ss")
                .arg(start_seconds)
                .arg("-t")
                .arg(duration_seconds)
                .arg("-i")
                .arg(audio_path)
                .arg("-vn")
                .arg("-ac")
                .arg("1")
                .arg("-ar")
                .arg("16000")
                .arg(chunk_path)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| transcript_error(video_id, "ffmpeg chunk extraction timed out"))?
        .map_err(|error| transcript_error(video_id, error))?;

        if !output.status.success() {
            return Err(transcript_error(
                video_id,
                format!(
                    "ffmpeg chunk extraction failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        Ok(())
    }

    async fn transcribe_chunk(
        &self,
        video_id: &str,
        chunk_path: &Path,
    ) -> youtube_ingest::Result<String> {
        let mut command = Command::new(&self.config.voxtype_binary);
        command.kill_on_drop(true).arg("transcribe");
        if let Some(engine) = &self.config.voxtype_engine {
            command.arg("--engine").arg(engine);
        }
        command
            .arg(chunk_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = timeout(CHUNK_TIMEOUT, command.output())
            .await
            .map_err(|_| transcript_error(video_id, "voxtype transcription timed out"))?
            .map_err(|error| transcript_error(video_id, error))?;

        if !output.status.success() {
            return Err(transcript_error(
                video_id,
                format!(
                    "voxtype failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        Ok(normalize_transcriber_output(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }
}

impl Default for VoxtypeTranscriptSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TranscriptSource for VoxtypeTranscriptSource {
    async fn transcript_for_video(
        &self,
        video_id: &str,
    ) -> youtube_ingest::Result<Vec<TranscriptSegment>> {
        self.cached_or_transcribe(video_id).await
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionConfig {
    pub cache_dir: PathBuf,
    pub chunk_seconds: u32,
    pub yt_dlp_binary: String,
    pub ffmpeg_binary: String,
    pub ffprobe_binary: String,
    pub voxtype_binary: String,
    pub voxtype_engine: Option<String>,
}

impl TranscriptionConfig {
    pub fn with_cache_dir(mut self, cache_dir: impl Into<PathBuf>) -> Self {
        self.cache_dir = cache_dir.into();
        self
    }

    pub fn with_chunk_seconds(mut self, chunk_seconds: u32) -> Self {
        self.chunk_seconds = chunk_seconds.max(1);
        self
    }

    pub fn with_voxtype_engine(mut self, engine: Option<impl Into<String>>) -> Self {
        self.voxtype_engine = engine.map(Into::into);
        self
    }

    fn audio_dir(&self) -> PathBuf {
        self.cache_dir.join("audio")
    }

    fn chunks_dir(&self) -> PathBuf {
        self.cache_dir.join("chunks")
    }

    fn transcripts_dir(&self) -> PathBuf {
        self.cache_dir.join("transcripts")
    }

    fn audio_path(&self, video_id: &str) -> PathBuf {
        self.audio_dir()
            .join(format!("{}.wav", safe_file_stem(video_id)))
    }

    fn chunk_path(&self, video_id: &str, start_ms: TimestampMs) -> PathBuf {
        self.chunks_dir()
            .join(format!("{}_{}.wav", safe_file_stem(video_id), start_ms))
    }

    fn transcript_path(&self, video_id: &str) -> PathBuf {
        self.transcripts_dir()
            .join(format!("{}.json", safe_file_stem(video_id)))
    }

    fn audio_output_template(&self, video_id: &str) -> PathBuf {
        self.audio_dir()
            .join(format!("{}.%(ext)s", safe_file_stem(video_id)))
    }

    async fn ensure_directories(&self) -> youtube_ingest::Result<()> {
        for directory in [self.audio_dir(), self.chunks_dir(), self.transcripts_dir()] {
            fs::create_dir_all(directory).await.map_err(|error| {
                IngestError::TranscriptUnavailable {
                    video_id: "<setup>".to_string(),
                    reason: error.to_string(),
                }
            })?;
        }
        Ok(())
    }
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            cache_dir: PathBuf::from(DEFAULT_CACHE_DIR),
            chunk_seconds: DEFAULT_CHUNK_SECONDS,
            yt_dlp_binary: DEFAULT_YT_DLP_BINARY.to_string(),
            ffmpeg_binary: DEFAULT_FFMPEG_BINARY.to_string(),
            ffprobe_binary: DEFAULT_FFPROBE_BINARY.to_string(),
            voxtype_binary: DEFAULT_VOXTYPE_BINARY.to_string(),
            voxtype_engine: None,
        }
    }
}

fn transcript_error(video_id: &str, error: impl ToString) -> IngestError {
    IngestError::TranscriptUnavailable {
        video_id: video_id.to_string(),
        reason: error.to_string(),
    }
}

fn safe_file_stem(video_id: &str) -> String {
    let mut safe = String::with_capacity(video_id.len());
    for byte in video_id.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' {
            safe.push(char::from(byte));
        }
    }
    if safe.is_empty() {
        "video".to_string()
    } else {
        safe
    }
}

fn video_url(video_id: &str) -> String {
    let mut url = String::with_capacity(YOUTUBE_WATCH_URL_PREFIX.len() + video_id.len());
    url.push_str(YOUTUBE_WATCH_URL_PREFIX);
    url.push_str(video_id);
    url
}

fn format_seconds(milliseconds: TimestampMs) -> String {
    let seconds = f64::from(milliseconds) / 1000.0;
    format!("{seconds:.3}")
}

fn has_text(text: &str) -> bool {
    text.split_whitespace().next().is_some()
}

fn normalize_transcriber_output(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .filter(|line| is_transcript_line(line))
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_transcript_line(line: &str) -> bool {
    if line.is_empty() {
        return false;
    }

    const LOG_PREFIXES: [&str; 18] = [
        "Loading audio file:",
        "Audio format:",
        "Processing ",
        "whisper_",
        "whisper_model_load:",
        "whisper_backend_",
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
    ];

    !LOG_PREFIXES.iter().any(|prefix| line.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_file_stem_keeps_youtube_id_characters() {
        assert_eq!(safe_file_stem("abc_DEF-123"), "abc_DEF-123");
        assert_eq!(safe_file_stem("../bad"), "bad");
    }

    #[test]
    fn formats_seconds_for_ffmpeg() {
        assert_eq!(format_seconds(1_234), "1.234");
        assert_eq!(format_seconds(30_000), "30.000");
    }

    #[test]
    fn normalizes_transcriber_output() {
        assert_eq!(
            normalize_transcriber_output(" hello\n  world "),
            "hello world"
        );
    }
}
