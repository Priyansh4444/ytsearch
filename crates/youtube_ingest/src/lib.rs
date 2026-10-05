mod chunker;
mod error;
mod model;
mod ports;

pub use chunker::Chunker;
pub use error::{IngestError, Result};
pub use model::{
    DurationMs, TimestampMs, TranscriptChunk, TranscriptSegment, VideoMetadata, Youtuber,
};
pub use ports::{TranscriptSource, VideoMetadataSource};

/// The canonical creator ID: the handle without `@`, lowercased, with runs of
/// other characters collapsed to `-`; the channel ID when there is no handle.
/// Every metadata source and indexing path must use this, or one creator's
/// videos are filed under two IDs.
pub fn stable_youtuber_id(handle: &str, channel_id: &str) -> String {
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

#[cfg(test)]
mod youtuber_id_tests {
    use super::stable_youtuber_id;

    #[test]
    fn normalizes_handles_and_falls_back_to_the_channel_id() {
        assert_eq!(stable_youtuber_id("@T3 Dot GG", "fallback"), "t3-dot-gg");
        assert_eq!(stable_youtuber_id("@t3dotgg", "UCx"), "t3dotgg");
        assert_eq!(stable_youtuber_id("t3dotgg", "UCx"), "t3dotgg");
        assert_eq!(stable_youtuber_id("", "UC123"), "uc123");
    }
}
