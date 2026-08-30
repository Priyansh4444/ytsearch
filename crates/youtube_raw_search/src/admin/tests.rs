use super::*;
use crate::remote_publication::{
    GenerationManifest, GenerationUploadRequest, RAW_GENERATION_FILES,
};
use crate::{GenerationBuilder, PublisherPaths, RawSearchEngine, build_index};
use axum::extract::{Path, State};
use flate2::{Compression, write::GzEncoder};
use serde::Serialize;
use std::{fs::File, io::Write};
use youtube_search_service::{SearchBackend, SearchParams};

const KEY: &str = "test-admin-key-24-bytes-long";

fn chunk(video_id: &str, token: &str) -> TranscriptChunk {
    chunk_for_channel(video_id, token, "creator")
}

fn chunk_for_channel(video_id: &str, token: &str, youtuber_id: &str) -> TranscriptChunk {
    serde_json::from_value(serde_json::json!({
        "id": format!("{video_id}-0"),
        "youtuber_id": youtuber_id,
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

fn gzip_json_lines<T: Serialize>(path: &std::path::Path, values: &[T]) {
    let mut output = GzEncoder::new(File::create(path).unwrap(), Compression::fast());
    for value in values {
        serde_json::to_writer(&mut output, value).unwrap();
        output.write_all(b"\n").unwrap();
    }
    output.finish().unwrap();
}

fn setup() -> (tempfile::TempDir, AdminState, Arc<Publisher>) {
    let root = tempfile::tempdir().unwrap();
    let base_chunks_gzip = root.path().join("chunks.jsonl.gz");
    let base_channels_gzip = root.path().join("channels.jsonl.gz");
    gzip_json_lines(&base_chunks_gzip, &[chunk("old-video", "legacytoken")]);
    gzip_json_lines(&base_channels_gzip, &[channel()]);

    let plain_chunks = root.path().join("chunks.jsonl");
    let plain_channels = root.path().join("channels.jsonl");
    writeln!(
        File::create(&plain_chunks).unwrap(),
        "{}",
        serde_json::to_string(&chunk("old-video", "legacytoken")).unwrap()
    )
    .unwrap();
    writeln!(
        File::create(&plain_channels).unwrap(),
        "{}",
        serde_json::to_string(&channel()).unwrap()
    )
    .unwrap();
    let base_index = root.path().join("base-index");
    build_index(&plain_chunks, &plain_channels, &base_index).unwrap();

    let state_directory = root.path().join("state");
    let publisher = Arc::new(Publisher::new(PublisherPaths {
        base_chunks_gzip,
        base_channels_gzip,
        base_index: base_index.clone(),
        state_directory: state_directory.clone(),
    }));
    let store = Arc::new(MutationStore::open(&state_directory.join("mutations.sqlite3")).unwrap());
    let engine = Arc::new(ReloadableEngine::new(
        RawSearchEngine::open(&base_index).unwrap(),
        0,
    ));
    let state = AdminState::new(engine, store, Arc::clone(&publisher), KEY.to_string()).unwrap();
    (root, state, publisher)
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ADMIN_KEY_HEADER, KEY.parse().unwrap());
    headers
}

async fn publish_remotely(root: &std::path::Path, state: &AdminState) {
    let job = state.remote_publication.prepare().await.unwrap();
    let mut download = state
        .remote_publication
        .open_snapshot(&job.job_id)
        .await
        .unwrap();
    let snapshot_path = root.join("off-reader-snapshot.sqlite3");
    let mut snapshot_output = tokio::fs::File::create(&snapshot_path).await.unwrap();
    tokio::io::copy(&mut download.file, &mut snapshot_output)
        .await
        .unwrap();
    snapshot_output.sync_all().await.unwrap();
    drop(snapshot_output);
    let snapshot_store = MutationStore::open_snapshot_read_only(&snapshot_path).unwrap();
    let output = root.join("off-reader-generation");
    GenerationBuilder::new(root.join("chunks.jsonl.gz"), root.join("channels.jsonl.gz"))
        .build(&snapshot_store, &job.compatibility, &output)
        .unwrap();
    let manifest: GenerationManifest =
        serde_json::from_slice(&std::fs::read(output.join("manifest.json")).unwrap()).unwrap();
    state
        .remote_publication
        .accept_manifest(
            &job.job_id,
            GenerationUploadRequest {
                target_sequence: job.target_sequence,
                compatibility: job.compatibility,
                manifest,
            },
        )
        .await
        .unwrap();
    for name in RAW_GENERATION_FILES {
        let bytes = std::fs::read(output.join(name)).unwrap();
        state
            .remote_publication
            .upload_file(
                &job.job_id,
                name,
                Some(bytes.len() as u64),
                Body::from(bytes),
            )
            .await
            .unwrap();
    }
    state
        .remote_publication
        .activate(&job.job_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn video_mutation_is_durable_then_atomically_published() {
    let (root, state, _publisher) = setup();
    let (_, Json(operation)) = replace_video(
        State(state.clone()),
        Path("new-video".to_string()),
        headers(),
        Json(ReplaceVideoRequest {
            publication_api_version: RAW_PUBLICATION_API_VERSION,
            idempotency_key: "replace-new-video".to_string(),
            chunks: vec![chunk("new-video", "currenttoken")],
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        operation.state,
        youtube_index_store::IndexOperationState::Durable
    );
    assert_eq!(state.engine.sequence(), 0);
    let Json(count) = video_chunk_count(
        State(state.clone()),
        Path("new-video".to_string()),
        headers(),
    )
    .await
    .unwrap();
    assert_eq!(count["count"], 1);
    let Json(stats) = channel_stats(State(state.clone()), Path("creator".to_string()), headers())
        .await
        .unwrap();
    assert_eq!(stats.indexed_videos, 2);
    assert_eq!(stats.indexed_chunks, 2);

    publish_remotely(root.path(), &state).await;

    assert_eq!(state.engine.sequence(), operation.sequence.unwrap());
    assert_eq!(
        state.store.operation("replace-new-video").unwrap().state,
        youtube_index_store::IndexOperationState::Published
    );
    let result = state
        .engine
        .search(&SearchParams {
            q: "currenttoken".to_string(),
            ..SearchParams::default()
        })
        .unwrap();
    assert_eq!(result["found"], 1);
}

#[tokio::test]
async fn active_channel_read_does_not_report_an_unpublished_overlay() {
    let (root, state, _publisher) = setup();
    let mut replacement = channel();
    replacement.name = "Replacement creator".to_string();
    let _ = upsert_channel(
        State(state.clone()),
        Path("creator".to_string()),
        headers(),
        Json(UpsertChannelRequest {
            publication_api_version: RAW_PUBLICATION_API_VERSION,
            idempotency_key: "replace-creator-metadata".to_string(),
            document: replacement.clone(),
        }),
    )
    .await
    .unwrap();

    let Json(merged) = super::channel(State(state.clone()), Path("creator".to_string()), headers())
        .await
        .unwrap();
    let Json(active_before) =
        super::active_channel(State(state.clone()), Path("creator".to_string()), headers())
            .await
            .unwrap();
    assert_eq!(merged, Some(replacement.clone()));
    assert_eq!(active_before, Some(channel()));

    publish_remotely(root.path(), &state).await;

    let Json(active_after) =
        super::active_channel(State(state), Path("creator".to_string()), headers())
            .await
            .unwrap();
    assert_eq!(active_after, Some(replacement));
}

#[tokio::test]
async fn handle_form_mutations_keep_canonical_channel_counts_and_scope() {
    let (root, state, _publisher) = setup();
    let _ = replace_video(
        State(state.clone()),
        Path("handle-video".to_string()),
        headers(),
        Json(ReplaceVideoRequest {
            publication_api_version: RAW_PUBLICATION_API_VERSION,
            idempotency_key: "replace-handle-video".to_string(),
            chunks: vec![chunk_for_channel("handle-video", "handletoken", "@Creator")],
        }),
    )
    .await
    .unwrap();

    let Json(pending) = channel_stats(State(state.clone()), Path("creator".to_string()), headers())
        .await
        .unwrap();
    assert_eq!(pending.indexed_videos, 2);
    assert_eq!(pending.indexed_chunks, 2);

    publish_remotely(root.path(), &state).await;

    let published = state.engine.active().unwrap().channel_stats("creator");
    assert_eq!(published.indexed_videos, 2);
    assert_eq!(published.indexed_chunks, 2);
}

#[tokio::test]
async fn channel_deletion_scope_controls_transcript_retention() {
    for (cascade, expected_hits) in [(false, 1), (true, 0)] {
        let (root, state, _publisher) = setup();
        let _operation = delete_channel(
            State(state.clone()),
            Path("creator".to_string()),
            headers(),
            Query(DeleteChannelRequest {
                publication_api_version: RAW_PUBLICATION_API_VERSION,
                idempotency_key: format!("delete-creator-{cascade}"),
                cascade,
            }),
        )
        .await
        .unwrap();

        publish_remotely(root.path(), &state).await;

        assert!(
            state
                .engine
                .active()
                .unwrap()
                .channel_document("creator")
                .unwrap()
                .is_none()
        );
        let result = state
            .engine
            .search(&SearchParams {
                q: "legacytoken".to_string(),
                ..SearchParams::default()
            })
            .unwrap();
        assert_eq!(result["found"], expected_hits);
    }
}

#[tokio::test]
async fn incompatible_mutation_protocol_is_rejected_before_durable_write() {
    let (_root, state, _publisher) = setup();
    let before = state.store.last_sequence().unwrap();
    let error = replace_video(
        State(state.clone()),
        Path("blocked-video".to_string()),
        headers(),
        Json(ReplaceVideoRequest {
            publication_api_version: RAW_PUBLICATION_API_VERSION + 1,
            idempotency_key: "replace-blocked-video".to_string(),
            chunks: vec![chunk("blocked-video", "blockedtoken")],
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(error.into_response().status(), StatusCode::CONFLICT);
    assert_eq!(state.store.last_sequence().unwrap(), before);
    assert!(matches!(
        state.store.operation("replace-blocked-video"),
        Err(IndexStoreError::InvalidInput(_))
    ));
}

#[tokio::test]
async fn admin_routes_reject_missing_keys_before_accessing_state() {
    let (_root, state, _publisher) = setup();
    let error = readiness(State(state), HeaderMap::new()).await.unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn remote_publication_and_legacy_publish_fail_closed_without_authority_or_worker() {
    let (_root, state, _publisher) = setup();
    let unauthorized = prepare_publication(State(state.clone()), HeaderMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        unauthorized.into_response().status(),
        StatusCode::UNAUTHORIZED
    );

    let legacy = publish(State(state), headers()).await.unwrap_err();
    assert_eq!(legacy.into_response().status(), StatusCode::CONFLICT);
}

#[test]
fn admin_keys_have_a_minimum_strength_and_constant_time_comparison() {
    let (_root, state, publisher) = setup();
    assert!(constant_time_eq(KEY, KEY));
    assert!(!constant_time_eq(KEY, "test-admin-key-24-bytes-lonf"));
    assert!(!constant_time_eq(KEY, "short"));
    assert!(matches!(
        AdminState::new(
            Arc::clone(&state.engine),
            Arc::clone(&state.store),
            publisher,
            "too-short".to_string()
        ),
        Err(IndexStoreError::InvalidInput(_))
    ));
}

#[test]
fn video_id_pages_are_sorted_cursor_exclusive_and_bounded() {
    let page = paginate(
        vec!["c".to_string(), "a".to_string(), "b".to_string()],
        Some("a"),
        NonZeroU16::new(1).unwrap(),
    );
    assert_eq!(page.video_ids, ["b"]);
    assert_eq!(page.next_cursor.as_deref(), Some("b"));
}
