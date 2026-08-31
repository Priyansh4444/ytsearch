use crate::RawSearchEngine;
use serde_json::Value;
use std::sync::{
    Arc, RwLock,
    atomic::{AtomicU64, Ordering},
};
use youtube_search_service::{BackendError, SearchBackend, SearchParams};

pub struct ReloadableEngine {
    engine: RwLock<Arc<RawSearchEngine>>,
    sequence: AtomicU64,
}

impl ReloadableEngine {
    pub fn new(engine: RawSearchEngine, sequence: u64) -> Self {
        Self {
            engine: RwLock::new(Arc::new(engine)),
            sequence: AtomicU64::new(sequence),
        }
    }

    pub fn active(&self) -> Result<Arc<RawSearchEngine>, BackendError> {
        self.engine
            .read()
            .map(|engine| Arc::clone(&engine))
            .map_err(|_| BackendError::Internal("raw engine read lock poisoned".to_string()))
    }

    /// Pins one immutable engine and its activation sequence under the same
    /// read lock. Administrative audits can then enumerate a catalog without
    /// mixing generations during a concurrent atomic replacement.
    pub fn snapshot(&self) -> Result<(Arc<RawSearchEngine>, u64), BackendError> {
        let engine = self
            .engine
            .read()
            .map_err(|_| BackendError::Internal("raw engine read lock poisoned".to_string()))?;
        let sequence = self.sequence.load(Ordering::Acquire);
        Ok((Arc::clone(&engine), sequence))
    }

    pub fn replace(&self, engine: RawSearchEngine, sequence: u64) -> Result<(), BackendError> {
        let mut active = self
            .engine
            .write()
            .map_err(|_| BackendError::Internal("raw engine write lock poisoned".to_string()))?;
        *active = Arc::new(engine);
        self.sequence.store(sequence, Ordering::Release);
        Ok(())
    }

    pub fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }
}

impl SearchBackend for ReloadableEngine {
    fn name(&self) -> &'static str {
        "raw-inverted-index"
    }

    fn search(&self, params: &SearchParams) -> Result<Value, BackendError> {
        self.active()?.search(params).map_err(map_error)
    }

    fn search_channels(&self, params: &SearchParams) -> Result<Value, BackendError> {
        self.active()?.search_channels(params).map_err(map_error)
    }

    fn documents(&self) -> u64 {
        self.active().map_or(0, |engine| engine.document_count())
    }

    fn index_bytes(&self) -> u64 {
        self.active().map_or(0, |engine| engine.index_size())
    }

    fn cache_generation(&self) -> u64 {
        self.sequence()
    }
}

fn map_error(error: crate::Error) -> BackendError {
    match error {
        crate::Error::InvalidInput(message) => BackendError::InvalidInput(message),
        error => BackendError::Internal(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
