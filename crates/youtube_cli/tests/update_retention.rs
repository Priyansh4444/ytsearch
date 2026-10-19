#![cfg(unix)]

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use youtube_index_store::{RawIndexAdminClient, RawIndexAdminConfig};
use youtube_raw_search::{
    AdminState, MutationStore, Publisher, PublisherPaths, RawSearchEngine, ReloadableEngine,
    admin_router, build_index,
};
use youtube_search_service::{SearchBackend, SearchParams};

const KEY: &str = "local-fixture-admin-key-24-bytes";

struct Fixture {
    directory: tempfile::TempDir,
    address: String,
    admin: RawIndexAdminClient,
    engine: Arc<ReloadableEngine>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new(chunks: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("yt-dlp");
        fs::write(
            &executable,
            r#"#!/bin/sh
for last do :; done
case "$last" in
  */@fixture/*)
    case "$last" in
      */videos) entries='[{"id":"new00000001","title":"New failing video"},{"id":"old00000001","title":"Old indexed video"}]' ;;
      *) entries='[]' ;;
    esac
    printf '{"channel_id":"UCfixture","channel":"Fixture","uploader_id":"@fixture","entries":%s}\n' "$entries"
    ;;
  *) printf 'fixture metadata temporarily unavailable\n' >&2; exit 1 ;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();

        let base_chunks = (0..chunks)
            .map(|sequence| {
                json!({
                    "id":format!("old00000001-{sequence}"),
                    "youtuber_id":"fixture", "youtuber_handle":"@fixture",
                    "youtuber_name":"Fixture", "channel_id":"UCfixture",
                    "video_id":"old00000001", "title":"Old indexed video",
                    "text":"legacytoken retained transcript", "sequence":sequence,
                    "t_start_ms":sequence * 1000, "t_end_ms":(sequence + 1) * 1000,
                    "uploader_id":"fixture",
                })
            })
            .collect::<Vec<_>>();
        let channels = if chunks == 0 {
            vec![]
        } else {
            vec![json!({
                "id":"fixture", "handle":"@fixture", "name":"Fixture",
                "channel_id":"UCfixture", "uploads_playlist":"UUfixture",
                "public_video_count":2, "indexed_video_count":1,
                "indexed_chunk_count":chunks,
            })]
        };
        write_corpus(directory.path(), "chunks", &base_chunks);
        write_corpus(directory.path(), "channels", &channels);
        let base_index = directory.path().join("base-index");
        build_index(
            &directory.path().join("chunks.jsonl"),
            &directory.path().join("channels.jsonl"),
            &base_index,
        )
        .unwrap();
        let state_directory = directory.path().join("state");
        let publisher = Arc::new(Publisher::new(PublisherPaths {
            base_chunks_gzip: directory.path().join("chunks.jsonl.gz"),
            base_channels_gzip: directory.path().join("channels.jsonl.gz"),
            base_index: base_index.clone(),
            state_directory: state_directory.clone(),
        }));
        let store =
            Arc::new(MutationStore::open(&state_directory.join("mutations.sqlite3")).unwrap());
        let engine = Arc::new(ReloadableEngine::new(
            RawSearchEngine::open(&base_index).unwrap(),
            0,
        ));
        let state = AdminState::new(Arc::clone(&engine), store, publisher, KEY.to_owned()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, admin_router(state)).await.unwrap();
        });
        let admin = RawIndexAdminClient::new(RawIndexAdminConfig::new(&address, KEY)).unwrap();
        Self {
            directory,
            address,
            admin,
            engine,
            server,
        }
    }

    fn index(&self, arguments: &[&str]) -> Output {
        let mut args = vec![
            "index-youtuber",
            "@fixture",
            "--jobs",
            "1",
            "--defer-publish",
        ];
        args.extend_from_slice(arguments);
        args.push("--transcript-cache-dir");
        self.command(&args, Some(&self.directory.path().join("cache")))
    }

    fn publish(&self) {
        let output = self.command(&["publish"], None);
        assert!(output.status.success(), "{}", transcript(&output));
    }

    fn command(&self, arguments: &[&str], last_path: Option<&Path>) -> Output {
        let stdout = self.directory.path().join("stdout");
        let stderr = self.directory.path().join("stderr");
        let mut command = Command::new(env!("CARGO_BIN_EXE_ytsearch"));
        command.args(arguments);
        if let Some(path) = last_path {
            command.arg(path);
        }
        let mut child = command
            .current_dir(self.directory.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.directory.path())
            .env("RAW_SEARCH_API_URL", &self.address)
            .env("RAW_SEARCH_ADMIN_API_KEY", KEY)
            .env("YT_DLP_BINARY", self.directory.path().join("yt-dlp"))
            .env(
                "RAW_SEARCH_BASE_CHUNKS_GZIP",
                self.directory.path().join("chunks.jsonl.gz"),
            )
            .env(
                "RAW_SEARCH_BASE_CHANNELS_GZIP",
                self.directory.path().join("channels.jsonl.gz"),
            )
            .env(
                "RAW_PUBLICATION_WORK_DIRECTORY",
                self.directory.path().join("publication"),
            )
            .env("RAW_PUBLICATION_MIN_FREE_BYTES", "0")
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("fixture CLI exceeded 30 seconds");
            }
            thread::sleep(Duration::from_millis(10));
        };
        Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }

    fn legacy_hits(&self) -> u64 {
        self.engine
            .search(&SearchParams {
                q: "legacytoken".to_owned(),
                ..SearchParams::default()
            })
            .unwrap()["found"]
            .as_u64()
            .unwrap()
    }

    async fn assert_retained_after_publication(&self, output: &Output) {
        assert_eq!(output.status.code(), Some(75), "{}", transcript(output));
        let pending = self.admin.channel_stats("fixture").await.unwrap();
        assert_eq!(pending.indexed_chunks, 2);
        assert_eq!(pending.indexed_videos, 1);
        self.publish();
        assert_eq!(self.legacy_hits(), 2, "{}", transcript(output));
        let published = self.admin.channel_stats("fixture").await.unwrap();
        assert_eq!(published, pending);
        let profile = self.admin.channel("fixture").await.unwrap().unwrap();
        assert_eq!(profile.indexed_chunk_count, 2);
        assert_eq!(profile.indexed_video_count, 1);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn write_corpus(root: &Path, name: &str, values: &[Value]) {
    let mut plain = fs::File::create(root.join(format!("{name}.jsonl"))).unwrap();
    let mut gzip = GzEncoder::new(
        fs::File::create(root.join(format!("{name}.jsonl.gz"))).unwrap(),
        Compression::fast(),
    );
    for value in values {
        writeln!(plain, "{value}").unwrap();
        writeln!(gzip, "{value}").unwrap();
    }
    gzip.finish().unwrap();
}

fn transcript(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_new_video_cap_preserves_stored_creator_and_chunks() {
    let fixture = Fixture::new(2).await;
    let output = fixture.index(&["--mode", "update", "--max-new-videos", "1"]);
    fixture.assert_retained_after_publication(&output).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_discovery_cap_preserves_stored_creator_and_chunks() {
    let fixture = Fixture::new(2).await;
    let output = fixture.index(&["--mode", "update", "--max-videos", "1"]);
    fixture.assert_retained_after_publication(&output).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ledger_deferred_capped_update_preserves_stored_creator_and_chunks() {
    let fixture = Fixture::new(2).await;
    let first = fixture.index(&["--mode", "update", "--max-videos", "1"]);
    fixture.assert_retained_after_publication(&first).await;
    let second = fixture.index(&["--mode", "update", "--max-videos", "1"]);
    assert!(transcript(&second).contains("Deferred 1 classified video failure(s)"));
    fixture.assert_retained_after_publication(&second).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_empty_creator_removes_its_unsearchable_profile_after_publication() {
    let fixture = Fixture::new(0).await;
    let output = fixture.index(&["--mode", "update", "--max-new-videos", "1"]);
    assert_eq!(output.status.code(), Some(75), "{}", transcript(&output));
    fixture.publish();
    assert_eq!(fixture.legacy_hits(), 0);
    assert_eq!(
        fixture
            .admin
            .channel_stats("fixture")
            .await
            .unwrap()
            .indexed_chunks,
        0
    );
    assert!(fixture.admin.channel("fixture").await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_acknowledged_rewipe_keeps_old_chunks_deleted_after_publication() {
    let fixture = Fixture::new(2).await;
    assert_eq!(fixture.legacy_hits(), 2);
    let output = fixture.index(&[
        "--mode",
        "rewipe",
        "--allow-destructive-rewipe",
        "--max-videos",
        "1",
    ]);
    assert_eq!(output.status.code(), Some(75), "{}", transcript(&output));
    assert!(transcript(&output).contains("No searchable transcript chunks were imported"));
    fixture.publish();
    eprintln!(
        "failed acknowledged rewipe: published legacy hits={}",
        fixture.legacy_hits()
    );
    assert_eq!(fixture.legacy_hits(), 0, "{}", transcript(&output));
    assert!(fixture.admin.channel("fixture").await.unwrap().is_none());
}
