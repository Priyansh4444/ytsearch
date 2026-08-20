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

#[test]
fn publication_atomically_replaces_adds_and_deletes_videos() {
    let (_directory, publisher, store) = fixture();
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

    let published = publisher.publish(&store).unwrap().unwrap();

    assert_eq!(found(&published.engine, "legacyuniquetoken"), 0);
    assert_eq!(found(&published.engine, "replacement blackhole"), 1);
    assert_eq!(found(&published.engine, "brandnew searchable"), 1);
    assert_eq!(found(&published.engine, "deleteuniquetoken"), 0);
    assert_eq!(publisher.active_index_path().unwrap(), published.path);
    assert_eq!(
        store.operation("add-video").unwrap().state,
        youtube_index_store::IndexOperationState::Published
    );
    assert_eq!(
        fs::read_dir(publisher.paths.state_directory.join("work"))
            .unwrap()
            .count(),
        0,
        "publication must not materialize a merged JSON corpus"
    );
    assert!(
        fs::read_dir(&published.path).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with('.')
        }),
        "completed generations must not retain builder spools"
    );
}

#[test]
fn failed_publication_keeps_the_previous_manifest_and_durable_wal() {
    let (_directory, publisher, store) = fixture();
    store.delete_video("video-old", "delete-old").unwrap();
    fs::remove_file(&publisher.paths.base_chunks_gzip).unwrap();

    assert!(publisher.publish(&store).is_err());
    assert_eq!(publisher.active_sequence().unwrap(), 0);
    assert_eq!(
        store.operation("delete-old").unwrap().state,
        youtube_index_store::IndexOperationState::Durable
    );
}

#[test]
fn restart_open_fully_revalidates_a_published_generation() {
    let (_directory, publisher, store) = fixture();
    store.delete_video("video-delete", "delete-video").unwrap();
    let published = publisher.publish(&store).unwrap().unwrap();
    let generation_path = published.path.clone();
    drop(published);

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

#[test]
fn publication_removes_only_uncommitted_recovery_artifacts() {
    let (_directory, publisher, store) = fixture();
    let work = publisher.paths.state_directory.join("work/stale");
    let candidate = publisher
        .paths
        .state_directory
        .join("generations/00000000000000000007.candidate");
    let completed = publisher
        .paths
        .state_directory
        .join("generations/00000000000000000006");
    for path in [&work, &candidate, &completed] {
        fs::create_dir_all(path).unwrap();
        File::create(path.join("marker")).unwrap();
    }

    assert!(publisher.publish(&store).unwrap().is_none());

    assert!(!work.exists());
    assert!(!candidate.exists());
    assert!(completed.join("marker").exists());
}
