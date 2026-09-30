use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use youtube_ingest::TranscriptChunk;

async fn read_request(socket: &mut TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let mut buffer = [0_u8; 4_096];
        let read = socket.read(&mut buffer).await.unwrap();
        assert!(read > 0);
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let body_start = header_end + 4;
        let content_length = String::from_utf8_lossy(&request[..header_end])
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if request.len() >= body_start + content_length {
            return String::from_utf8(request).unwrap();
        }
    }
}

async fn respond(socket: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await.unwrap();
}

fn chunk() -> TranscriptChunk {
    serde_json::from_value(serde_json::json!({
        "id": "video-1-0",
        "youtuber_id": "creator",
        "youtuber_handle": "@creator",
        "youtuber_name": "Creator",
        "channel_id": "UCcreator",
        "video_id": "video-1",
        "title": "Video",
        "text": "Transcript",
        "sequence": 0,
        "t_start_ms": 0,
        "t_end_ms": 1,
        "uploader_id": "creator"
    }))
    .unwrap()
}

#[tokio::test]
async fn replacement_uses_the_authenticated_raw_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        assert!(request.starts_with("PUT /admin/videos/video-1 HTTP/1.1"));
        assert!(request.contains("x-raw-admin-key: test-admin-key"));
        assert!(request.contains(r#""publication_api_version":1"#));
        assert!(request.contains(r#""idempotency_key":"replace-video-1""#));
        assert!(request.contains(r#""video_id":"video-1""#));
        respond(
            &mut socket,
            "202 Accepted",
            r#"{"id":"replace-video-1","sequence":1,"state":"Durable","acknowledged_readers":0,"error":null}"#,
        )
        .await;
    });
    let client = RawIndexAdminClient::new(RawIndexAdminConfig::new(
        format!("http://{address}"),
        "test-admin-key",
    ))
    .unwrap();
    let chunks = [chunk()];

    let operation = client
        .replace_video(
            VideoReplacement::new("video-1", &chunks).unwrap(),
            "replace-video-1",
        )
        .await
        .unwrap();

    assert_eq!(operation.sequence, Some(1));
    server.await.unwrap();
}

async fn assert_channel_deletion_query(scope: ChannelDeletionScope, expected: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let expected = expected.to_string();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        assert!(request.starts_with("DELETE /admin/channels/creator?"));
        assert!(request.contains("idempotency_key=delete-creator"));
        assert!(request.contains(&format!("cascade={expected}")));
        assert!(request.contains("publication_api_version=1"));
        respond(
            &mut socket,
            "202 Accepted",
            r#"{"id":"delete-creator","sequence":1,"state":"Durable","acknowledged_readers":0,"error":null}"#,
        )
        .await;
    });
    let client = RawIndexAdminClient::new(RawIndexAdminConfig::new(
        format!("http://{address}"),
        "test-admin-key",
    ))
    .unwrap();

    client
        .delete_channel("creator", scope, "delete-creator")
        .await
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn channel_deletion_scope_preserves_the_legacy_boolean_wire_contract() {
    assert_channel_deletion_query(ChannelDeletionScope::ChannelOnly, "false").await;
    assert_channel_deletion_query(ChannelDeletionScope::ChannelAndVideos, "true").await;
}

#[test]
fn status_mapping_preserves_retry_and_conflict_semantics() {
    assert!(matches!(
        map_status(StatusCode::CONFLICT, "duplicate".to_string()),
        IndexStoreError::Conflict(_)
    ));
    assert!(matches!(
        map_status(StatusCode::SERVICE_UNAVAILABLE, "busy".to_string()),
        IndexStoreError::Unavailable { .. }
    ));
    assert!(RawIndexAdminClient::new(RawIndexAdminConfig::new("file:///tmp", "key")).is_err());
}

#[test]
fn empty_error_responses_preserve_the_http_status() {
    let error = map_status(StatusCode::NOT_FOUND, String::new());

    assert_eq!(
        error.to_string(),
        "invalid index administration input: raw admin returned 404 Not Found"
    );
}

#[tokio::test]
async fn prepare_outlasts_the_admin_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let digest = "0".repeat(64);
    let body = serde_json::json!({
        "job_id": "00000000000000000001-0123456789abcdef",
        "target_sequence": 1,
        "snapshot_bytes": 4096,
        "snapshot_blake3": digest,
        "compatibility": {
            "generation_format": "raw-index-schema-7-v2",
            "corpus_identity": digest,
            "base_chunks_blake3": digest,
            "base_channels_blake3": digest
        },
        "status": "prepared",
        "completed_files": []
    })
    .to_string();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        assert!(request.starts_with("POST /admin/publications/prepare "));
        // Slower than the admin timeout, as a large snapshot export is.
        tokio::time::sleep(Duration::from_millis(300)).await;
        respond(&mut socket, "201 Created", &body).await;
    });
    let mut config = RawIndexAdminConfig::new(format!("http://{address}"), "secret")
        .with_publication_timeout(Duration::from_secs(10));
    config.timeout = Duration::from_millis(100);
    let client = RawIndexAdminClient::new(config).unwrap();
    let job = client
        .prepare_publication()
        .await
        .expect("prepare waits for the export");
    assert_eq!(job.target_sequence, 1);
    server.await.unwrap();
}

#[test]
fn publication_job_responses_are_validated_before_filesystem_use() {
    let digest = "0".repeat(64);
    let mut job = RawPublicationJob {
        job_id: "00000000000000000001-0123456789abcdef".to_string(),
        target_sequence: 1,
        snapshot_bytes: 4096,
        snapshot_blake3: digest.clone(),
        compatibility: crate::RawGenerationCompatibility {
            generation_format: "raw-index-schema-6-v1".to_string(),
            corpus_identity: digest.clone(),
            base_chunks_blake3: digest.clone(),
            base_channels_blake3: digest,
        },
        status: crate::RawPublicationStatus::Prepared,
        completed_files: std::collections::BTreeSet::new(),
    };
    validate_publication_job_response(&job).unwrap();

    job.job_id = "..".to_string();
    assert!(validate_publication_job_response(&job).is_err());
    job.job_id = "00000000000000000001-0123456789abcdef".to_string();
    job.completed_files.insert("manifest.json".to_string());
    assert!(validate_publication_job_response(&job).is_err());
}

#[tokio::test]
async fn publication_paths_reject_untrusted_components_before_network_io() {
    let client = RawIndexAdminClient::new(RawIndexAdminConfig::new(
        "http://127.0.0.1:1",
        "test-admin-key",
    ))
    .unwrap();

    assert!(matches!(
        client.publication_status("../escape").await,
        Err(IndexStoreError::InvalidInput(_))
    ));
    assert!(matches!(
        client.publication_status("..").await,
        Err(IndexStoreError::InvalidInput(_))
    ));
    assert!(matches!(
        client
            .upload_publication_file(
                "00000000000000000001-0123456789abcdef",
                "../escape",
                Path::new("missing")
            )
            .await,
        Err(IndexStoreError::InvalidInput(_))
    ));
    assert!(matches!(
        client
            .upload_publication_file(
                "00000000000000000001-0123456789abcdef",
                "manifest.json",
                Path::new("missing")
            )
            .await,
        Err(IndexStoreError::InvalidInput(_))
    ));
}
