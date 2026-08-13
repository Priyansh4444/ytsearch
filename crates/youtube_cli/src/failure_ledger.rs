use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const RECORD_VERSION: u8 = 1;
const MAX_REASON_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FailureCategory {
    BotChallenge,
    RateLimited,
    Scheduled,
    NoCaptions,
    Private,
    Unavailable,
    Timeout,
    Metadata,
    Import,
    Chunking,
    Unknown,
}

impl FailureCategory {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::BotChallenge => "bot_challenge",
            Self::RateLimited => "rate_limited",
            Self::Scheduled => "scheduled",
            Self::NoCaptions => "no_captions",
            Self::Private => "private",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Metadata => "metadata",
            Self::Import => "import",
            Self::Chunking => "chunking",
            Self::Unknown => "unknown",
        }
    }

    pub(super) const fn retryable(self) -> bool {
        !matches!(
            self,
            Self::NoCaptions | Self::Private | Self::Unavailable | Self::Chunking
        )
    }

    pub(super) const fn blocks_run(self) -> bool {
        matches!(self, Self::BotChallenge | Self::RateLimited)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum FailureStage {
    Metadata,
    Transcript,
    Chunking,
    Import,
    Timeout,
}

#[derive(Clone, Debug)]
pub(super) struct FailureLedger {
    directory: PathBuf,
    default_retry_after: Duration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FailureDecision {
    pub(super) deferred: bool,
    pub(super) category: Option<FailureCategory>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FailureRecord {
    version: u8,
    video_id: String,
    category: FailureCategory,
    retryable: bool,
    attempts: u32,
    first_failed_at_unix: u64,
    last_failed_at_unix: u64,
    next_retry_at_unix: Option<u64>,
    reason: String,
}

impl FailureLedger {
    pub(super) fn new(directory: PathBuf, default_retry_after: Duration) -> Self {
        Self {
            directory,
            default_retry_after,
        }
    }

    pub(super) fn decision(&self, video_id: &str) -> FailureDecision {
        let path = failure_path(&self.directory, video_id);
        let now = unix_seconds(SystemTime::now());
        match read_record(&path) {
            Ok(Some(record)) => {
                let category = effective_category(&record);
                let policy_retry_at = category.retryable().then(|| {
                    record.last_failed_at_unix.saturating_add(
                        jittered_retry_delay(
                            video_id,
                            category,
                            record.attempts,
                            self.default_retry_after,
                        )
                        .as_secs(),
                    )
                });
                // A record may have been written by an older retry policy. Keep
                // its attempt history, but never let a stale persisted deadline
                // defer work beyond the current category-specific policy.
                let retry_at = match (record.next_retry_at_unix, policy_retry_at) {
                    (Some(stored), Some(current)) => Some(stored.min(current)),
                    (stored, current) => stored.or(current),
                };
                FailureDecision {
                    deferred: !category.retryable()
                        || retry_at.is_some_and(|retry_at| now < retry_at),
                    category: Some(category),
                }
            }
            Ok(None) => legacy_decision(&path, self.default_retry_after),
            Err(error) => {
                tracing::warn!(video_id, %error, "failed to read classified video failure");
                legacy_decision(&path, self.default_retry_after)
            }
        }
    }

    pub(super) fn record(
        &self,
        video_id: &str,
        stage: FailureStage,
        reason: &str,
    ) -> FailureCategory {
        let category = classify(stage, reason);
        let path = failure_path(&self.directory, video_id);
        let previous = read_record(&path).ok().flatten();
        let now = unix_seconds(SystemTime::now());
        let attempts = previous
            .as_ref()
            .map_or(1, |record| record.attempts.saturating_add(1));
        let retryable = category.retryable();
        let record = FailureRecord {
            version: RECORD_VERSION,
            video_id: video_id.to_string(),
            category,
            retryable,
            attempts,
            first_failed_at_unix: previous
                .as_ref()
                .map_or(now, |record| record.first_failed_at_unix),
            last_failed_at_unix: now,
            next_retry_at_unix: retryable.then(|| {
                now.saturating_add(
                    jittered_retry_delay(video_id, category, attempts, self.default_retry_after)
                        .as_secs(),
                )
            }),
            reason: bounded_reason(reason),
        };
        if let Err(error) = write_record_atomic(&path, &record) {
            tracing::warn!(video_id, %error, "failed to persist classified video failure");
        }
        category
    }

    pub(super) fn clear(&self, video_id: &str) {
        let path = failure_path(&self.directory, video_id);
        if let Err(error) = fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(video_id, %error, "failed to clear classified video failure");
        }
    }
}

fn classify(stage: FailureStage, reason: &str) -> FailureCategory {
    let reason = reason.to_ascii_lowercase();
    if reason.contains("premieres in")
        || reason.contains("premiere will begin")
        || reason.contains("upcoming live event")
        || reason.contains("live event will begin")
        || reason.contains("we're processing this video")
        || reason.contains("we are processing this video")
    {
        return FailureCategory::Scheduled;
    }
    if reason.contains("sign in to confirm")
        || reason.contains("not a bot")
        || reason.contains("bot challenge")
    {
        return FailureCategory::BotChallenge;
    }
    if reason.contains("429")
        || reason.contains("too many requests")
        || reason.contains("rate-limited")
        || reason.contains("rate limited")
        || reason.contains("rate limit exceeded")
    {
        return FailureCategory::RateLimited;
    }
    if reason.contains("no english subtitles")
        || reason.contains("no usable captions")
        || reason.contains("no usable transcript")
        || reason.contains("captions are unavailable")
    {
        return FailureCategory::NoCaptions;
    }
    if reason.contains("private video") || reason.contains("this video is private") {
        return FailureCategory::Private;
    }
    if reason.contains("video unavailable")
        || reason.contains("has been removed")
        || reason.contains("deleted video")
    {
        return FailureCategory::Unavailable;
    }
    match stage {
        FailureStage::Timeout => FailureCategory::Timeout,
        FailureStage::Metadata => FailureCategory::Metadata,
        FailureStage::Import => FailureCategory::Import,
        FailureStage::Chunking => FailureCategory::Chunking,
        FailureStage::Transcript => FailureCategory::Unknown,
    }
}

fn retry_delay(category: FailureCategory, attempts: u32, default: Duration) -> Duration {
    let (base, maximum) = match category {
        FailureCategory::Import => (Duration::from_secs(60), Duration::from_secs(3_600)),
        FailureCategory::Timeout => (Duration::from_secs(900), Duration::from_secs(21_600)),
        FailureCategory::Scheduled => (Duration::from_secs(3_600), Duration::from_secs(43_200)),
        FailureCategory::Metadata | FailureCategory::Unknown => (
            default.max(Duration::from_secs(900)),
            Duration::from_secs(86_400),
        ),
        FailureCategory::BotChallenge => (
            default.max(Duration::from_secs(3_600)),
            Duration::from_secs(259_200),
        ),
        FailureCategory::RateLimited => (Duration::from_secs(3_600), Duration::from_secs(86_400)),
        FailureCategory::NoCaptions
        | FailureCategory::Private
        | FailureCategory::Unavailable
        | FailureCategory::Chunking => return Duration::ZERO,
    };
    let exponent = attempts.saturating_sub(1).min(10);
    base.saturating_mul(1_u32 << exponent).min(maximum)
}

fn effective_category(record: &FailureRecord) -> FailureCategory {
    if record.category == FailureCategory::Unknown {
        classify(FailureStage::Transcript, &record.reason)
    } else {
        record.category
    }
}

fn jittered_retry_delay(
    video_id: &str,
    category: FailureCategory,
    attempts: u32,
    default: Duration,
) -> Duration {
    let delay = retry_delay(category, attempts, default);
    if delay.is_zero() {
        return delay;
    }
    let key = format!("{video_id}:{}:{attempts}", category.as_str());
    let digest = blake3::hash(key.as_bytes());
    let jitter_bucket = u16::from_le_bytes([digest.as_bytes()[0], digest.as_bytes()[1]]) % 41;
    let percent = u64::from(80 + jitter_bucket);
    Duration::from_secs(delay.as_secs().saturating_mul(percent) / 100)
}

fn legacy_decision(path: &Path, retry_after: Duration) -> FailureDecision {
    if retry_after.is_zero() {
        return FailureDecision::default();
    }
    let recent = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < retry_after);
    FailureDecision {
        deferred: recent,
        category: recent.then_some(FailureCategory::Unknown),
    }
}

fn read_record(path: &Path) -> std::io::Result<Option<FailureRecord>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    match serde_json::from_slice::<FailureRecord>(&bytes) {
        Ok(record) if record.version == RECORD_VERSION => Ok(Some(record)),
        Ok(_) | Err(_) => Ok(None),
    }
}

