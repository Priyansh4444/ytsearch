mod domain;
mod model;
mod store;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use domain::*;
use model::{Gemma, Inference};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::File,
    io::{BufRead, BufReader, Read, Seek},
    path::Path,
    sync::Arc,
    time::Instant,
};
use store::Store;
use tokio::sync::Semaphore;

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("{0}")]
    Input(&'static str),
    #[error("invalid JSON")]
    Json(#[from] serde_json::Error),
    #[error("local snapshot I/O failed")]
    Io(#[from] std::io::Error),
    #[error("local model unavailable or inference failed")]
    Model,
    #[error("Qdrant request failed")]
    Store,
    #[error("collection semantic profile mismatch")]
    Profile,
    #[error("semantic worker busy")]
    Busy,
}
type Result<T> = std::result::Result<T, Error>;
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Input(_) | Self::Json(_) => StatusCode::BAD_REQUEST,
            Self::Busy => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, Json(json!({"error":self.to_string()}))).into_response()
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
async fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if !matches!(args.first().map(String::as_str), Some("serve" | "index"))
        || (args[0] == "serve" && args.len() != 1)
        || (args[0] == "index" && args.len() != 2)
    {
        return Err(Error::Input(
            "usage: youtube-semantic serve | index SNAPSHOT.jsonl (no rewipe supported)",
        ));
    }
    let profile = Profile {
        dimensions: EmbeddingDimensions::parse(
            &env::var("SEMANTIC_DIMENSIONS").unwrap_or_else(|_| "128".into()),
        )?,
    };
    let collection = env::var("SEMANTIC_COLLECTION")
        .map_err(|_| Error::Input("SEMANTIC_COLLECTION is required"))?;
    let key_env = if args[0] == "index" {
        "SEMANTIC_QDRANT_WRITE_KEY"
    } else {
        "SEMANTIC_QDRANT_READ_KEY"
    };
    let store = Store::new(
        &env::var("SEMANTIC_QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:16333".into()),
        collection,
        env::var(key_env).ok(),
        profile.clone(),
    )?;
    // ort-load-dynamic never downloads a runtime. Fail closed before its lazy global initialization.
    let runtime = env::var("ORT_DYLIB_PATH").map_err(|_| {
        Error::Input("ORT_DYLIB_PATH must name a local ONNX Runtime shared library")
    })?;
    if !Path::new(&runtime).is_file() {
        return Err(Error::Input("ONNX Runtime shared library missing"));
    }
    let model_dir = env::var("SEMANTIC_MODEL_DIR")
        .map_err(|_| Error::Input("SEMANTIC_MODEL_DIR is required"))?;
    let gemma = Gemma::load(Path::new(&model_dir))?;
    if args[0] == "index" {
        let count_tokens = gemma.counter();
        let _writer_lock = store.writer_lock()?;
        let receipt_path = format!("{}.manifest.json", args[1]);
        let receipt: SnapshotReceipt =
            serde_json::from_reader(File::open(receipt_path)?.take(64 * 1024))?;

        // Freeze bounded input first, so validation and writes read exactly the same snapshot.
        let mut source = File::open(&args[1])?.take(MAX_SNAPSHOT_BYTES + 1);
        let mut frozen = tempfile::tempfile()?;
        if std::io::copy(&mut source, &mut frozen)? > MAX_SNAPSHOT_BYTES {
            return Err(Error::Input("snapshot exceeds 64 MiB pilot cap"));
        }
        frozen.rewind()?;
        let mut reader = VideoReader::new(BufReader::new(frozen));
        let mut complete_videos = std::collections::BTreeMap::new();
        let mut videos = 0;
        while let Some(chunks) = reader.next_video()? {
            passages(&chunks, &profile, &count_tokens)?;
            complete_videos.insert(chunks[0].video_id.clone(), chunks.len());
            videos += 1;
            if videos > 128 {
                return Err(Error::Input("snapshot exceeds 128 video pilot cap"));
            }
        }
        if videos == 0 {
            return Err(Error::Input("empty snapshot"));
        }
        let mut frozen = reader.reader.into_inner();
        frozen.rewind()?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = frozen.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        receipt.validate(&format!("{:x}", hasher.finalize()), &complete_videos)?;
        frozen.rewind()?;
        // No network mutations occurred before complete snapshot preflight.
        store.ensure().await?;
        let inference = gemma.worker(profile.clone());
        let mut reader = VideoReader::new(BufReader::new(frozen));
        let mut written = 0;
        while let Some(chunks) = reader.next_video()? {
            let passages = passages(&chunks, &profile, &count_tokens)?;
            written += replace_video(&store, &inference, &passages).await?;
        }
        println!(
            "{}",
            json!({"videos":videos,"embedded_passages":written,"profile_id":profile.id()})
        );
        Ok(())
    } else {
        store.ready().await?;
        let inference = gemma.worker(profile);
        inference.embed(query_prompt("readiness")).await?;
        let state = Arc::new(Service {
            store,
            inference,
            admission: Arc::new(Semaphore::new(model::QUEUE_DEPTH)),
        });
        let bind = env::var("SEMANTIC_BIND").unwrap_or_else(|_| "127.0.0.1:8790".into());
        let bind: std::net::SocketAddr = bind
            .parse()
            .map_err(|_| Error::Input("invalid semantic bind address"))?;
        if !bind.ip().is_loopback() {
            return Err(Error::Input("pilot service must bind loopback"));
        }
        let listener = tokio::net::TcpListener::bind(bind).await?;
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    }
}

