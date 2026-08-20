use super::*;
use crate::{GenerationBuilder, PublisherPaths, RawSearchEngine, build_index};
use axum::body::Body;
use flate2::{Compression, write::GzEncoder};
use serde::Serialize;
use serde_json::json;
use std::io::Write;
use tempfile::TempDir;
use youtube_index_store::{ChannelDocument, IndexOperationState, VideoReplacement};
use youtube_ingest::TranscriptChunk;
use youtube_search_service::SearchParams;

struct Fixture {
    root: TempDir,
    engine: Arc<ReloadableEngine>,
    store: Arc<MutationStore>,
    publisher: Arc<Publisher>,
    publication: RemotePublication,
}

fn chunk(video_id: &str, token: &str) -> TranscriptChunk {
    serde_json::from_value(json!({
        "id": format!("{video_id}-0"),
        "youtuber_id": "creator",
        "youtuber_handle": "@creator",
        "youtuber_name": "Creator",
        "channel_id": "UCcreator",
        "video_id": video_id,
        "title": "Video",
        "text": token,
        "sequence": 0,
        "t_start_ms": 0,
        "t_end_ms": 1000,
        "uploader_id": "creator"
    }))
    .unwrap()
}

fn channel() -> ChannelDocument {
    ChannelDocument {
        id: "creator".to_string(),
        handle: "@creator".to_string(),
        name: "Creator".to_string(),
        channel_id: "UCcreator".to_string(),
        uploads_playlist: None,
        public_video_count: 1,
        indexed_video_count: 1,
        indexed_chunk_count: 1,
        avatar_url: None,
    }
}

fn write_gzip_lines(path: &Path, values: &[impl Serialize]) {
    let mut output = GzEncoder::new(File::create(path).unwrap(), Compression::fast());
    for value in values {
        serde_json::to_writer(&mut output, value).unwrap();
        output.write_all(b"\n").unwrap();
    }
    output.finish().unwrap();
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let chunks_gzip = root.path().join("chunks.jsonl.gz");
    let channels_gzip = root.path().join("channels.jsonl.gz");
    write_gzip_lines(&chunks_gzip, &[chunk("base-video", "basetoken")]);
    write_gzip_lines(&channels_gzip, &[channel()]);
    let chunks = root.path().join("chunks.jsonl");
    let channels = root.path().join("channels.jsonl");
    writeln!(
        File::create(&chunks).unwrap(),
        "{}",
        serde_json::to_string(&chunk("base-video", "basetoken")).unwrap()
    )
    .unwrap();
    writeln!(
        File::create(&channels).unwrap(),
        "{}",
        serde_json::to_string(&channel()).unwrap()
    )
    .unwrap();
    let base_index = root.path().join("base-index");
    build_index(&chunks, &channels, &base_index).unwrap();
    let state = root.path().join("state");
    let publisher = Arc::new(Publisher::new(PublisherPaths {
        base_chunks_gzip: chunks_gzip,
        base_channels_gzip: channels_gzip,
        base_index: base_index.clone(),
        state_directory: state.clone(),
    }));
    let store = Arc::new(MutationStore::open(&state.join("mutations.sqlite3")).unwrap());
    let engine = Arc::new(ReloadableEngine::new(
        RawSearchEngine::open(&base_index).unwrap(),
        0,
    ));
    let publication = RemotePublication::new(
        Arc::clone(&engine),
        Arc::clone(&store),
        Arc::clone(&publisher),
    )
    .unwrap();
    Fixture {
        root,
        engine,
        store,
        publisher,
        publication,
    }
}

fn replace(store: &MutationStore, video_id: &str, token: &str, key: &str) -> u64 {
    store
        .replace_video(
            VideoReplacement::new(video_id, &[chunk(video_id, token)]).unwrap(),
            key,
        )
        .unwrap()
        .sequence
        .unwrap()
}

fn detached_for_job(fixture: &Fixture, job: &RemoteJobResponse) -> (PathBuf, GenerationManifest) {
    let snapshot = fixture
        .publication
        .job_directory(&job.job_id)
        .unwrap()
        .join(SNAPSHOT_FILE);
    let snapshot_store = MutationStore::open_snapshot_read_only(&snapshot).unwrap();
    let output = fixture.root.path().join(format!("built-{}", job.job_id));
    GenerationBuilder::new(
        fixture.root.path().join("chunks.jsonl.gz"),
        fixture.root.path().join("channels.jsonl.gz"),
    )
    .build(&snapshot_store, &job.compatibility, &output)
    .unwrap();
    let manifest =
        serde_json::from_slice(&fs::read(output.join(GENERATION_MANIFEST)).unwrap()).unwrap();
    (output, manifest)
}

async fn accept(
    publication: &RemotePublication,
    job: &RemoteJobResponse,
    manifest: GenerationManifest,
) {
    publication
        .accept_manifest(
            &job.job_id,
            GenerationUploadRequest {
                target_sequence: job.target_sequence,
                compatibility: job.compatibility.clone(),
                manifest,
            },
        )
        .await
        .unwrap();
}

