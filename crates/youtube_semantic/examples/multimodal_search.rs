//! Search precomputed transcript, intra-frame, and inter-frame embeddings.
//!
//! This is deliberately model-agnostic: generate each modality with its own
//! local encoder, then use this example to validate retrieval and fusion
//! without adding ML runtimes to the production API.

use serde::Deserialize;
use serde_json::json;
use std::{
    env,
    error::Error,
    fs::File,
    io::{BufRead, BufReader},
};
use youtube_semantic::{EvidenceKind, ModalityQuery, MultimodalVectorStore, SemanticEvidence};

#[derive(Deserialize)]
struct EvidenceRow {
    id: String,
    segment_id: String,
    video_id: String,
    t_start_ms: u32,
    t_end_ms: u32,
    kind: String,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct QueryFile {
    #[serde(default)]
    transcript: Option<Vec<f32>>,
    #[serde(default)]
    intra_frame: Option<Vec<f32>>,
    #[serde(default)]
    inter_frame: Option<Vec<f32>>,
    #[serde(default = "default_weight")]
    transcript_weight: f32,
    #[serde(default = "default_weight")]
    intra_frame_weight: f32,
    #[serde(default = "default_weight")]
    inter_frame_weight: f32,
}

fn default_weight() -> f32 {
    1.0
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let corpus_path = args.next().ok_or(
        "usage: cargo run -p youtube_semantic --example multimodal_search -- CORPUS.jsonl QUERY.json",
    )?;
    let query_path = args.next().ok_or("missing query JSON path")?;
    if args.next().is_some() {
        return Err("unexpected extra argument".into());
    }

    let store = MultimodalVectorStore::new();
    let reader = BufReader::new(File::open(corpus_path)?);
    for (line_index, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row: EvidenceRow = serde_json::from_str(&line)
            .map_err(|error| format!("invalid corpus line {}: {error}", line_index + 1))?;
        store.upsert([SemanticEvidence {
            id: row.id,
            segment_id: row.segment_id,
            video_id: row.video_id,
            t_start_ms: row.t_start_ms,
            t_end_ms: row.t_end_ms,
            kind: parse_kind(&row.kind)?,
            embedding: row.embedding,
        }])?;
    }

    let query: QueryFile = serde_json::from_reader(File::open(query_path)?)?;
    let mut modalities = Vec::new();
    if let Some(embedding) = &query.transcript {
        modalities.push(ModalityQuery {
            kind: EvidenceKind::Transcript,
            embedding,
            weight: query.transcript_weight,
        });
    }
    if let Some(embedding) = &query.intra_frame {
        modalities.push(ModalityQuery {
            kind: EvidenceKind::IntraFrame,
            embedding,
            weight: query.intra_frame_weight,
        });
    }
    if let Some(embedding) = &query.inter_frame {
        modalities.push(ModalityQuery {
            kind: EvidenceKind::InterFrame,
            embedding,
            weight: query.inter_frame_weight,
        });
    }

    let results = store.search_fused(&modalities, 50, 60, 20)?;
    let output = results
        .iter()
        .map(|result| {
            json!({
                "segment_id": result.segment_id,
                "video_id": result.video_id,
                "t_start_ms": result.t_start_ms,
                "t_end_ms": result.t_end_ms,
                "score": result.score,
                "evidence": result.evidence.iter().map(|item| json!({
                    "id": item.evidence.id,
                    "kind": format!("{:?}", item.evidence.kind),
                    "similarity": item.score,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn parse_kind(value: &str) -> Result<EvidenceKind, Box<dyn Error>> {
    match value {
        "transcript" => Ok(EvidenceKind::Transcript),
        "intra_frame" => Ok(EvidenceKind::IntraFrame),
        "inter_frame" => Ok(EvidenceKind::InterFrame),
        _ => Err(format!(
            "unknown evidence kind {value:?}; expected transcript, intra_frame, or inter_frame"
        )
        .into()),
    }
}
