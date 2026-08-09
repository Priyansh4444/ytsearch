use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};
use thiserror::Error;

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

    fn search_kind(
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
