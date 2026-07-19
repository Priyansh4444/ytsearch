use async_trait::async_trait;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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

/// The independently searchable signals in the multimodal experiment.
///
/// `IntraFrame` describes what is visible in one frame. `InterFrame` describes
/// a change or action between two frames and therefore spans a time interval.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EvidenceKind {
    Transcript,
    IntraFrame,
    InterFrame,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticEvidence {
    pub id: String,
    /// Evidence from different modalities should share this ID when it refers
    /// to the same scene or transcript interval, allowing rank fusion.
    pub segment_id: String,
    pub video_id: String,
    pub t_start_ms: u32,
    pub t_end_ms: u32,
    pub kind: EvidenceKind,
    pub embedding: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceMatch {
    pub evidence: SemanticEvidence,
    pub score: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FusedSemanticMatch {
    pub segment_id: String,
    pub video_id: String,
    pub t_start_ms: u32,
    pub t_end_ms: u32,
    pub score: f32,
    pub evidence: Vec<EvidenceMatch>,
}

#[derive(Clone, Copy, Debug)]
pub struct ModalityQuery<'a> {
    pub kind: EvidenceKind,
    pub embedding: &'a [f32],
    pub weight: f32,
}

/// Exact, in-memory store for the bounded frame experiment.
///
/// It intentionally keeps modalities separate: image-text embeddings for
/// frames are not assumed to share a vector space with transcript embeddings
/// or temporal-transition embeddings.
#[derive(Clone, Debug, Default)]
pub struct MultimodalVectorStore {
    evidence: Arc<RwLock<BTreeMap<String, SemanticEvidence>>>,
}

impl MultimodalVectorStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> Result<usize> {
        Ok(self
            .evidence
            .read()
            .map_err(|_| SemanticError::StorePoisoned)?
            .len())
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn upsert(&self, values: impl IntoIterator<Item = SemanticEvidence>) -> Result<()> {
        let mut store = self
            .evidence
            .write()
            .map_err(|_| SemanticError::StorePoisoned)?;
        for value in values {
            validate_evidence(&value)?;
            if let Some(existing) = store
                .values()
                .find(|existing| existing.kind == value.kind && existing.id != value.id)
                && existing.embedding.len() != value.embedding.len()
            {
                return Err(SemanticError::InvalidEvidence(format!(
                    "{:?} embeddings must all have {} dimensions",
                    value.kind,
                    existing.embedding.len()
                )));
            }
            store.insert(value.id.clone(), value);
        }
        Ok(())
    }

    pub fn search_kind(
        &self,
        kind: EvidenceKind,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<EvidenceMatch>> {
        validate_query(query)?;
        let store = self
            .evidence
            .read()
            .map_err(|_| SemanticError::StorePoisoned)?;
        if let Some(candidate) = store.values().find(|value| value.kind == kind)
            && candidate.embedding.len() != query.len()
        {
            return Err(SemanticError::InvalidEmbedding(format!(
                "{:?} query has {} dimensions; expected {}",
                kind,
                query.len(),
                candidate.embedding.len()
            )));
        }
        let mut matches = store
            .values()
            .filter(|value| value.kind == kind)
            .filter_map(|value| {
                cosine_similarity(query, &value.embedding).map(|score| EvidenceMatch {
                    evidence: value.clone(),
                    score,
                })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.evidence.id.cmp(&right.evidence.id))
        });
        matches.truncate(limit);
        Ok(matches)
    }

    /// Retrieve each modality independently and fuse scene/interval ranks.
    pub fn search_fused(
        &self,
        queries: &[ModalityQuery<'_>],
        per_modality: usize,
        rank_constant: u32,
        limit: usize,
    ) -> Result<Vec<FusedSemanticMatch>> {
        let mut fused = BTreeMap::<String, FusedSemanticMatch>::new();
        let rank_constant = rank_constant as f32;
        for query in queries {
            if query.weight <= 0.0 {
                continue;
            }
            for (rank, found) in self
                .search_kind(query.kind, query.embedding, per_modality)?
                .into_iter()
                .enumerate()
            {
                let contribution = query.weight / (rank_constant + rank as f32 + 1.0);
                let segment_id = found.evidence.segment_id.clone();
                let fusion_key = format!("{}\0{segment_id}", found.evidence.video_id);
                let entry = fused
                    .entry(fusion_key)
                    .or_insert_with(|| FusedSemanticMatch {
                        segment_id,
                        video_id: found.evidence.video_id.clone(),
                        t_start_ms: found.evidence.t_start_ms,
                        t_end_ms: found.evidence.t_end_ms,
                        score: 0.0,
                        evidence: Vec::new(),
                    });
                entry.t_start_ms = entry.t_start_ms.min(found.evidence.t_start_ms);
                entry.t_end_ms = entry.t_end_ms.max(found.evidence.t_end_ms);
                entry.score += contribution;
                entry.evidence.push(found);
            }
        }
        let mut results = fused.into_values().collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.segment_id.cmp(&right.segment_id))
        });
        results.truncate(limit);
        Ok(results)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FusedMatch {
    pub chunk_id: String,
    pub score: f32,
}

/// Fuse independently ranked result lists without assuming comparable scores.
///
/// `rank_constant` controls how strongly the highest positions dominate. The
/// conventional RRF default is 60. Each list can be weighted independently.
pub fn reciprocal_rank_fusion(
    rankings: &[(&[String], f32)],
    rank_constant: u32,
    limit: usize,
) -> Vec<FusedMatch> {
    let mut scores = HashMap::<String, f32>::new();
    let rank_constant = rank_constant as f32;

    for (ranking, weight) in rankings {
        let mut seen = HashSet::new();
        for (offset, chunk_id) in ranking.iter().enumerate() {
            if !seen.insert(chunk_id) {
                continue;
            }
            *scores.entry(chunk_id.clone()).or_default() +=
                weight.max(0.0) / (rank_constant + offset as f32 + 1.0);
        }
    }

    let mut matches = scores
        .into_iter()
        .map(|(chunk_id, score)| FusedMatch { chunk_id, score })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    matches.truncate(limit);
    matches
}

pub fn reciprocal_rank(results: &[String], relevant: &HashSet<String>) -> f32 {
    results
        .iter()
        .position(|chunk_id| relevant.contains(chunk_id))
        .map(|offset| 1.0 / (offset as f32 + 1.0))
        .unwrap_or(0.0)
}

pub fn recall_at_k(results: &[String], relevant: &HashSet<String>, k: usize) -> f32 {
    if relevant.is_empty() {
        return 0.0;
    }
    let found = results
        .iter()
        .take(k)
        .filter(|chunk_id| relevant.contains(*chunk_id))
        .collect::<HashSet<_>>()
        .len();
    found as f32 / relevant.len() as f32
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
    #[error("invalid evidence: {0}")]
    InvalidEvidence(String),
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

fn validate_query(embedding: &[f32]) -> Result<()> {
    if embedding.is_empty() || embedding.iter().any(|value| !value.is_finite()) {
        return Err(SemanticError::InvalidEmbedding(
            "embedding must contain finite values".to_string(),
        ));
    }
    Ok(())
}

fn validate_evidence(evidence: &SemanticEvidence) -> Result<()> {
    if evidence.id.trim().is_empty()
        || evidence.segment_id.trim().is_empty()
        || evidence.video_id.trim().is_empty()
    {
        return Err(SemanticError::InvalidEvidence(
            "id, segment_id, and video_id are required".to_string(),
        ));
    }
    if evidence.t_end_ms < evidence.t_start_ms {
        return Err(SemanticError::InvalidEvidence(
            "evidence end must not precede its start".to_string(),
        ));
    }
    validate_query(&evidence.embedding)
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

    fn evidence(
        id: &str,
        segment_id: &str,
        kind: EvidenceKind,
        embedding: Vec<f32>,
    ) -> SemanticEvidence {
        SemanticEvidence {
            id: id.to_string(),
            segment_id: segment_id.to_string(),
            video_id: "video-1".to_string(),
            t_start_ms: 10_000,
            t_end_ms: 15_000,
            kind,
            embedding,
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

    #[test]
    fn reciprocal_rank_fusion_rewards_agreement_between_retrievers() {
        let lexical = vec!["exact".to_string(), "shared".to_string()];
        let semantic = vec!["concept".to_string(), "shared".to_string()];

        let fused = reciprocal_rank_fusion(&[(&lexical, 1.0), (&semantic, 1.0)], 60, 3);

        assert_eq!(fused[0].chunk_id, "shared");
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn reciprocal_rank_fusion_deduplicates_within_each_retriever() {
        let repeated = vec!["same".to_string(), "same".to_string()];
        let other = vec!["other".to_string()];

        let fused = reciprocal_rank_fusion(&[(&repeated, 1.0), (&other, 1.0)], 60, 10);

        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].score, fused[1].score);
        assert_eq!(fused[0].chunk_id, "other");
    }

    #[test]
    fn retrieval_metrics_measure_first_hit_and_coverage() {
        let results = vec![
            "wrong".to_string(),
            "right-1".to_string(),
            "right-2".to_string(),
        ];
        let relevant = HashSet::from(["right-1".to_string(), "right-2".to_string()]);

        assert_eq!(reciprocal_rank(&results, &relevant), 0.5);
        assert_eq!(recall_at_k(&results, &relevant, 2), 0.5);
        assert_eq!(recall_at_k(&results, &relevant, 3), 1.0);
    }

    #[test]
    fn multimodal_search_keeps_vector_spaces_independent() {
        let store = MultimodalVectorStore::new();
        store
            .upsert([
                evidence(
                    "frame-near",
                    "scene-1",
                    EvidenceKind::IntraFrame,
                    vec![1.0, 0.0],
                ),
                evidence(
                    "frame-far",
                    "scene-2",
                    EvidenceKind::IntraFrame,
                    vec![0.0, 1.0],
                ),
                evidence(
                    "transition",
                    "scene-2",
                    EvidenceKind::InterFrame,
                    vec![1.0, 0.0, 0.0],
                ),
            ])
            .unwrap();

        let matches = store
            .search_kind(EvidenceKind::IntraFrame, &[1.0, 0.0], 10)
            .unwrap();

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].evidence.id, "frame-near");
    }

    #[test]
    fn multimodal_fusion_rewards_aligned_frame_and_transition_evidence() {
        let store = MultimodalVectorStore::new();
        store
            .upsert([
                evidence(
                    "frame-shared",
                    "scene-shared",
                    EvidenceKind::IntraFrame,
                    vec![0.9, 0.1],
                ),
                evidence(
                    "frame-only",
                    "scene-frame",
                    EvidenceKind::IntraFrame,
                    vec![1.0, 0.0],
                ),
                evidence(
                    "transition-shared",
                    "scene-shared",
                    EvidenceKind::InterFrame,
                    vec![1.0, 0.0, 0.0],
                ),
                evidence(
                    "transition-only",
                    "scene-transition",
                    EvidenceKind::InterFrame,
                    vec![0.0, 1.0, 0.0],
                ),
            ])
            .unwrap();
        let frame_query = [1.0, 0.0];
        let transition_query = [1.0, 0.0, 0.0];

        let matches = store
            .search_fused(
                &[
                    ModalityQuery {
                        kind: EvidenceKind::IntraFrame,
                        embedding: &frame_query,
                        weight: 1.0,
                    },
                    ModalityQuery {
                        kind: EvidenceKind::InterFrame,
                        embedding: &transition_query,
                        weight: 1.0,
                    },
                ],
                2,
                60,
                10,
            )
            .unwrap();

        assert_eq!(matches[0].segment_id, "scene-shared");
        assert_eq!(matches[0].evidence.len(), 2);
    }

    #[test]
    fn multimodal_store_rejects_invalid_intervals_and_embeddings() {
        let store = MultimodalVectorStore::new();
        let mut invalid = evidence("bad", "scene-bad", EvidenceKind::InterFrame, vec![f32::NAN]);
        invalid.t_start_ms = 20;
        invalid.t_end_ms = 10;

        assert!(matches!(
            store.upsert([invalid]),
            Err(SemanticError::InvalidEvidence(_))
        ));
    }

    #[test]
    fn multimodal_store_rejects_dimension_mismatches_within_a_modality() {
        let store = MultimodalVectorStore::new();
        store
            .upsert([evidence(
                "frame-1",
                "scene-1",
                EvidenceKind::IntraFrame,
                vec![1.0, 0.0],
            )])
            .unwrap();

        assert!(matches!(
            store.upsert([evidence(
                "frame-2",
                "scene-2",
                EvidenceKind::IntraFrame,
                vec![1.0, 0.0, 0.0],
            )]),
            Err(SemanticError::InvalidEvidence(_))
        ));
        assert!(matches!(
            store.search_kind(EvidenceKind::IntraFrame, &[1.0], 10),
            Err(SemanticError::InvalidEmbedding(_))
        ));
    }
}
