mod engine;
mod format;
mod fuzzy;
mod positions;

pub use engine::{BuildReport, RawSearchEngine, SliceReport, build_index, slice_corpus};
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