// The exporter writes this receipt only after completing every declared video's snapshot.
// Bare JSONL cannot distinguish a deliberately shorter transcript from a truncated export.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotReceipt {
    schema: u8,
    sha256: String,
    complete_videos: std::collections::BTreeMap<String, usize>,
}
impl SnapshotReceipt {
    fn validate(
        &self,
        sha256: &str,
        complete_videos: &std::collections::BTreeMap<String, usize>,
    ) -> Result<()> {
        if self.schema != 1 || self.sha256 != sha256 || &self.complete_videos != complete_videos {
            return Err(Error::Input(
                "snapshot receipt mismatch: regenerate a complete export before importing",
            ));
        }
        Ok(())
    }
}

struct VideoReader<R> {
    reader: R,
    pending: Option<Chunk>,
    previous_video: Option<String>,
}
impl<R: BufRead> VideoReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            pending: None,
            previous_video: None,
        }
    }
    fn line(&mut self) -> Result<Option<Chunk>> {
        let mut bytes = Vec::new();
        let count = self
            .reader
            .by_ref()
            .take((MAX_LINE_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)?;
        if count == 0 {
            return Ok(None);
        }
        if count > MAX_LINE_BYTES {
            return Err(Error::Input("JSONL line exceeds 64 KiB"));
        }
        Ok(Some(serde_json::from_slice(&bytes)?))
    }
    fn next_video(&mut self) -> Result<Option<Vec<Chunk>>> {
        let first = match self.pending.take() {
            Some(v) => Some(v),
            None => self.line()?,
        };
        let Some(first) = first else { return Ok(None) };
        if self
            .previous_video
            .as_deref()
            .is_some_and(|v| v >= first.video_id.as_str())
        {
            return Err(Error::Input(
                "snapshot must be sorted by video_id then timestamp; each video must be complete",
            ));
        }
        let video = first.video_id.clone();
        let mut bytes = serde_json::to_vec(&first)?.len();
        let mut output = vec![first];
        while let Some(chunk) = self.line()? {
            if chunk.video_id != video {
                self.pending = Some(chunk);
                break;
            }
            bytes += serde_json::to_vec(&chunk)?.len();
            if output.len() >= MAX_VIDEO_CHUNKS || bytes > MAX_VIDEO_BYTES {
                return Err(Error::Input("video exceeds pilot cap"));
            }
            output.push(chunk);
        }
        self.previous_video = Some(video);
        Ok(Some(output))
    }
}

// Separate the reconciliation decision from effects so retries have one explicit state transition.
struct Replacement {
    missing: Vec<usize>,
    stale_payload: Vec<usize>,
    obsolete: Vec<String>,
}
fn replacement(
    existing: &std::collections::HashMap<String, Value>,
    desired: &[Passage],
) -> Result<Replacement> {
    let mut remaining = existing.clone();
    let mut missing = Vec::new();
    let mut stale_payload = Vec::new();
    for (i, p) in desired.iter().enumerate() {
        match remaining.remove(&p.id) {
            Some(payload) if payload == serde_json::to_value(p)? => (),
            // The ID pins the embedded content, so only display payload differs.
            Some(_) => stale_payload.push(i),
            None => missing.push(i),
        }
    }
    Ok(Replacement {
        missing,
        stale_payload,
        obsolete: remaining.into_keys().collect(),
    })
}
async fn replace_video(store: &Store, inference: &Inference, desired: &[Passage]) -> Result<usize> {
    let first = desired
        .first()
        .ok_or(Error::Input("empty video replacement unsupported"))?;
    let existing = store.existing(&first.video_id).await?;
    let plan = replacement(&existing, desired)?;
    for &i in &plan.missing {
        let passage = &desired[i];
        let vector = inference
            .embed(document_prompt(&passage.title, &passage.text))
            .await?;
        store.upsert(passage, vector).await?;
    }
    for &i in &plan.stale_payload {
        store.overwrite_payload(&desired[i]).await?;
    }
    // This is reached only once EVERY replacement point has a successful durable acknowledgment.
    // Failure/retry leaves old passages reachable; rerun skips completed content IDs and converges.
    store.remove_obsolete(&plan.obsolete).await?;
    Ok(plan.missing.len())
}

