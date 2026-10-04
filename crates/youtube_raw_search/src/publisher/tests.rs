use super::*;
use crate::build_index;
use flate2::{Compression, write::GzEncoder};
use serde::Serialize;
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
};
use tempfile::tempdir;
use youtube_index_store::VideoReplacement;
use youtube_ingest::TranscriptChunk;
use youtube_search_service::SearchParams;

fn write_gzip_lines(path: &Path, values: &[impl Serialize]) {
    let mut output = GzEncoder::new(File::create(path).unwrap(), Compression::fast());
    for value in values {
        serde_json::to_writer(&mut output, value).unwrap();
        output.write_all(b"\n").unwrap();
    }
    output.finish().unwrap();
}

fn chunk(video_id: &str, text: &str) -> TranscriptChunk {
    serde_json::from_value(json!({
        "id": format!("{video_id}-0"),
        "youtuber_id": "channel",
        "youtuber_handle": "@channel",
        "youtuber_name": "Channel",
        "channel_id": "UC123",
        "video_id": video_id,
        "title": format!("Title {video_id}"),
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

fn fixture() -> (tempfile::TempDir, Publisher, MutationStore) {
    let directory = tempdir().unwrap();
    let base_chunks = directory.path().join("base-chunks.jsonl.gz");
    let base_channels = directory.path().join("base-channels.jsonl.gz");
    let base_index = directory.path().join("base-index");
    let base_chunks_values = [
        serde_json::to_value(chunk("video-old", "legacyuniquetoken")).unwrap(),
        serde_json::to_value(chunk("video-delete", "deleteuniquetoken")).unwrap(),
    ];
    write_gzip_lines(&base_chunks, &base_chunks_values);
    write_gzip_lines(&base_channels, &[channel(2)]);
    let plain_chunks = directory.path().join("base-chunks.jsonl");
    let plain_channels = directory.path().join("base-channels.jsonl");
    let mut chunks_output = BufWriter::new(File::create(&plain_chunks).unwrap());
    for value in &base_chunks_values {
        serde_json::to_writer(&mut chunks_output, value).unwrap();
        chunks_output.write_all(b"\n").unwrap();
    }
    chunks_output.flush().unwrap();
    let mut channels_output = BufWriter::new(File::create(&plain_channels).unwrap());
    serde_json::to_writer(&mut channels_output, &channel(2)).unwrap();
    channels_output.write_all(b"\n").unwrap();
    channels_output.flush().unwrap();
    build_index(&plain_chunks, &plain_channels, &base_index).unwrap();

    let state_directory = directory.path().join("state");
    let publisher = Publisher::new(PublisherPaths {
        base_chunks_gzip: base_chunks,
        base_channels_gzip: base_channels,
        base_index,
        state_directory: state_directory.clone(),
    });
    let store = MutationStore::open(&state_directory.join("mutations.sqlite3")).unwrap();
    (directory, publisher, store)
}

fn found(engine: &RawSearchEngine, query: &str) -> u64 {
    engine
        .search(&SearchParams {
            q: query.to_string(),
            group_by: Some("video_id".to_string()),
            ..SearchParams::default()
        })
        .unwrap()["found"]
        .as_u64()
        .unwrap()
}

fn replace_add_and_delete(store: &MutationStore) {
    let replacement_chunks = [chunk("video-old", "replacement blackhole phrase")];
    let added_chunks = [chunk("video-added", "brandnew searchable phrase")];
    store
        .replace_video(
            VideoReplacement::new("video-old", &replacement_chunks).unwrap(),
            "replace-old",
        )
        .unwrap();
    store
        .replace_video(
            VideoReplacement::new("video-added", &added_chunks).unwrap(),
            "add-video",
        )
        .unwrap();
    store.delete_video("video-delete", "delete-video").unwrap();
    store.upsert_channel(&channel(2), "upsert-channel").unwrap();
}

/// Publishes `store` the way production does: a detached build inside remote
/// work, then an exact install.
fn install_detached(publisher: &Publisher, store: &MutationStore) {
    let work = publisher
        .paths
        .state_directory
        .join("work/remote-publication");
    fs::create_dir_all(work.join("job")).unwrap();
    let snapshot_path = work.join("snapshot.sqlite3");
    store.export_snapshot(&snapshot_path).unwrap();
    let snapshot_store = MutationStore::open_snapshot_read_only(&snapshot_path).unwrap();
    let builder = GenerationBuilder::new(
        publisher.paths.base_chunks_gzip.clone(),
        publisher.paths.base_channels_gzip.clone(),
    );
    let compatibility = builder.compatibility().unwrap();
    let detached = builder
        .build(
            &snapshot_store,
            &compatibility,
            &work.join("job/generation"),
        )
        .unwrap();
    let manifest = fs::read(detached.path.join(GENERATION_MANIFEST)).unwrap();
    publisher
        .install_verified_generation(detached.sequence, &detached.path, &manifest)
        .unwrap();
}

#[test]
fn detached_generation_builds_without_activation_or_wal_changes() {
    let (directory, publisher, store) = fixture();
    replace_add_and_delete(&store);
    let snapshot_path = directory.path().join("mutation-snapshot.sqlite3");
    let export = store.export_snapshot(&snapshot_path).unwrap();
    let snapshot_store = MutationStore::open_snapshot_read_only(&snapshot_path).unwrap();
    let builder = GenerationBuilder::new(
        publisher.paths.base_chunks_gzip.clone(),
        publisher.paths.base_channels_gzip.clone(),
    );
    let compatibility = builder.compatibility().unwrap();
    let detached_path = directory.path().join("detached-generation");

    let detached = builder
        .build(&snapshot_store, &compatibility, &detached_path)
        .unwrap();

    assert_eq!(detached.sequence, export.through_sequence);
    assert_eq!(detached.path, detached_path);
    assert!(
        fs::read_dir(&detached.path).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')
        }),
        "completed generations must not retain builder spools"
    );
    let detached_engine = RawSearchEngine::open(&detached.path).unwrap();
    assert_eq!(found(&detached_engine, "legacyuniquetoken"), 0);
    assert_eq!(found(&detached_engine, "replacement blackhole"), 1);
    assert_eq!(found(&detached_engine, "brandnew searchable"), 1);
    assert_eq!(found(&detached_engine, "deleteuniquetoken"), 0);
    assert_eq!(publisher.active_sequence().unwrap(), 0);
    assert!(
        !publisher
            .paths
            .state_directory
            .join(ACTIVE_MANIFEST)
            .exists()
    );
    assert_eq!(
        store.operation("add-video").unwrap().state,
        youtube_index_store::IndexOperationState::Durable
    );
}

#[test]
fn detached_install_is_exact_monotonic_and_does_not_acknowledge_wal() {
    let (_directory, publisher, store) = fixture();
    replace_add_and_delete(&store);
    let snapshot_path = publisher
        .paths
        .state_directory
        .join("work/remote-publication/snapshot.sqlite3");
    fs::create_dir_all(snapshot_path.parent().unwrap()).unwrap();
    store.export_snapshot(&snapshot_path).unwrap();
    let snapshot_store = MutationStore::open_snapshot_read_only(&snapshot_path).unwrap();
    let builder = GenerationBuilder::new(
        publisher.paths.base_chunks_gzip.clone(),
        publisher.paths.base_channels_gzip.clone(),
    );
    let compatibility = builder.compatibility().unwrap();
    let incoming = publisher
        .paths
        .state_directory
        .join("work/remote-publication/job/generation");
    fs::create_dir_all(incoming.parent().unwrap()).unwrap();
    let detached = builder
        .build(&snapshot_store, &compatibility, &incoming)
        .unwrap();
    let manifest = fs::read(detached.path.join(GENERATION_MANIFEST)).unwrap();
    let recovered_final = publisher
        .paths
        .state_directory
        .join(format!("generations/{:020}", detached.sequence));
    fs::create_dir_all(recovered_final.parent().unwrap()).unwrap();
    fs::rename(&detached.path, &recovered_final).unwrap();

    let installed = publisher
        .install_verified_generation(detached.sequence, &detached.path, &manifest)
        .unwrap();

    assert_eq!(publisher.active_sequence().unwrap(), detached.sequence);
    assert_eq!(found(&installed.into_engine(), "replacement blackhole"), 1);
    let installed_path = publisher.active_index_path().unwrap();
    assert_eq!(
        store.operation("add-video").unwrap().state,
        youtube_index_store::IndexOperationState::Durable,
        "installation must not acknowledge the WAL before the reader swap"
    );
    assert!(
        !detached.path.exists(),
        "activation recovery must not require the already-renamed input"
    );

    publisher
        .install_verified_generation(detached.sequence, &detached.path, &manifest)
        .unwrap();
    assert_eq!(publisher.active_index_path().unwrap(), installed_path);

    let mut different_manifest = manifest.clone();
    different_manifest.push(b'\n');
    assert!(matches!(
        publisher.install_verified_generation(
            detached.sequence,
            &detached.path,
            &different_manifest
        ),
        Err(IndexStoreError::Conflict(_))
    ));
    assert!(matches!(
        publisher.install_verified_generation(detached.sequence - 1, &detached.path, &manifest),
        Err(IndexStoreError::Conflict(_))
    ));

    let (recovered, recovered_sequence) = publisher.open_active_generation(&store).unwrap();
    assert_eq!(recovered_sequence, detached.sequence);
    assert_eq!(found(&recovered, "replacement blackhole"), 1);
    assert_eq!(
        store.operation("add-video").unwrap().state,
        youtube_index_store::IndexOperationState::Published,
        "startup must reconcile an active generation persisted before WAL acknowledgement"
    );
}

#[test]
fn detached_install_rejects_a_verified_generation_outside_remote_work() {
    let (directory, publisher, store) = fixture();
    replace_add_and_delete(&store);
    let builder = GenerationBuilder::new(
        publisher.paths.base_chunks_gzip.clone(),
        publisher.paths.base_channels_gzip.clone(),
    );
    let compatibility = builder.compatibility().unwrap();
    let outside = directory.path().join("outside-generation");
    let detached = builder.build(&store, &compatibility, &outside).unwrap();
    let manifest = fs::read(detached.path.join(GENERATION_MANIFEST)).unwrap();

    assert!(matches!(
        publisher.install_verified_generation(detached.sequence, &detached.path, &manifest),
        Err(IndexStoreError::InvalidInput(_))
    ));
    assert_eq!(publisher.active_sequence().unwrap(), 0);
}

#[test]
fn corpus_identity_is_path_independent_and_rejects_source_or_format_mismatch() {
    let (directory, publisher, store) = fixture();
    let expected_builder = GenerationBuilder::new(
        publisher.paths.base_chunks_gzip.clone(),
        publisher.paths.base_channels_gzip.clone(),
    );
    let expected = expected_builder.compatibility().unwrap();
    assert_eq!(expected.generation_format, RAW_GENERATION_FORMAT);

    let copied_chunks = directory.path().join("copied-chunks.jsonl.gz");
    let copied_channels = directory.path().join("copied-channels.jsonl.gz");
    fs::copy(&publisher.paths.base_chunks_gzip, &copied_chunks).unwrap();
    fs::copy(&publisher.paths.base_channels_gzip, &copied_channels).unwrap();
    let copied_builder = GenerationBuilder::new(copied_chunks.clone(), copied_channels.clone());
    assert_eq!(copied_builder.compatibility().unwrap(), expected);

    write_gzip_lines(&copied_channels, &[channel(3)]);
    let channel_changed = copied_builder.compatibility().unwrap();
    assert_eq!(
        channel_changed.base_chunks_blake3,
        expected.base_chunks_blake3
    );
    assert_ne!(
        channel_changed.base_channels_blake3,
        expected.base_channels_blake3
    );
    assert_ne!(channel_changed.corpus_identity, expected.corpus_identity);

    fs::copy(&publisher.paths.base_channels_gzip, &copied_channels).unwrap();
    write_gzip_lines(
        &copied_chunks,
        &[serde_json::to_value(chunk("different", "different corpus")).unwrap()],
    );
    let chunks_changed = copied_builder.compatibility().unwrap();
    assert_ne!(
        chunks_changed.base_chunks_blake3,
        expected.base_chunks_blake3
    );
    assert_eq!(
        chunks_changed.base_channels_blake3,
        expected.base_channels_blake3
    );
    assert_ne!(chunks_changed.corpus_identity, expected.corpus_identity);

    let source_mismatch_path = directory.path().join("source-mismatch");
    assert!(matches!(
        copied_builder.build(&store, &expected, &source_mismatch_path),
        Err(IndexStoreError::InvalidInput(_))
    ));
    assert!(!source_mismatch_path.exists());

    let mut wrong_format = expected.clone();
    wrong_format.generation_format = "raw-index-schema-unknown".to_string();
    let format_mismatch_path = directory.path().join("format-mismatch");
    assert!(matches!(
        expected_builder.build(&store, &wrong_format, &format_mismatch_path),
        Err(IndexStoreError::InvalidInput(_))
    ));
    assert!(!format_mismatch_path.exists());
}

#[test]
fn publisher_caches_corpus_compatibility_after_the_first_verified_read() {
    let (_directory, publisher, _store) = fixture();
    let expected = publisher.generation_compatibility().unwrap();

    fs::remove_file(&publisher.paths.base_chunks_gzip).unwrap();
    fs::remove_file(&publisher.paths.base_channels_gzip).unwrap();

    assert_eq!(publisher.generation_compatibility().unwrap(), expected);
}

#[test]
fn restart_open_fully_revalidates_a_published_generation() {
    let (_directory, publisher, store) = fixture();
    store.delete_video("video-delete", "delete-video").unwrap();
    install_detached(&publisher, &store);
    let generation_path = publisher.active_index_path().unwrap();

    let mut postings = OpenOptions::new()
        .read(true)
        .write(true)
        .open(generation_path.join("postings.bin"))
        .expect("postings");
    postings.seek(SeekFrom::End(-1)).expect("seek posting");
    let mut byte = [0_u8; 1];
    postings.read_exact(&mut byte).expect("read posting");
    byte[0] ^= 0x01;
    postings
        .seek(SeekFrom::End(-1))
        .expect("seek posting again");
    postings.write_all(&byte).expect("corrupt posting");
    postings.flush().expect("flush corruption");

    let active_path = publisher.active_index_path().expect("active path");
    assert!(matches!(
        RawSearchEngine::open(&active_path),
        Err(crate::Error::Corrupt(_))
    ));
}
