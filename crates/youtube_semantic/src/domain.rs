use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const MAX_TOKENS: usize = 512;
pub(crate) const MAX_VIDEO_CHUNKS: usize = 4096;
pub(crate) const MAX_VIDEO_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EmbeddingDimensions {
    #[default]
    D128,
    D256,
}
impl EmbeddingDimensions {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "128" => Ok(Self::D128),
            "256" => Ok(Self::D256),
            _ => Err(Error::Input("dimensions must be 128 or 256")),
        }
    }
    pub(crate) fn size(self) -> usize {
        match self {
            Self::D128 => 128,
            Self::D256 => 256,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Profile {
    pub(crate) dimensions: EmbeddingDimensions,
}
impl Profile {
    pub(crate) fn manifest(&self) -> Value {
        json!({
            "schema": 1,
            "creator_identity": "ASCII lowercase @handle",
            "repository": "onnx-community/embeddinggemma-300m-ONNX",
            "revision": "5090578d9565bb06545b4552f76e6bc2c93e4a66",
            "graph": "onnx/model_q4.onnx",
            "assets": ASSETS,
            "inference": "Q4 weights, float32 activations, sentence_embedding:768",
            "query_prompt": "task: search result | query: {q}",
            "document_prompt": "title: {title} | text: {text}",
            "bos": "tokenizer postprocessor",
            "max_tokens": MAX_TOKENS,
            "dimensions": self.dimensions.size(),
            "normalization": "prefix truncation then L2",
            "passages": "greedy ordered original chunks, source overlaps preserved, <=60000ms, token capped v2",
            "distance": "Cosine", "datatype": "float16",
            "vectors_on_disk": true, "payload_on_disk": true
        })
    }
    pub(crate) fn id(&self) -> String {
        digest(&serde_json::to_vec(&self.manifest()).expect("JSON manifest"))
    }
}

pub(crate) const ASSETS: [(&str, &str); 6] = [
    (
        "onnx/model_q4.onnx",
        "ad1dfee81a70f7944b9b9d1cc6e48075b832881cf33fab2f2b248be78f3f0043",
    ),
    (
        "onnx/model_q4.onnx_data",
        "599962c3143b040de2dd05e5975be3e9091dd067cacc6a8f7186e3203bab9e02",
    ),
    (
        "tokenizer.json",
        "4dda02faaf32bc91031dc8c88457ac272b00c1016cc679757d1c441b248b9c47",
    ),
    (
        "tokenizer_config.json",
        "3ca953eea6c3c9fcda9cf3df22949ff18b216f7c74bd6459230f3f1013953f3a",
    ),
    (
        "config.json",
        "6e1f06404b7163e0325ed2ea3e6781cde50f4a50b31780a95ad0d30e8404d77b",
    ),
    (
        "special_tokens_map.json",
        "2f7b0adf4fb469770bb1490e3e35df87b1dc578246c5e7e6fc76ecf33213a397",
    ),
];
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn query_prompt(q: &str) -> String {
    format!("task: search result | query: {q}")
}
pub(crate) fn document_prompt(title: &str, text: &str) -> String {
    format!("title: {title} | text: {text}")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Chunk {
    pub(crate) id: String,
    pub(crate) video_id: String,
    pub(crate) title: String,
    pub(crate) text: String,
    #[serde(deserialize_with = "deserialize_handle")]
    pub(crate) youtuber_handle: String,
    pub(crate) youtuber_name: String,
    pub(crate) t_start_ms: u32,
    pub(crate) t_end_ms: u32,
    #[serde(flatten)]
    pub(crate) metadata: serde_json::Map<String, Value>,
}
impl Chunk {
    pub(crate) fn validate(&self) -> Result<()> {
        if !valid_identity(&self.video_id)
            || !valid_identity(self.youtuber_handle.trim_start_matches('@'))
            || !self.youtuber_handle.starts_with('@')
            || self.youtuber_handle != self.youtuber_handle.to_lowercase()
            || self.id.is_empty()
            || self.id.len() > 256
            || self.title.trim().is_empty()
            || self.title.len() > 2048
            || self.youtuber_name.is_empty()
            || self.youtuber_name.len() > 512
            || self.text.trim().is_empty()
            || self.text.len() > 16 * 1024
            || self.t_end_ms <= self.t_start_ms
            || self.t_end_ms - self.t_start_ms > 60_000
        {
            return Err(Error::Input("invalid transcript chunk"));
        }
        Ok(())
    }
}
// Keep import and direct-service filters on one canonical identity boundary.
fn deserialize_handle<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    String::deserialize(d).map(|v| v.to_ascii_lowercase())
}
fn deserialize_creator<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d).map(|v| v.map(|v| v.to_ascii_lowercase()))
}
fn valid_identity(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Passage {
    pub(crate) id: String,
    pub(crate) profile_id: String,
    pub(crate) source_revision: String,
    pub(crate) text_digest: String,
    pub(crate) video_id: String,
    pub(crate) title: String,
    pub(crate) text: String,
    pub(crate) youtuber_handle: String,
    pub(crate) youtuber_name: String,
    pub(crate) t_start_ms: u32,
    pub(crate) t_end_ms: u32,
    #[serde(flatten)]
    pub(crate) metadata: serde_json::Map<String, Value>,
}

fn display_metadata(chunk: &Chunk) -> serde_json::Map<String, Value> {
    [
        "thumbnail",
        "published_at",
        "view_count",
        "like_count",
        "comment_count",
        "duration",
    ]
    .into_iter()
    .filter_map(|key| {
        chunk
            .metadata
            .get(key)
            .map(|v| (key.to_string(), v.clone()))
    })
    .collect()
}

// A video is the reconciliation unit. Never split one video across snapshots.
// Long individual chunks are rejected, never truncated or assigned invented times.
pub(crate) fn passages(
    chunks: &[Chunk],
    profile: &Profile,
    mut tokens: impl FnMut(&str) -> Result<usize>,
) -> Result<Vec<Passage>> {
    if chunks.is_empty() || chunks.len() > MAX_VIDEO_CHUNKS {
        return Err(Error::Input("invalid video size"));
    }
    let first = &chunks[0];
    let mut previous_start = 0;
    let mut ids = std::collections::HashSet::new();
    let mut bytes = 0;
    for (i, chunk) in chunks.iter().enumerate() {
        chunk.validate()?;
        bytes += serde_json::to_vec(chunk)?.len();
        if bytes > MAX_VIDEO_BYTES
            || chunk.video_id != first.video_id
            || chunk.title != first.title
            || chunk.youtuber_handle != first.youtuber_handle
            || chunk.youtuber_name != first.youtuber_name
            || display_metadata(chunk) != display_metadata(first)
            || !ids.insert(&chunk.id)
            || (i > 0 && chunk.t_start_ms < previous_start)
        {
            return Err(Error::Input(
                "video must have consistent metadata and chunks ordered by start timestamp",
            ));
        }
        if tokens(&document_prompt(&chunk.title, &chunk.text))? > MAX_TOKENS {
            return Err(Error::Input(
                "individual chunk exceeds token cap; split it upstream with original timestamps",
            ));
        }
        previous_start = chunk.t_start_ms;
    }
    let mut output = Vec::new();
    let mut start = 0;
    while start < chunks.len() {
        let mut end = start + 1;
        let mut text = chunks[start].text.clone();
        let mut range_end = chunks[start].t_end_ms;
        while end < chunks.len()
            && range_end.max(chunks[end].t_end_ms) - chunks[start].t_start_ms <= 60_000
        {
            let candidate = format!("{text}\n{}", chunks[end].text);
            if tokens(&document_prompt(&first.title, &candidate))? > MAX_TOKENS {
                break;
            }
            text = candidate;
            range_end = range_end.max(chunks[end].t_end_ms);
            end += 1;
        }
        // Preserve only supported display metadata; never let arbitrary input overwrite identity/profile fields.
        let metadata = display_metadata(first);
        let mut passage = Passage {
            id: String::new(),
            profile_id: profile.id(),
            source_revision: digest(&serde_json::to_vec(&chunks[start..end])?),
            text_digest: digest(text.as_bytes()),
            video_id: first.video_id.clone(),
            title: first.title.clone(),
            text,
            youtuber_handle: first.youtuber_handle.clone(),
            youtuber_name: first.youtuber_name.clone(),
            t_start_ms: chunks[start].t_start_ms,
            t_end_ms: range_end,
            metadata,
        };
        passage.id = Uuid::new_v5(&Uuid::NAMESPACE_URL, &serde_json::to_vec(&passage)?).to_string();
        output.push(passage);
        start = end;
    }
    Ok(output)
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchFilter {
    #[serde(default, deserialize_with = "deserialize_creator")]
    pub(crate) creator: Option<String>,
    pub(crate) video_id: Option<String>,
}
impl SearchFilter {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.video_id.as_deref().is_some_and(|v| !valid_identity(v))
            || self.creator.as_deref().is_some_and(|v| {
                !v.starts_with('@') || !valid_identity(&v[1..]) || v != v.to_lowercase()
            })
        {
            return Err(Error::Input("invalid creator or video filter"));
        }
        Ok(())
    }
    pub(crate) fn qdrant(&self, profile: &Profile) -> Value {
        let mut must = vec![json!({"key":"profile_id", "match":{"value":profile.id()}})];
        if let Some(creator) = &self.creator {
            must.push(json!({"key":"youtuber_handle", "match":{"value":creator}}));
        }
        if let Some(video_id) = &self.video_id {
            must.push(json!({"key":"video_id", "match":{"value":video_id}}));
        }
        json!({"must":must})
    }
}

pub(crate) fn normalize(mut vector: Vec<f32>, dimensions: EmbeddingDimensions) -> Result<Vec<f32>> {
    if vector.len() != 768 || vector.iter().any(|v| !v.is_finite()) {
        return Err(Error::Model);
    }
    vector.truncate(dimensions.size());
    let norm = vector
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm <= 1e-12 {
        return Err(Error::Model);
    }
    for v in &mut vector {
        *v = (f64::from(*v) / norm) as f32;
    }
    Ok(vector)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn chunk(id: &str, start: u32, end: u32) -> Chunk {
        serde_json::from_value(json!({"id":id,"video_id":"video","title":"Title","text":id,"youtuber_handle":"@creator","youtuber_name":"Creator","t_start_ms":start,"t_end_ms":end})).unwrap()
    }
    #[test]
    fn dimensions_profiles_and_vectors_are_locked() {
        assert!(EmbeddingDimensions::parse("768").is_err());
        let a = Profile {
            dimensions: EmbeddingDimensions::D128,
        };
        let b = Profile {
            dimensions: EmbeddingDimensions::D256,
        };
        assert_ne!(a.id(), b.id());
        let v = normalize(vec![1.; 768], a.dimensions).unwrap();
        assert_eq!(v.len(), 128);
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.).abs() < 1e-5);
        assert!(normalize(vec![f32::NAN; 768], a.dimensions).is_err());
        assert!(normalize(vec![0.; 768], a.dimensions).is_err());
        assert!(normalize(vec![1.; 128], a.dimensions).is_err());
    }
    #[test]
    fn passages_are_stable_bounded_and_preserve_time() {
        let p = Profile {
            dimensions: EmbeddingDimensions::default(),
        };
        let chunks = [
            chunk("a", 0, 30_000),
            chunk("b", 30_000, 60_000),
            chunk("c", 60_000, 61_000),
        ];
        let a = passages(&chunks, &p, |_| Ok(10)).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!((a[0].t_start_ms, a[0].t_end_ms), (0, 60_000));
        assert_eq!(a[0].text, "a\nb");
        assert_eq!(
            a[0].id,
            passages(&chunks[..2], &p, |_| Ok(10)).unwrap()[0].id
        );
        let mut changed = chunks.to_vec();
        changed[0].text = "changed".into();
        assert_ne!(a[0].id, passages(&changed, &p, |_| Ok(10)).unwrap()[0].id);
        assert!(passages(&[chunk("a", 0, 60_001)], &p, |_| Ok(10)).is_err());
        assert!(passages(&[chunk("a", 10, 20), chunk("b", 9, 20)], &p, |_| Ok(10)).is_err());
        assert_eq!(
            passages(&chunks, &p, |s| Ok(if s.contains('\n') { 513 } else { 10 }))
                .unwrap()
                .len(),
            3
        );
        assert!(passages(&chunks, &p, |_| Ok(513)).is_err());
    }
    #[test]
    fn accepts_overlapping_caption_ranges_from_the_existing_chunker() {
        use youtube_ingest::{Chunker, TranscriptSegment, VideoMetadata, Youtuber};
        let metadata = VideoMetadata {
            video_id: "video".into(),
            title: "Title".into(),
            youtuber: Youtuber {
                id: "creator".into(),
                handle: "@creator".into(),
                name: "Creator".into(),
                channel_id: "channel".into(),
            },
            duration_ms: None,
            thumbnail: None,
            published_at: None,
            upload_date: None,
        };
        let segments = [
            TranscriptSegment {
                start_ms: 0,
                end_ms: 9000,
                text: "First caption".into(),
            },
            TranscriptSegment {
                start_ms: 7000,
                end_ms: 15000,
                text: "Second caption".into(),
            },
        ];
        let source = Chunker::new(10).chunks(&metadata, &segments).unwrap();
        let chunks: Vec<Chunk> =
            serde_json::from_value(serde_json::to_value(source).unwrap()).unwrap();
        assert_eq!(chunks.len(), 2);
        let p = Profile {
            dimensions: EmbeddingDimensions::default(),
        };
        let combined = passages(&chunks, &p, |_| Ok(10)).unwrap();
        assert_eq!(combined[0].text, "First caption\nSecond caption");
        assert_eq!((combined[0].t_start_ms, combined[0].t_end_ms), (0, 15000));
        let split = passages(&chunks, &p, |s| Ok(if s.contains('\n') { 513 } else { 10 })).unwrap();
        assert_eq!((split[0].t_start_ms, split[0].t_end_ms), (0, 9000));
        assert_eq!((split[1].t_start_ms, split[1].t_end_ms), (7000, 15000));
        let nested = [chunk("outer", 0, 10000), chunk("inner", 5000, 6000)];
        assert_eq!(
            passages(&nested, &p, |_| Ok(10)).unwrap()[0].t_end_ms,
            10000
        );
    }
    #[test]
    fn existing_chunk_context_metadata_may_vary_per_chunk() {
        let profile = Profile {
            dimensions: EmbeddingDimensions::default(),
        };
        let mut a = chunk("a", 0, 10);
        let mut b = chunk("b", 10, 20);
        a.metadata.insert("sequence".into(), json!(0));
        b.metadata.insert("sequence".into(), json!(1));
        b.metadata.insert("prev_text".into(), json!("a"));
        assert_eq!(
            passages(&[a.clone(), b.clone()], &profile, |_| Ok(10))
                .unwrap()
                .len(),
            1
        );
        a.metadata
            .insert("published_at".into(), json!("2026-10-15T00:00:00Z"));
        assert!(passages(&[a, b], &profile, |_| Ok(10)).is_err());
    }
    #[test]
    fn import_and_direct_filters_share_case_insensitive_creator_identity() {
        let mut doc = serde_json::to_value(chunk("a", 0, 10)).unwrap();
        doc["youtuber_handle"] = json!("@aiDotEngineer");
        let chunk: Chunk = serde_json::from_value(doc).unwrap();
        chunk.validate().unwrap();
        assert_eq!(chunk.youtuber_handle, "@aidotengineer");
        let filter: SearchFilter =
            serde_json::from_value(json!({"creator":"@aiDotEngineer"})).unwrap();
        filter.validate().unwrap();
        assert_eq!(
            filter.creator.as_deref(),
            Some(chunk.youtuber_handle.as_str())
        );
    }
    #[test]
    fn filters_are_inside_retrieval_and_not_strings() {
        let f = SearchFilter {
            creator: Some("@creator".into()),
            video_id: Some("vid".into()),
        };
        f.validate().unwrap();
        let json = f.qdrant(&Profile {
            dimensions: EmbeddingDimensions::default(),
        });
        assert_eq!(json["must"][1]["match"]["value"], "@creator");
        assert_eq!(json["must"][2]["key"], "video_id");
        assert!(
            SearchFilter {
                creator: Some("@x || true".into()),
                video_id: None
            }
            .validate()
            .is_err()
        );
    }
}