struct Service {
    store: Store,
    inference: Inference,
    admission: Arc<Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchRequest {
    q: String,
    #[serde(default)]
    filter: SearchFilter,
    limit: usize,
}
fn router(state: Arc<Service>) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Json(json!({"service":"youtube_semantic","pilot":true})) }),
        )
        .route("/health", get(|| async { StatusCode::OK }))
        .route("/ready", get(ready))
        .route("/search", post(search))
        .layer(DefaultBodyLimit::max(8 * 1024))
        .with_state(state)
}
async fn ready(State(state): State<Arc<Service>>) -> Result<StatusCode> {
    if !state.inference.ready() {
        return Err(Error::Model);
    }
    state.store.ready().await?;
    Ok(StatusCode::OK)
}
async fn search(
    State(state): State<Arc<Service>>,
    input: std::result::Result<Json<SearchRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<Value>> {
    let Json(input) = input.map_err(|_| Error::Input("invalid semantic search request"))?;
    let _permit = state
        .admission
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Busy)?;
    let q = input.q.trim();
    if q.is_empty() || q.len() > 4096 || !(1..=100).contains(&input.limit) {
        return Err(Error::Input("invalid query or limit (1..100)"));
    }
    input.filter.validate()?;
    let start = Instant::now();
    let vector = state.inference.embed(query_prompt(q)).await?;
    let embedding_ms = start.elapsed().as_millis() as u64;
    let hits = state
        .store
        .search(vector, &input.filter, input.limit)
        .await?;
    Ok(Json(
        json!({"hits":hits,"mode":"semantic","profile_id":state.store.profile.id(),
        "timings":{"embedding_ms":embedding_ms,"total_ms":start.elapsed().as_millis() as u64}}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_cannot_return_to_prior_video_or_hide_a_partial_line() {
        let a = json!({"id":"a","video_id":"b","title":"T","text":"t","youtuber_handle":"@x","youtuber_name":"X","t_start_ms":0,"t_end_ms":1});
        let mut b = a.clone();
        b["video_id"] = json!("a");
        let text = format!("{a}\n{b}\n");
        let mut reader = VideoReader::new(text.as_bytes());
        assert!(reader.next_video().unwrap().is_some());
        assert!(reader.next_video().is_err());
        assert!(VideoReader::new(&b"{partial"[..]).next_video().is_err());
        let huge = vec![b'x'; MAX_LINE_BYTES + 1];
        assert!(VideoReader::new(&huge[..]).next_video().is_err());
    }
    #[test]
    fn snapshot_receipt_rejects_truncation_even_at_a_valid_line_boundary() {
        let videos = std::collections::BTreeMap::from([("v".into(), 3)]);
        let receipt = SnapshotReceipt {
            schema: 1,
            sha256: digest(b"first\nsecond\nthird\n"),
            complete_videos: videos.clone(),
        };
        receipt
            .validate(&digest(b"first\nsecond\nthird\n"), &videos)
            .unwrap();
        assert!(
            receipt
                .validate(&digest(b"first\nsecond\n"), &videos)
                .is_err()
        );
        assert!(
            receipt
                .validate(
                    &receipt.sha256,
                    &std::collections::BTreeMap::from([("v".into(), 2)])
                )
                .is_err()
        );
    }
    #[test]
    fn retry_and_shrinking_video_reconcile_content_ids() {
        let chunk:Chunk=serde_json::from_value(json!({"id":"a","video_id":"v","title":"T","text":"t","youtuber_handle":"@x","youtuber_name":"X","t_start_ms":0,"t_end_ms":1})).unwrap();
        let desired = passages(
            &[chunk],
            &Profile {
                dimensions: EmbeddingDimensions::default(),
            },
            |_| Ok(5),
        )
        .unwrap();
        let mut existing = std::collections::HashMap::from([("obsolete".into(), json!({}))]);
        let plan = replacement(&existing, &desired).unwrap();
        assert_eq!(plan.missing, vec![0]);
        assert_eq!(plan.obsolete, vec!["obsolete"]);
        // Crash after upsert but before delete: retry skips inference and still removes obsolete IDs.
        existing.insert(
            desired[0].id.clone(),
            serde_json::to_value(&desired[0]).unwrap(),
        );
        let plan = replacement(&existing, &desired).unwrap();
        assert!(plan.missing.is_empty());
        assert_eq!(plan.obsolete, vec!["obsolete"]);
        existing.remove("obsolete");
        assert!(
            replacement(&existing, &desired)
                .unwrap()
                .obsolete
                .is_empty()
        );
        // Same content ID, different payload (e.g. a new view count): overwrite, never re-embed.
        existing.insert(desired[0].id.clone(), json!({"view_count":1}));
        let plan = replacement(&existing, &desired).unwrap();
        assert!(plan.missing.is_empty());
        assert_eq!(plan.stale_payload, vec![0]);
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use axum::routing::put;
    use std::collections::HashMap;
    #[derive(Default)]
    struct Fake {
        points: HashMap<String, Value>,
        writes: usize,
        payload_writes: usize,
        deletes: usize,
        fail_at: usize,
    }
    type FakeState = Arc<std::sync::Mutex<Fake>>;
    async fn scroll(State(state): State<FakeState>) -> Json<Value> {
        let state = state.lock().unwrap();
        Json(
            json!({"status":"ok","result":{"points":state.points.iter().map(|(id,payload)|json!({"id":id,"payload":payload})).collect::<Vec<_>>(),"next_page_offset":null}}),
        )
    }
    async fn upsert(State(state): State<FakeState>, Json(body): Json<Value>) -> Response {
        let mut state = state.lock().unwrap();
        state.writes += 1;
        if state.writes == state.fail_at {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let point = &body["points"][0];
        assert_eq!(point["vector"].as_array().unwrap().len(), 128);
        state.points.insert(
            point["id"].as_str().unwrap().into(),
            point["payload"].clone(),
        );
        Json(json!({"status":"ok","result":{"status":"completed"}})).into_response()
    }
    async fn overwrite(State(state): State<FakeState>, Json(body): Json<Value>) -> Json<Value> {
        let mut state = state.lock().unwrap();
        state.payload_writes += 1;
        for id in body["points"].as_array().unwrap() {
            state
                .points
                .insert(id.as_str().unwrap().into(), body["payload"].clone());
        }
        Json(json!({"status":"ok","result":{"status":"completed"}}))
    }
    async fn delete(State(state): State<FakeState>, Json(body): Json<Value>) -> Json<Value> {
        let mut state = state.lock().unwrap();
        state.deletes += 1;
        for id in body["points"].as_array().unwrap() {
            state.points.remove(id.as_str().unwrap());
        }
        Json(json!({"status":"ok","result":{"status":"completed"}}))
    }
    #[tokio::test]
    async fn partial_transport_failure_never_deletes_then_retry_skips_completed_points() {
        let state = Arc::new(std::sync::Mutex::new(Fake {
            points: HashMap::from([("old".into(), json!({}))]),
            fail_at: 2,
            ..Default::default()
        }));
        let app = Router::new()
            .route("/collections/{name}/points/scroll", post(scroll))
            .route("/collections/{name}/points", put(upsert))
            .route("/collections/{name}/points/delete", post(delete))
            .route("/collections/{name}/points/payload", put(overwrite))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let profile = Profile {
            dimensions: EmbeddingDimensions::default(),
        };
        let store = Store::new(
            &base,
            "ytsearch_semantic_pilot_transport".into(),
            None,
            profile.clone(),
        )
        .unwrap();
        let a:Chunk=serde_json::from_value(json!({"id":"a","video_id":"v","title":"T","text":"first","youtuber_handle":"@x","youtuber_name":"X","t_start_ms":0,"t_end_ms":1000})).unwrap();
        let mut b = a.clone();
        b.id = "b".into();
        b.text = "second".into();
        b.t_start_ms = 61_000;
        b.t_end_ms = 62_000;
        let desired = passages(&[a, b], &profile, |_| Ok(10)).unwrap();
        let inference = Inference::fake();
        assert!(replace_video(&store, &inference, &desired).await.is_err());
        {
            let state = state.lock().unwrap();
            assert_eq!(state.deletes, 0);
            assert!(state.points.contains_key("old"));
            assert_eq!(state.points.len(), 2);
        }
        assert_eq!(
            replace_video(&store, &inference, &desired).await.unwrap(),
            1
        );
        {
            let state = state.lock().unwrap();
            assert!(!state.points.contains_key("old"));
            assert_eq!(state.points.len(), 2);
            assert_eq!(state.writes, 3);
        }
        assert_eq!(
            replace_video(&store, &inference, &desired).await.unwrap(),
            0
        );
        assert_eq!(
            replace_video(&store, &inference, &desired[..1])
                .await
                .unwrap(),
            0
        );
        assert_eq!(state.lock().unwrap().points.len(), 1);
        // A daily metadata refresh rewrites the payload in place: no embedding, no new point.
        let mut refreshed = desired[..1].to_vec();
        refreshed[0]
            .metadata
            .insert("view_count".into(), json!(1_000_001));
        assert_eq!(
            replace_video(&store, &inference, &refreshed).await.unwrap(),
            0
        );
        {
            let state = state.lock().unwrap();
            assert_eq!((state.writes, state.payload_writes), (3, 1));
            assert_eq!(state.points[&refreshed[0].id]["view_count"], 1_000_001);
        }
        server.abort();
    }
}
