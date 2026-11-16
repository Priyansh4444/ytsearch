use super::*;
use tempfile::tempdir;

/// A snapshot with its streamed chunks collected, for assertions.
struct Materialized {
    through_sequence: u64,
    videos: Vec<StoredVideoMutation>,
    channels: Vec<StoredChannelMutation>,
}

struct StoredVideoMutation {
    chunks: Option<Vec<TranscriptChunk>>,
}

fn collect(snapshot: &MutationSnapshot, chunks: Vec<Option<Vec<TranscriptChunk>>>) -> Materialized {
    assert_eq!(snapshot.video_ids.len(), chunks.len());
    Materialized {
        through_sequence: snapshot.through_sequence,
        videos: chunks
            .into_iter()
            .map(|chunks| StoredVideoMutation { chunks })
            .collect(),
        channels: snapshot.channels.clone(),
    }
}

fn materialize(transaction: &Transaction<'_>, through_sequence: i64) -> Materialized {
    let snapshot = snapshot_at_sequence(transaction, through_sequence).unwrap();
    let mut statement = transaction.prepare(VIDEO_CHUNKS_SQL).unwrap();
    let chunks = statement
        .query_map([through_sequence], |row| {
            json_column::<Vec<TranscriptChunk>>(row, 0)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    collect(&snapshot, chunks)
}

impl MutationStore {
    fn snapshot(&self) -> Result<Materialized, IndexStoreError> {
        self.with_snapshot(|snapshot, chunks| {
            Ok(collect(snapshot, chunks.collect::<Result<Vec<_>, _>>()?))
        })
    }
}

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
    assert_eq!(snapshot.through_sequence, deletion.sequence.unwrap());
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

    let captured = materialize(&transaction, captured_sequence);
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

#[test]
fn legacy_summary_indexes_preserve_chunks_tombstones_and_reopen() {
    const LEGACY_CHUNKS: &[u8] = br#"[{"id":"legacy-0","youtuber_id":"Legacy","youtuber_handle":"@legacy","youtuber_name":"Legacy","channel_id":"UClegacy","video_id":"legacy","title":"Title","text":"preserve these chunks","sequence":0,"t_start_ms":0,"t_end_ms":1,"uploader_id":"Legacy"}]"#;
    let directory = tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite3");
    // Literal pre-summary schema and wire data avoid relying on today's writer.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE video_mutations (
               video_id TEXT PRIMARY KEY, sequence INTEGER NOT NULL, chunks_json BLOB
             );
             INSERT INTO video_mutations VALUES ('deleted', 2, NULL);
             PRAGMA user_version=1;",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO video_mutations VALUES ('legacy', 1, ?1)",
            [LEGACY_CHUNKS],
        )
        .unwrap();
    drop(connection);

    for _ in 0..2 {
        let store = MutationStore::open(&path).unwrap();
        assert_eq!(
            store.video_mutation_summaries().unwrap(),
            [
                VideoMutationSummary {
                    video_id: "deleted".to_string(),
                    youtuber_id: None,
                    chunk_count: None,
                },
                VideoMutationSummary {
                    video_id: "legacy".to_string(),
                    youtuber_id: Some("Legacy".to_string()),
                    chunk_count: Some(1),
                },
            ]
        );
        let connection = store.connection().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT chunks_json FROM video_mutations WHERE video_id='legacy'",
                    [],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .unwrap(),
            LEGACY_CHUNKS
        );
        let missing: i64 = connection
            .query_row(
                "SELECT count(*) FROM video_mutations INDEXED BY video_mutations_missing_summary
                 WHERE chunks_json IS NOT NULL
                   AND (youtuber_id IS NULL OR chunk_count IS NULL)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(connection);
        assert_eq!(missing, 0);
    }
}

// Counts SQLite page-cache misses, including overflow-page reads which VM-step
// counts cannot expose. The connection is borrowed exclusively by this test.
fn sqlite_cache_misses(connection: &Connection) -> i32 {
    let mut current = 0;
    let mut highwater = 0;
    // SAFETY: the connection remains live and unused concurrently; SQLite owns
    // the handle, and both output pointers refer to initialized stack integers.
    let result = unsafe {
        rusqlite::ffi::sqlite3_db_status(
            connection.handle(),
            rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS,
            &raw mut current,
            &raw mut highwater,
            0,
        )
    };
    assert_eq!(result, rusqlite::ffi::SQLITE_OK);
    current
}

#[test]
fn completed_summary_reads_and_startup_do_not_page_through_chunk_bodies() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("large-chunks.sqlite3");
    let store = MutationStore::open(&path).unwrap();
    {
        let mut connection = store.connection().unwrap();
        // Metadata appears after each 64 KiB body, reproducing the overflow-page
        // layout. Completed summaries must not parse or read the arbitrary body.
        let transaction = connection.transaction().unwrap();
        transaction
            .execute(
                "INSERT INTO operations(sequence,id,kind,target,payload_hash,state)
                 VALUES (1,'fixture','replace_video','fixture',x'00',0)",
                [],
            )
            .unwrap();
        for index in 0..128 {
            transaction
                .execute(
                    "INSERT INTO video_mutations
                       (video_id,sequence,chunks_json,youtuber_id,chunk_count)
                     VALUES (?1,1,zeroblob(65536),'creator',7)",
                    [format!("video-{index:03}")],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        drop(connection);
    }
    drop(store);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA cache_size=8; PRAGMA mmap_size=0; PRAGMA shrink_memory;")
        .unwrap();
    let startup_before = sqlite_cache_misses(&connection);
    ensure_video_summary_columns(&connection).unwrap();
    let startup_misses = sqlite_cache_misses(&connection) - startup_before;
    assert!(startup_misses < 24, "startup read {startup_misses} pages");
    let sql = "SELECT video_id,youtuber_id,chunk_count FROM video_mutations ORDER BY video_id";
    {
        let plan = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("\n");
        assert!(plan.contains("COVERING INDEX"), "{plan}");
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
    }
    connection.execute_batch("PRAGMA shrink_memory;").unwrap();
    let store = MutationStore {
        connection: Mutex::new(connection),
    };
    let before = sqlite_cache_misses(&store.connection().unwrap());
    let summaries = store.video_mutation_summaries().unwrap();
    let indexed_misses = sqlite_cache_misses(&store.connection().unwrap()) - before;
    assert_eq!(summaries.len(), 128);
    assert_eq!(summaries[64].video_id, "video-064");
    assert!(summaries.iter().all(|summary| {
        summary.youtuber_id.as_deref() == Some("creator") && summary.chunk_count == Some(7)
    }));
    assert!(indexed_misses < 24, "summary read {indexed_misses} pages");
    assert_eq!(
        store.video_mutation_summary("video-064").unwrap(),
        Some(summaries[64].clone())
    );

    // Negative control: the same result from the large table really is costly,
    // so a fixture that accidentally had no overflow pages cannot pass silently.
    let connection = store.connection().unwrap();
    connection.execute_batch("PRAGMA shrink_memory;").unwrap();
    let before = sqlite_cache_misses(&connection);
    let unindexed = connection
        .prepare(
            "SELECT video_id,youtuber_id,chunk_count FROM video_mutations NOT INDEXED
             ORDER BY video_id",
        )
        .unwrap()
        .query_map([], video_summary_from_row)
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let table_misses = sqlite_cache_misses(&connection) - before;
    drop(connection);
    assert_eq!(unindexed, summaries);
    eprintln!(
        "summary_page_reads startup={startup_misses} indexed={indexed_misses} table={table_misses}"
    );
    assert!(
        table_misses > 128,
        "negative control read only {table_misses} pages"
    );
}
