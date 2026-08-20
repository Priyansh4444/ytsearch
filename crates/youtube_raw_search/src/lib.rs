mod admin;
mod cache_io;
mod engine;
mod format;
mod fuzzy;
mod mutable;
mod positions;
mod publisher;
mod reloadable;

pub use engine::{BuildReport, RawSearchEngine, SliceReport, build_index, slice_corpus};
pub use mutable::{
    MutationSnapshot, MutationSnapshotExport, MutationStore, StoredChannelMutation,
    StoredVideoMutation, VideoMutationSummary,
};
pub use publisher::{PublishedGeneration, Publisher, PublisherPaths};
pub use reloadable::ReloadableEngine;
pub use youtube_search_service::SearchParams;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("index is corrupt: {0}")]
    Corrupt(String),
    #[error("index directory already contains a raw index: {0}")]
    ExistingIndex(String),
    #[error("input document is missing {0}")]
    MissingField(&'static str),
    #[error("invalid index input: {0}")]
    InvalidInput(String),
}

pub type Result<T> = std::result::Result<T, Error>;
pub use admin::{AdminState, router as admin_router};

fn channel_ids_equal(left: &str, right: &str) -> bool {
    CanonicalChannelId::new(left).eq(CanonicalChannelId::new(right))
}

struct CanonicalChannelId<'a> {
    bytes: std::str::Bytes<'a>,
    buffered: Option<u8>,
    emitted: bool,
    separator_pending: bool,
}

impl<'a> CanonicalChannelId<'a> {
    fn new(value: &'a str) -> Self {
        Self {
            bytes: value.strip_prefix('@').unwrap_or(value).bytes(),
            buffered: None,
            emitted: false,
            separator_pending: false,
        }
    }
}

impl Iterator for CanonicalChannelId<'_> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(byte) = self.buffered.take() {
            self.emitted = true;
            return Some(byte);
        }
        for byte in self.bytes.by_ref() {
            if byte.is_ascii_alphanumeric() {
                let byte = byte.to_ascii_lowercase();
                if self.separator_pending && self.emitted {
                    self.separator_pending = false;
                    self.buffered = Some(byte);
                    return Some(b'-');
                }
                self.emitted = true;
                return Some(byte);
            }
            if self.emitted {
                self.separator_pending = true;
            }
        }
        None
    }
}

#[cfg(test)]
mod channel_id_tests {
    use super::channel_ids_equal;

    #[test]
    fn compares_ids_with_the_shared_ascii_canonicalization_rule() {
        assert!(channel_ids_equal("cursor-ai", "@Cursor_AI"));
        assert!(channel_ids_equal("ardens", "@ardens."));
        assert!(channel_ids_equal("better-software", "@Better  Software"));
        assert!(!channel_ids_equal("cursor-ai", "cursor-ais"));
    }
}
