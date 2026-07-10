use async_trait::async_trait;
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use thiserror::Error;
use youtube_ingest::{EmbeddingSource, TranscriptChunk};

#[derive(Clone, Debug, Default)]
pub struct NoopEmbeddingSource;

#[async_trait]
impl EmbeddingSource for NoopEmbeddingSource {
    async fn embed_chunks(&self, _chunks: &mut [TranscriptChunk]) -> youtube_ingest::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct StaticEmbeddingSource {
    dimensions: usize,
    value: f32,
}

impl StaticEmbeddingSource {
    pub fn new(dimensions: usize, value: f32) -> Self {
        Self { dimensions, value }
    }
}

#[async_trait]
impl EmbeddingSource for StaticEmbeddingSource {
    async fn embed_chunks(&self, chunks: &mut [TranscriptChunk]) -> youtube_ingest::Result<()> {
        for chunk in chunks {
            chunk.embedding = Some(vec![self.value; self.dimensions]);
        }

        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticMatch {
    pub chunk_id: String,
    pub score: f32,
}

#[async_trait]
pub trait VectorStore {
    async fn upsert_chunks(&self, chunks: &[TranscriptChunk]) -> Result<()>;
    async fn search(&self, embedding: &[f32], limit: usize) -> Result<Vec<SemanticMatch>>;
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryVectorStore {
    embeddings: Arc<RwLock<BTreeMap<String, Vec<f32>>>>,
}

impl InMemoryVectorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> Result<usize> {
        Ok(self.read()?.len())
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, BTreeMap<String, Vec<f32>>>> {
        self.embeddings
            .read()
            .map_err(|_| SemanticError::StorePoisoned)
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, BTreeMap<String, Vec<f32>>>> {
        self.embeddings
            .write()
            .map_err(|_| SemanticError::StorePoisoned)
    }
}

#[async_trait]
impl VectorStore for InMemoryVectorStore {
    async fn upsert_chunks(&self, chunks: &[TranscriptChunk]) -> Result<()> {
        let mut embeddings = self.write()?;
        for chunk in chunks {
            if let Some(embedding) = &chunk.embedding {
                embeddings.insert(chunk.id.clone(), embedding.clone());
            }
        }

        Ok(())
    }

    async fn search(&self, embedding: &[f32], limit: usize) -> Result<Vec<SemanticMatch>> {
        if embedding.is_empty() {
            return Err(SemanticError::InvalidEmbedding(
                "query embedding cannot be empty".to_string(),
            ));
        }

        let embeddings = self.read()?;
        let mut matches = embeddings
            .iter()
            .filter_map(|(chunk_id, candidate)| {
                cosine_similarity(embedding, candidate).map(|score| SemanticMatch {
                    chunk_id: chunk_id.clone(),
                    score,
                })
            })
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| right.score.total_cmp(&left.score));
        matches.truncate(limit);
        Ok(matches)
    }
}

#[derive(Debug, Error)]
pub enum SemanticError {
    #[error("invalid embedding: {0}")]
    InvalidEmbedding(String),
    #[error("vector store lock was poisoned")]
    StorePoisoned,
}

pub type Result<T> = std::result::Result<T, SemanticError>;

fn cosine_similarity(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }

    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;

    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }

    Some(dot / (left_norm.sqrt() * right_norm.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn chunk(id: &str, embedding: Vec<f32>) -> TranscriptChunk {
        TranscriptChunk {
            id: id.to_string(),
            youtuber_id: "t3dotgg".to_string(),
            youtuber_handle: "@t3dotgg".to_string(),
            youtuber_name: "Theo - t3.gg".to_string(),
            channel_id: "channel-1".to_string(),
            video_id: "video-1".to_string(),
            title: "A useful video".to_string(),
            text: "hello world".to_string(),
            prev_text: None,
            next_text: None,
            sequence: 0,
            chapter: None,
            chapter_index: None,
            duration: Some(5_000),
            t_start_ms: 0,
            t_end_ms: 5_000,
            thumbnail: None,
            upload_date: Some("20260101".to_string()),
            published_at: Some("2026-01-01T00:00:00Z".to_string()),
            published_at_timestamp: Some(1_767_225_600),
            uploader_id: "@t3dotgg".to_string(),
            embedding: Some(embedding),
            extra: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn static_embedding_source_fills_chunks() {
        let mut chunks = vec![chunk("video-1_0", vec![0.0])];

        StaticEmbeddingSource::new(3, 0.5)
            .embed_chunks(&mut chunks)
            .await
            .unwrap();

        assert_eq!(
            chunks[0].embedding.as_deref(),
            Some([0.5, 0.5, 0.5].as_ref())
        );
    }

    #[tokio::test]
    async fn in_memory_store_returns_nearest_embedding() {
        let store = InMemoryVectorStore::new();
        store
            .upsert_chunks(&[chunk("near", vec![1.0, 0.0]), chunk("far", vec![0.0, 1.0])])
            .await
            .unwrap();

        let matches = store.search(&[1.0, 0.0], 2).await.unwrap();

        assert_eq!(matches[0].chunk_id, "near");
        assert!(matches[0].score > matches[1].score);
    }
}
