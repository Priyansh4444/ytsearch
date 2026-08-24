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

fn channel(indexed_videos: u32) -> ChannelDocument {
    ChannelDocument {
        id: "channel".to_string(),
        handle: "@channel".to_string(),
        name: "Channel".to_string(),
        channel_id: "UC123".to_string(),
        uploads_playlist: None,
        public_video_count: indexed_videos,
        indexed_video_count: indexed_videos,
        indexed_chunk_count: indexed_videos,
        avatar_url: None,
    }
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
fn channel_deletion_scope_is_durable_and_part_of_idempotency() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("mutations.sqlite3");
    let store = MutationStore::open(&path).unwrap();
    let first = store
        .delete_channel(
            "channel-only",
            ChannelDeletionScope::ChannelOnly,
            "delete-channel-only",
        )
        .unwrap();
    let repeated = store
        .delete_channel(
            "channel-only",
            ChannelDeletionScope::ChannelOnly,
            "delete-channel-only",
        )
        .unwrap();
    assert_eq!(repeated.sequence, first.sequence);
    assert!(matches!(
        store.delete_channel(
            "channel-only",
            ChannelDeletionScope::ChannelAndVideos,
            "delete-channel-only",
        ),
        Err(IndexStoreError::Conflict(_))
    ));
    store
        .delete_channel(
            "channel-and-videos",
            ChannelDeletionScope::ChannelAndVideos,
            "delete-channel-and-videos",
        )
        .unwrap();
    drop(store);

    let snapshot = MutationStore::open(&path).unwrap().snapshot().unwrap();
    assert_eq!(snapshot.channels.len(), 2);
    assert_eq!(
        snapshot.channels[0].deletion_scope,
        ChannelDeletionScope::ChannelAndVideos
    );
    assert_eq!(
        snapshot.channels[1].deletion_scope,
        ChannelDeletionScope::ChannelOnly
    );
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

#[test]
fn snapshot_pins_latest_rows_to_its_captured_sequence() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("mutations.sqlite3");
    let first = MutationStore::open(&path).unwrap();
    let second = MutationStore::open(&path).unwrap();
    let old_chunks = [chunk("video-1", "old text")];
    let new_chunks = [chunk("video-1", "new text")];
    first
        .replace_video(
            VideoReplacement::new("video-1", &old_chunks).unwrap(),
            "replace-video-old",
        )
        .unwrap();
    first
        .upsert_channel(&channel(1), "upsert-channel-old")
        .unwrap();

    let mut connection = first.connection().unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Deferred)
        .unwrap();
    let captured_sequence = snapshot_sequence(&transaction).unwrap();

    second
        .replace_video(
            VideoReplacement::new("video-1", &new_chunks).unwrap(),
            "replace-video-new",
        )
        .unwrap();
    second
        .upsert_channel(&channel(2), "upsert-channel-new")
        .unwrap();

    let captured = snapshot_at_sequence(&transaction, captured_sequence).unwrap();
    transaction.commit().unwrap();
    drop(connection);
    assert_eq!(captured.through_sequence, 2);
    assert_eq!(
        captured.videos[0].chunks.as_deref(),
        Some(old_chunks.as_slice())
    );
    assert_eq!(captured.channels[0].document, Some(channel(1)));

    let current = first.snapshot().unwrap();
    assert_eq!(current.through_sequence, 4);
    assert_eq!(
        current.videos[0].chunks.as_deref(),
        Some(new_chunks.as_slice())
    );
    assert_eq!(current.channels[0].document, Some(channel(2)));
}

#[test]
fn exported_snapshot_is_immutable_and_sequence_exact() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("mutations.sqlite3");
    let export_path = directory.path().join("publication.sqlite3");
    let store = MutationStore::open(&path).unwrap();
    let old_chunks = [chunk("video-1", "old text")];
    let new_chunks = [chunk("video-1", "new text")];
    store
        .replace_video(
            VideoReplacement::new("video-1", &old_chunks).unwrap(),
            "replace-video-old",
        )
        .unwrap();

    let export = store.export_snapshot(&export_path).unwrap();
    assert_eq!(export.through_sequence, 1);
    assert_eq!(export.bytes, std::fs::metadata(&export_path).unwrap().len());
    assert_eq!(export.blake3, hash_file(&export_path).unwrap());

    store
        .replace_video(
            VideoReplacement::new("video-1", &new_chunks).unwrap(),
            "replace-video-new",
        )
        .unwrap();
    let snapshot_store = MutationStore::open_snapshot_read_only(&export_path).unwrap();
    let snapshot = snapshot_store.snapshot().unwrap();
    assert_eq!(snapshot.through_sequence, 1);
    assert_eq!(
        snapshot.videos[0].chunks.as_deref(),
        Some(old_chunks.as_slice())
    );
    assert!(
        snapshot_store
            .delete_video("video-1", "readonly-delete")
            .is_err()
    );
    assert!(matches!(
        store.export_snapshot(&export_path),
        Err(IndexStoreError::Conflict(_))
    ));
}

#[test]
fn read_only_snapshot_rejects_unknown_schema() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("unknown.sqlite3");
    Connection::open(&path).unwrap();

    assert!(matches!(
        MutationStore::open_snapshot_read_only(&path),
        Err(IndexStoreError::InvalidInput(_))
    ));
}