async fn upload(
    publication: &RemotePublication,
    job: &RemoteJobResponse,
    generation: &Path,
    name: &str,
) {
    let bytes = fs::read(generation.join(name)).unwrap();
    publication
        .upload_file(
            &job.job_id,
            name,
            Some(bytes.len() as u64),
            Body::from(bytes),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn corrupted_upload_is_removed_and_restart_preserves_n_plus_one() {
    let fixture = fixture();
    let sequence_n = replace(&fixture.store, "video-n", "snapshotonly", "operation-n");
    let job = fixture.publication.prepare().await.unwrap();
    assert_eq!(job.target_sequence, sequence_n);
    let sequence_n_plus_one = replace(
        &fixture.store,
        "video-n-plus-one",
        "nextsnapshot",
        "operation-n-plus-one",
    );
    let (generation, manifest) = detached_for_job(&fixture, &job);
    accept(&fixture.publication, &job, manifest).await;

    let first = RAW_GENERATION_FILES[0];
    let mut corrupt = fs::read(generation.join(first)).unwrap();
    corrupt[0] ^= 0xff;
    assert!(matches!(
        fixture
            .publication
            .upload_file(
                &job.job_id,
                first,
                Some(corrupt.len() as u64),
                Body::from(corrupt)
            )
            .await,
        Err(IndexStoreError::InvalidInput(_))
    ));
    let incoming = fixture
        .publication
        .job_directory(&job.job_id)
        .unwrap()
        .join(GENERATION_DIRECTORY);
    assert!(!incoming.join(first).exists());
    assert!(!incoming.join(format!(".{first}.part")).exists());

    upload(&fixture.publication, &job, &generation, first).await;
    for name in RAW_GENERATION_FILES.into_iter().skip(1).take(7) {
        upload(&fixture.publication, &job, &generation, name).await;
    }
    assert!(matches!(
        fixture.publication.activate(&job.job_id).await,
        Err(IndexStoreError::Conflict(_))
    ));

    let restarted = RemotePublication::new(
        Arc::clone(&fixture.engine),
        Arc::clone(&fixture.store),
        Arc::clone(&fixture.publisher),
    )
    .unwrap();
    let persisted = restarted.status(&job.job_id).await.unwrap();
    assert_eq!(persisted.completed_files.len(), 8);
    upload(&restarted, &job, &generation, RAW_GENERATION_FILES[8]).await;
    let activated = restarted.activate(&job.job_id).await.unwrap();
    assert_eq!(activated.status, RemoteJobStatus::Activated);
    assert_eq!(fixture.engine.sequence(), sequence_n);
    assert_eq!(
        fixture.store.operation("operation-n").unwrap().state,
        IndexOperationState::Published
    );
    assert_eq!(
        fixture
            .store
            .operation("operation-n-plus-one")
            .unwrap()
            .state,
        IndexOperationState::Durable
    );
    assert_eq!(fixture.store.last_sequence().unwrap(), sequence_n_plus_one);
    let active = fixture.engine.active().unwrap();
    assert_eq!(found(&active, "snapshotonly"), 1);
    assert_eq!(found(&active, "nextsnapshot"), 0);
}

#[tokio::test]
async fn compatibility_manifest_and_names_fail_closed_and_are_idempotent() {
    let fixture = fixture();
    replace(&fixture.store, "video-n", "snapshotonly", "operation-n");
    let job = fixture.publication.prepare().await.unwrap();
    let same_job = fixture.publication.prepare().await.unwrap();
    assert_eq!(same_job.job_id, job.job_id);
    let (generation, manifest) = detached_for_job(&fixture, &job);

    let mut wrong_compatibility = job.compatibility.clone();
    wrong_compatibility.generation_format.push_str("-wrong");
    assert!(matches!(
        fixture
            .publication
            .accept_manifest(
                &job.job_id,
                GenerationUploadRequest {
                    target_sequence: job.target_sequence,
                    compatibility: wrong_compatibility,
                    manifest: manifest.clone(),
                }
            )
            .await,
        Err(IndexStoreError::Conflict(_))
    ));

    let mut extra_file = manifest.clone();
    extra_file.files.insert(
        "../escape".to_string(),
        GenerationFile {
            bytes: 1,
            blake3: blake3::hash(b"x").to_hex().to_string(),
        },
    );
    assert!(matches!(
        fixture
            .publication
            .accept_manifest(
                &job.job_id,
                GenerationUploadRequest {
                    target_sequence: job.target_sequence,
                    compatibility: job.compatibility.clone(),
                    manifest: extra_file,
                }
            )
            .await,
        Err(IndexStoreError::InvalidInput(_))
    ));

    accept(&fixture.publication, &job, manifest.clone()).await;
    accept(&fixture.publication, &job, manifest.clone()).await;
    assert!(matches!(
        fixture
            .publication
            .upload_file(&job.job_id, "../postings.bin", None, Body::from("x"))
            .await,
        Err(IndexStoreError::InvalidInput(_))
    ));
    upload(
        &fixture.publication,
        &job,
        &generation,
        RAW_GENERATION_FILES[0],
    )
    .await;
    upload(
        &fixture.publication,
        &job,
        &generation,
        RAW_GENERATION_FILES[0],
    )
    .await;
    let mut different = manifest;
    different.documents = different.documents.saturating_add(1);
    assert!(matches!(
        fixture
            .publication
            .accept_manifest(
                &job.job_id,
                GenerationUploadRequest {
                    target_sequence: job.target_sequence,
                    compatibility: job.compatibility,
                    manifest: different,
                }
            )
            .await,
        Err(IndexStoreError::Conflict(_))
    ));
}

fn found(engine: &RawSearchEngine, query: &str) -> u64 {
    engine
        .search(&SearchParams {
            q: query.to_string(),
            ..SearchParams::default()
        })
        .unwrap()["found"]
        .as_u64()
        .unwrap()
}