fn write_record_atomic(path: &Path, record: &FailureRecord) -> std::io::Result<()> {
    let directory = path.parent().expect("failure path has parent directory");
    fs::create_dir_all(directory)?;
    let temporary = path.with_extension(format!(
        "failed.tmp.{}.{}",
        std::process::id(),
        unix_nanos(SystemTime::now())
    ));
    let encoded = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(&encoded)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    File::open(directory)?.sync_all()
}

fn bounded_reason(reason: &str) -> String {
    let normalized = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = String::with_capacity(normalized.len().min(MAX_REASON_BYTES));
    for character in normalized.chars() {
        if output.len() + character.len_utf8() > MAX_REASON_BYTES {
            break;
        }
        output.push(character);
    }
    output
}

fn failure_path(directory: &Path, video_id: &str) -> PathBuf {
    let key = video_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    directory.join(format!("{key}.failed"))
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_nanos(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ledger(name: &str) -> FailureLedger {
        let directory = std::env::temp_dir().join(format!(
            "ytsearch-failure-ledger-{name}-{}-{}",
            std::process::id(),
            unix_nanos(SystemTime::now())
        ));
        FailureLedger::new(directory, Duration::from_secs(21_600))
    }

    #[test]
    fn classifies_operational_and_terminal_failures() {
        assert_eq!(
            classify(
                FailureStage::Transcript,
                "Sign in to confirm you're not a bot"
            ),
            FailureCategory::BotChallenge
        );
        assert_eq!(
            classify(FailureStage::Metadata, "Premieres in 9 hours"),
            FailureCategory::Scheduled
        );
        assert_eq!(
            classify(
                FailureStage::Transcript,
                "We're processing this video. Check back later."
            ),
            FailureCategory::Scheduled
        );
        assert_eq!(
            classify(
                FailureStage::Transcript,
                "The current session has been rate-limited by YouTube for up to an hour"
            ),
            FailureCategory::RateLimited
        );
        assert_eq!(
            classify(
                FailureStage::Transcript,
                "yt-dlp found no English subtitles or automatic captions"
            ),
            FailureCategory::NoCaptions
        );
        assert_eq!(
            classify(FailureStage::Import, "connection reset"),
            FailureCategory::Import
        );
    }

    #[test]
    fn terminal_failure_stays_deferred_and_clear_removes_it() {
        let ledger = test_ledger("terminal");
        assert_eq!(
            ledger.record(
                "video-1",
                FailureStage::Transcript,
                "no English subtitles or automatic captions"
            ),
            FailureCategory::NoCaptions
        );
        assert_eq!(
            ledger.decision("video-1"),
            FailureDecision {
                deferred: true,
                category: Some(FailureCategory::NoCaptions)
            }
        );
        ledger.clear("video-1");
        assert_eq!(ledger.decision("video-1"), FailureDecision::default());
        let _ = fs::remove_dir_all(&ledger.directory);
    }

    #[test]
    fn retryable_failure_persists_attempts_and_future_retry() {
        let ledger = test_ledger("retryable");
        ledger.record("video-2", FailureStage::Import, "upstream 503");
        ledger.record("video-2", FailureStage::Import, "upstream 503");
        let record = read_record(&failure_path(&ledger.directory, "video-2"))
            .unwrap()
            .unwrap();
        assert_eq!(record.attempts, 2);
        assert!(record.retryable);
        assert!(record.next_retry_at_unix.unwrap() > record.last_failed_at_unix);
        assert!(ledger.decision("video-2").deferred);
        let _ = fs::remove_dir_all(&ledger.directory);
    }

    #[test]
    fn retry_jitter_is_stable_and_bounded() {
        let base = retry_delay(FailureCategory::RateLimited, 2, Duration::from_secs(21_600));
        let first = jittered_retry_delay(
            "video-3",
            FailureCategory::RateLimited,
            2,
            Duration::from_secs(21_600),
        );
        let second = jittered_retry_delay(
            "video-3",
            FailureCategory::RateLimited,
            2,
            Duration::from_secs(21_600),
        );
        assert_eq!(first, second);
        assert!(first >= base.saturating_mul(80) / 100);
        assert!(first <= base.saturating_mul(120) / 100);
    }

    #[test]
    fn upstream_protection_failures_block_the_current_run() {
        assert!(FailureCategory::RateLimited.blocks_run());
        assert!(FailureCategory::BotChallenge.blocks_run());
        assert!(!FailureCategory::NoCaptions.blocks_run());
        assert!(!FailureCategory::Metadata.blocks_run());
    }

    #[test]
    fn an_unknown_record_is_reclassified_from_its_preserved_reason() {
        let ledger = test_ledger("reclassify");
        let path = failure_path(&ledger.directory, "rate-limited-video");
        let now = unix_seconds(SystemTime::now());
        write_record_atomic(
            &path,
            &FailureRecord {
                version: RECORD_VERSION,
                video_id: "rate-limited-video".to_string(),
                category: FailureCategory::Unknown,
                retryable: true,
                attempts: 1,
                first_failed_at_unix: now,
                last_failed_at_unix: now,
                next_retry_at_unix: Some(now.saturating_add(21_600)),
                reason: "current session has been rate-limited by YouTube".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            ledger.decision("rate-limited-video").category,
            Some(FailureCategory::RateLimited)
        );
        let _ = fs::remove_dir_all(&ledger.directory);
    }

    #[test]
    fn persisted_retry_deadline_is_capped_by_the_current_policy() {
        let ledger = test_ledger("policy-cap");
        let path = failure_path(&ledger.directory, "old-rate-limit-video");
        let now = unix_seconds(SystemTime::now());
        let last_failed_at = now.saturating_sub(7_200);
        write_record_atomic(
            &path,
            &FailureRecord {
                version: RECORD_VERSION,
                video_id: "old-rate-limit-video".to_string(),
                category: FailureCategory::RateLimited,
                retryable: true,
                attempts: 1,
                first_failed_at_unix: last_failed_at,
                last_failed_at_unix: last_failed_at,
                next_retry_at_unix: Some(now.saturating_add(14_400)),
                reason: "upstream returned 429".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            ledger.decision("old-rate-limit-video"),
            FailureDecision {
                deferred: false,
                category: Some(FailureCategory::RateLimited),
            }
        );
        let _ = fs::remove_dir_all(&ledger.directory);
    }

    #[test]
    fn legacy_marker_uses_configured_cooldown() {
        let ledger = test_ledger("legacy");
        fs::create_dir_all(&ledger.directory).unwrap();
        fs::write(
            failure_path(&ledger.directory, "legacy-video"),
            b"transcript fetch failed; retry after cooldown\n",
        )
        .unwrap();
        assert_eq!(
            ledger.decision("legacy-video"),
            FailureDecision {
                deferred: true,
                category: Some(FailureCategory::Unknown)
            }
        );
        let _ = fs::remove_dir_all(&ledger.directory);
    }
}
