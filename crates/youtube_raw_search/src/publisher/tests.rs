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
        .install_verified_generation(detached.sequence, &detached.path, &manifest, &[])
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
        .install_verified_generation(detached.sequence, &detached.path, &manifest, &[])
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
        .install_verified_generation(detached.sequence, &detached.path, &manifest, &[])
        .unwrap();
    assert_eq!(publisher.active_index_path().unwrap(), installed_path);

    let mut different_manifest = manifest.clone();
    different_manifest.push(b'\n');
    assert!(matches!(
        publisher.install_verified_generation(
            detached.sequence,
            &detached.path,
            &different_manifest,
            &[],
        ),
        Err(IndexStoreError::Conflict(_))
    ));
    assert!(matches!(
        publisher.install_verified_generation(
            detached.sequence - 1,
            &detached.path,
            &manifest,
            &[]
        ),
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
        publisher.install_verified_generation(detached.sequence, &detached.path, &manifest, &[]),
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

#[cfg(unix)]
#[test]
#[ignore = "requires YTSEARCH_REUSE_PROFILE_INDEX with a corpus-sized immutable index"]
fn profile_verified_generation_file_reuse() {
    use std::os::unix::fs::MetadataExt;
    let index = PathBuf::from(std::env::var_os("YTSEARCH_REUSE_PROFILE_INDEX").unwrap());
    let root = tempfile::tempdir_in(index.parent().unwrap()).unwrap();
    let publisher = Publisher::new(PublisherPaths {
        base_chunks_gzip: index.join("unused-chunks"),
        base_channels_gzip: index.join("unused-channels"),
        base_index: index.clone(),
        state_directory: root.path().join("state"),
    });
    let manifest: youtube_index_store::RawGenerationManifest =
        serde_json::from_reader(File::open(index.join("manifest.json")).unwrap()).unwrap();
    for round in 0..2 {
        for share in if round == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let output = root.path().join(format!("{round}-{share}"));
            fs::create_dir(&output).unwrap();
            let started = std::time::Instant::now();
            let mut bytes = 0;
            for (name, expected) in &manifest.files {
                let source = index.join(name);
                let target = output.join(name);
                if share {
                    assert!(
                        publisher
                            .link_matching_active_file(name, expected, &target)
                            .unwrap()
                    );
                    assert_eq!(
                        fs::metadata(&source).unwrap().ino(),
                        fs::metadata(&target).unwrap().ino()
                    );
                } else {
                    bytes += fs::copy(source, &target).unwrap();
                    File::open(&target).unwrap().sync_all().unwrap();
                }
            }
            eprintln!(
                "reuse_profile round={round} share={share} elapsed_ms={} new_file_bytes={bytes} verified_source_bytes={}",
                started.elapsed().as_millis(),
                manifest.files.values().map(|f| f.bytes).sum::<u64>()
            );
        }
    }
}

#[test]
#[ignore = "requires full source gzip archives and a new YTSEARCH_CHANGED_VIDEO_ROOT"]
fn build_full_corpus_changed_video_fixture() {
    let root = PathBuf::from(std::env::var_os("YTSEARCH_CHANGED_VIDEO_ROOT").unwrap());
    assert!(!root.exists(), "use a fresh evidence directory");
    fs::create_dir_all(&root).unwrap();
    let chunks = PathBuf::from(std::env::var_os("YTSEARCH_CHANGED_VIDEO_CHUNKS").unwrap());
    let channels = PathBuf::from(std::env::var_os("YTSEARCH_CHANGED_VIDEO_CHANNELS").unwrap());
    let first = gzip_lines(&chunks).unwrap().next().unwrap().unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&first).unwrap();
    let video = "warmfix0001".to_string();
    value["video_id"] = serde_json::Value::String(video.clone());
    value["id"] = serde_json::Value::String(format!("{video}-0"));
    value["title"] = serde_json::Value::String("WarmFixtureProof".into());
    value["text"] = serde_json::Value::String("activationfixtureproof".into());
    let replacement: TranscriptChunk = serde_json::from_value(value).unwrap();
    let store = MutationStore::open(&root.join("mutations.sqlite3")).unwrap();
    store
        .replace_video(
            VideoReplacement::new(&video, &[replacement]).unwrap(),
            "changed-video-fixture",
        )
        .unwrap();
    let builder = GenerationBuilder::new(chunks, channels);
    let compatibility = builder.compatibility().unwrap();
    crate::cache_io::retain_build_page_cache();
    let started = std::time::Instant::now();
    let detached = builder
        .build(&store, &compatibility, &root.join("generation"))
        .unwrap();
    let engine = RawSearchEngine::open(&detached.path).unwrap();
    assert_eq!(
        engine
            .search(&SearchParams {
                q: "activationfixtureproof".into(),
                ..SearchParams::default()
            })
            .unwrap()["found"],
        1
    );
    eprintln!(
        "changed_video_fixture={}",
        serde_json::json!({"video_id":video,"sequence":detached.sequence,
        "documents":engine.document_count(),"elapsed_ms":started.elapsed().as_millis(),"index":detached.path})
    );
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires owned old/new corpus copies and an isolated memory cgroup"]
fn profile_corpus_activation_working_set() {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    use youtube_search_service::SearchBackend;
    let root = PathBuf::from(std::env::var_os("YTSEARCH_ACTIVATION_PROFILE_ROOT").unwrap());
    let mode = std::env::var("YTSEARCH_ACTIVATION_PROFILE_MODE").unwrap();
    let queries = include_str!("../../../../scripts/benchmark-search-cases.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut fields = line.split('\t');
            let _label = fields.next().unwrap();
            SearchParams {
                q: fields.next().unwrap().into(),
                filter_by: fields
                    .next()
                    .filter(|v| !v.is_empty())
                    .map(|v| format!("youtuber_handle:={v}")),
                query_by: Some("text,title".into()),
                group_by: Some("video_id".into()),
                query_by_weights: Some("4,6".into()),
                group_limit: Some(3),
                limit: Some(10),
                offset: Some(0),
                text_match_type: Some("sum_score".into()),
                prioritize_exact_match: Some(true),
                prioritize_token_position: Some(true),
                typo_tokens_threshold: Some(5),
                split_join_tokens: Some("always".into()),
                ..SearchParams::default()
            }
        })
        .collect::<Vec<_>>();
    // Only owned, unshared copies are evicted. Never touch global VM caches.
    for directory in [root.join("old"), root.join("new")] {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let file = File::open(path).unwrap();
            assert_eq!(file.metadata().unwrap().nlink(), 1);
            file.sync_all().unwrap();
            // SAFETY: a valid open file descriptor and an advisory whole-file range.
            assert_eq!(
                unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) },
                0
            );
        }
    }
    let old = RawSearchEngine::open(&root.join("old")).unwrap();
    old.warm();
    let serving = crate::ReloadableEngine::new(old, 0);
    let mut expected = Vec::new();
    for query in &queries {
        let result = serving.search(query).unwrap();
        expected.push(
            result["grouped_hits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|group| group["group_key"].clone())
                .collect::<Vec<_>>(),
        );
    }
    let started = std::time::Instant::now();
    let next = RawSearchEngine::open_verified_transfer(&root.join("new")).unwrap();
    let recent = if mode == "recent" {
        serving.recent_searches()
    } else {
        Vec::new()
    };
    let warmed = next.warmed_for_activation(&recent).unwrap();
    serving.replace(warmed, 1).unwrap();
    let activation_ms = started.elapsed().as_millis();
    let mut samples = Vec::new();
    for (query, expected) in queries.iter().zip(expected) {
        let before = profile_major_faults();
        let started = std::time::Instant::now();
        let result = serving.search(query).unwrap();
        let actual = result["grouped_hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group["group_key"].clone())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        samples.push(serde_json::json!({"query":query.q,"elapsed_us":started.elapsed().as_micros(),"major_faults":profile_major_faults().saturating_sub(before)}));
    }
    let cgroup = fs::read_to_string("/proc/self/cgroup").unwrap();
    let relative = cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap();
    let cgroup = PathBuf::from("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
    let peak = fs::read_to_string(cgroup.join("memory.peak")).unwrap();
    let swap = fs::read_to_string(cgroup.join("memory.swap.current")).unwrap();
    eprintln!(
        "activation_profile={}",
        serde_json::json!({"mode":mode,"activation_ms":activation_ms,"cgroup_memory_peak":peak.trim(),"swap":swap.trim(),"samples":samples})
    );
}

#[cfg(target_os = "linux")]
fn profile_major_faults() -> u64 {
    fs::read_to_string("/proc/self/stat")
        .unwrap()
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .nth(9)
        .unwrap()
        .parse()
        .unwrap()
}
