use super::*;
use tempfile::tempdir;

fn chunk(video_id: &str, text: &str) -> TranscriptChunk {
    serde_json::from_value(serde_json::json!({
        "id": format!("{video_id}-0"),
        "youtuber_id": "channel",
        "youtuber_handle": "@channel",
        "youtuber_name": "Channel",
        "channel_id": "UC123",
        "video_id": video_id,
        "title": "Title",
        "text": text,
        "sequence": 0,
        "t_start_ms": 0,
        "t_end_ms": 1,
        "uploader_id": "channel"
    }))
    .unwrap()
}

#[test]
fn durable_replacement_survives_reopen_and_is_idempotent() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("mutations.sqlite3");
    let chunks = [chunk("video-1", "black hole")];
    let replacement = VideoReplacement::new("video-1", &chunks).unwrap();
    let first = MutationStore::open(&path)
        .unwrap()
        .replace_video(replacement, "replace-video-1")
        .unwrap();

    let reopened = MutationStore::open(&path).unwrap();
    let repeated = reopened
        .replace_video(replacement, "replace-video-1")
        .unwrap();
    let snapshot = reopened.snapshot().unwrap();

    assert_eq!(repeated.sequence, first.sequence);
    assert_eq!(snapshot.through_sequence, first.sequence.unwrap());
    assert_eq!(snapshot.videos.len(), 1);
    assert_eq!(snapshot.videos[0].chunks.as_ref().unwrap(), &chunks);
    assert_eq!(
        reopened.video_mutation_summary("video-1").unwrap(),
        Some(VideoMutationSummary {
            video_id: "video-1".to_string(),
            youtuber_id: Some("channel".to_string()),
            chunk_count: Some(1),
        })
    );
}

#[test]
fn reused_idempotency_key_with_different_payload_conflicts() {
    let directory = tempdir().unwrap();
    let store = MutationStore::open(&directory.path().join("mutations.sqlite3")).unwrap();
    let first_chunks = [chunk("video-1", "first")];
    let second_chunks = [chunk("video-1", "second")];
    store
        .replace_video(
            VideoReplacement::new("video-1", &first_chunks).unwrap(),
            "replace-video-1",
        )
        .unwrap();

    let error = store
        .replace_video(
            VideoReplacement::new("video-1", &second_chunks).unwrap(),
            "replace-video-1",
        )
        .unwrap_err();

    assert!(matches!(error, IndexStoreError::Conflict(_)));
}

#[test]
fn latest_video_mutation_is_the_only_published_input() {
    let directory = tempdir().unwrap();
    let store = MutationStore::open(&directory.path().join("mutations.sqlite3")).unwrap();
    let chunks = [chunk("video-1", "first")];
    store
        .replace_video(
            VideoReplacement::new("video-1", &chunks).unwrap(),
            "replace-video-1",
        )
        .unwrap();
    let deletion = store.delete_video("video-1", "delete-video-1").unwrap();

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.videos.len(), 1);
    assert!(snapshot.videos[0].chunks.is_none());
    assert_eq!(snapshot.videos[0].sequence, deletion.sequence.unwrap());
    assert_eq!(
        store
            .video_mutation_summary("video-1")
            .unwrap()
            .unwrap()
            .chunk_count,
        None
    );
}

#[test]
fn publication_state_persists() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("mutations.sqlite3");
    let store = MutationStore::open(&path).unwrap();
    let operation = store.delete_video("video-1", "delete-video-1").unwrap();
    store.mark_published(operation.sequence.unwrap()).unwrap();
    drop(store);

    let published = MutationStore::open(&path)
        .unwrap()
        .operation("delete-video-1")
        .unwrap();
    assert_eq!(published.state, IndexOperationState::Published);
    assert_eq!(published.acknowledged_readers, 1);
}
