#![cfg(unix)]

use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

#[derive(Debug)]
struct StoredChannel {
    chunks: u64,
    profile: bool,
    deletions: Vec<String>,
}

struct Fixture {
    directory: tempfile::TempDir,
    address: String,
    stored: Arc<Mutex<StoredChannel>>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new(chunks: u64) -> Self {
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

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stored = Arc::new(Mutex::new(StoredChannel {
            chunks,
            profile: chunks > 0,
            deletions: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let server = {
            let stored = Arc::clone(&stored);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => respond(stream, &stored),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("fixture accept failed: {error}"),
                    }
                }
            })
        };
        Self {
            directory,
            address,
            stored,
            stop,
            server: Some(server),
        }
    }

    fn update(&self, cap: &str) -> Output {
        let stdout = self.directory.path().join("stdout");
        let stderr = self.directory.path().join("stderr");
        let mut child = Command::new(env!("CARGO_BIN_EXE_ytsearch"))
            .args([
                "index-youtuber",
                "@fixture",
                "--mode",
                "update",
                cap,
                "1",
                "--jobs",
                "1",
                "--defer-publish",
                "--transcript-cache-dir",
            ])
            .arg(self.directory.path().join("cache"))
            .current_dir(self.directory.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.directory.path())
            .env("RAW_SEARCH_API_URL", &self.address)
            .env("RAW_SEARCH_ADMIN_API_KEY", "local-fixture-only")
            .env("YT_DLP_BINARY", self.directory.path().join("yt-dlp"))
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("fixture CLI exceeded 20 seconds");
            }
            thread::sleep(Duration::from_millis(10));
        };
        Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }

    fn assert_retained(&self, output: &Output) {
        assert_eq!(output.status.code(), Some(75), "{}", transcript(output));
        let stored = self.stored.lock().unwrap();
        assert_eq!(stored.chunks, 2, "{stored:?}\n{}", transcript(output));
        assert!(stored.profile, "{stored:?}\n{}", transcript(output));
        assert!(stored.deletions.is_empty(), "{stored:?}");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.server.take().unwrap().join().unwrap();
    }
}

fn transcript(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn respond(mut stream: TcpStream, stored: &Mutex<StoredChannel>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(&stream);
    let mut request = String::new();
    reader.read_line(&mut request).unwrap();
    let mut length = 0;
    loop {
        let mut header = String::new();
        assert!(
            reader.read_line(&mut header).unwrap() > 0,
            "incomplete request"
        );
        if header == "\r\n" {
            break;
        }
        if let Some((key, value)) = header.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap();
    let path = parts.next().unwrap();
    let route = path.split('?').next().unwrap();
    let mut stored = stored.lock().unwrap();
    let value: Value = match (method, route) {
        ("GET", "/admin/readiness") => json!({
            "backend":"raw", "ready":true, "generation":"fixture", "last_sequence":1,
            "acknowledged_readers":1, "publication_api_version":1,
        }),
        ("GET", route) if route.ends_with("/stats") => json!({
            "indexed_videos": u64::from(stored.chunks > 0), "indexed_chunks": stored.chunks,
        }),
        ("GET", route) if route.ends_with("/chunk-count") => json!({
            "count": if route.contains("old00000001") { stored.chunks } else { 0 },
        }),
        ("GET", route) if route.ends_with("/video-ids") => json!({
            "video_ids": if stored.chunks > 0 { vec!["old00000001"] } else { vec![] },
            "next_cursor": null,
        }),
        ("GET", "/admin/channels/fixture") if stored.profile => json!({
            "id":"fixture", "handle":"@fixture", "name":"Fixture", "channel_id":"UCfixture",
            "uploads_playlist":"UUfixture", "public_video_count":2,
            "indexed_video_count":u64::from(stored.chunks > 0), "indexed_chunk_count":stored.chunks,
        }),
        ("GET", "/admin/channels/fixture") => Value::Null,
        ("PUT", "/admin/channels/fixture") => {
            stored.profile = true;
            json!({"id":"fixture-upsert", "sequence":1, "state":"Durable", "acknowledged_readers":1, "error":null})
        }
        ("DELETE", "/admin/channels/fixture") => {
            stored.deletions.push(path.to_owned());
            stored.profile = false;
            if path.contains("cascade=true") {
                stored.chunks = 0;
            }
            json!({"id":"fixture-delete", "sequence":2, "state":"Durable", "acknowledged_readers":1, "error":null})
        }
        _ => panic!("unexpected request {method} {path}"),
    };
    let body = serde_json::to_vec(&value).unwrap();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
}

#[test]
fn failed_new_video_cap_preserves_stored_creator_and_chunks() {
    let fixture = Fixture::new(2);
    fixture.assert_retained(&fixture.update("--max-new-videos"));
}

#[test]
fn failed_discovery_cap_preserves_stored_creator_and_chunks() {
    let fixture = Fixture::new(2);
    fixture.assert_retained(&fixture.update("--max-videos"));
}

#[test]
fn ledger_deferred_capped_update_preserves_stored_creator_and_chunks() {
    let fixture = Fixture::new(2);
    let _first = fixture.update("--max-videos");
    let second = fixture.update("--max-videos");
    assert!(transcript(&second).contains("Deferred 1 classified video failure(s)"));
    fixture.assert_retained(&second);
}

#[test]
fn failed_empty_creator_removes_only_its_unsearchable_profile() {
    let fixture = Fixture::new(0);
    let output = fixture.update("--max-new-videos");
    assert_eq!(output.status.code(), Some(75), "{}", transcript(&output));
    let stored = fixture.stored.lock().unwrap();
    assert_eq!(stored.chunks, 0);
    assert!(!stored.profile, "{stored:?}");
    assert_eq!(stored.deletions.len(), 1, "{stored:?}");
    assert!(stored.deletions[0].contains("cascade=false"), "{stored:?}");
}
