use crate::{RawSearchEngine, engine::WarmEngine};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex, RwLock,
    atomic::{AtomicU64, Ordering},
};
use youtube_search_service::{BackendError, SearchBackend, SearchParams};

pub struct ReloadableEngine {
    engine: RwLock<Arc<RawSearchEngine>>,
    sequence: AtomicU64,
    recent: Mutex<VecDeque<(Vec<u8>, SearchParams)>>,
}

impl ReloadableEngine {
    pub fn new(engine: RawSearchEngine, sequence: u64) -> Self {
        Self {
            engine: RwLock::new(Arc::new(engine)),
            sequence: AtomicU64::new(sequence),
            recent: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn active(&self) -> Result<Arc<RawSearchEngine>, BackendError> {
        self.engine
            .read()
            .map(|engine| Arc::clone(&engine))
            .map_err(|_| BackendError::Internal("raw engine read lock poisoned".to_string()))
    }

    /// Pins one immutable engine and its activation sequence under the same
    /// read lock. Administrative audits can then enumerate a catalog without
    /// mixing generations during a concurrent atomic replacement.
    pub(crate) fn snapshot(&self) -> Result<(Arc<RawSearchEngine>, u64), BackendError> {
        let engine = self
            .engine
            .read()
            .map_err(|_| BackendError::Internal("raw engine read lock poisoned".to_string()))?;
        let sequence = self.sequence.load(Ordering::Acquire);
        Ok((Arc::clone(&engine), sequence))
    }

    /// Swaps in a warmed engine. A cold engine cannot be passed: see
    /// `RawSearchEngine::warmed_for_activation`.
    pub(crate) fn replace(&self, engine: WarmEngine, sequence: u64) -> Result<(), BackendError> {
        let engine = engine.into_engine();
        let mut active = self
            .engine
            .write()
            .map_err(|_| BackendError::Internal("raw engine write lock poisoned".to_string()))?;
        let retired = std::mem::replace(&mut *active, Arc::new(engine));
        self.sequence.store(sequence, Ordering::Release);
        // Dropping a generation unmaps its files and frees its catalog. That
        // work must not extend the exclusive interval seen by query readers.
        drop(active);
        drop(retired);
        Ok(())
    }

    pub(crate) fn recent_searches(&self) -> Vec<SearchParams> {
        self.recent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(_, params)| params.clone())
            .collect()
    }

    fn record_search(&self, params: &SearchParams) {
        let Ok(key) = serde_json::to_vec(params) else {
            return;
        };
        // History has a fixed memory ceiling, independent of request length
        // and traffic. Preserve the complete query contract for exact replay.
        if key.len() > 8 * 1024 {
            return;
        }
        let mut recent = self
            .recent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(position) = recent.iter().position(|(existing, _)| existing == &key) {
            recent.remove(position);
        }
        recent.push_back((key, params.clone()));
        if recent.len() > 64 {
            recent.pop_front();
        }
    }

    pub(crate) fn sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }
}

impl SearchBackend for ReloadableEngine {
    fn name(&self) -> &'static str {
        "raw-inverted-index"
    }

    fn search(&self, params: &SearchParams) -> Result<Value, BackendError> {
        let response = self.active()?.search(params).map_err(map_error)?;
        self.record_search(params);
        Ok(response)
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
