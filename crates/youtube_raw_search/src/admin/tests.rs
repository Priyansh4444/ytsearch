use super::*;
use crate::{PublisherPaths, RawSearchEngine, build_index};
use axum::extract::{Path, State};
use flate2::{Compression, write::GzEncoder};
use std::{fs::File, io::Write};
use youtube_search_service::{SearchBackend, SearchParams};

const KEY: &str = "test-admin-key-24-bytes-long";

fn chunk(video_id: &str, token: &str) -> TranscriptChunk {
    serde_json::from_value(serde_json::json!({
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

fn gzip_json_lines<T: Serialize>(path: &std::path::Path, values: &[T]) {
    let mut output = GzEncoder::new(File::create(path).unwrap(), Compression::fast());
    for value in values {
        serde_json::to_writer(&mut output, value).unwrap();
        output.write_all(b"\n").unwrap();
    }
    output.finish().unwrap();
}

fn setup() -> (tempfile::TempDir, AdminState) {
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
    let state = AdminState::new(engine, store, publisher, KEY.to_string()).unwrap();
    (root, state)
}

fn headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ADMIN_KEY_HEADER, KEY.parse().unwrap());
    headers
}

#[tokio::test]
async fn video_mutation_is_durable_then_atomically_published() {
    let (_root, state) = setup();
    let (_, Json(operation)) = replace_video(
        State(state.clone()),
        Path("new-video".to_string()),
        headers(),
        Json(ReplaceVideoRequest {
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

    state.clone().publish_pending().await;

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
async fn admin_routes_reject_missing_keys_before_accessing_state() {
    let (_root, state) = setup();
    let error = readiness(State(state), HeaderMap::new()).await.unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn admin_keys_have_a_minimum_strength_and_constant_time_comparison() {
    let (_root, state) = setup();
    assert!(constant_time_eq(KEY, KEY));
    assert!(!constant_time_eq(KEY, "test-admin-key-24-bytes-lonf"));
    assert!(!constant_time_eq(KEY, "short"));
    assert!(matches!(
        AdminState::new(
            Arc::clone(&state.engine),
            Arc::clone(&state.store),
            Arc::clone(&state.publisher),
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
