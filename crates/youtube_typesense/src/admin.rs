use async_trait::async_trait;
use reqwest::StatusCode;
use std::{num::NonZeroU16, time::Duration};
use youtube_index_store::{
    BackendReadiness, ChannelDocument, ChunkStats, IndexOperation, IndexOperationState,
    IndexStoreError, SearchIndexAdmin, VideoIdPage, VideoReplacement,
};

use crate::{TypesenseAdminClient, TypesenseError};

#[async_trait]
impl SearchIndexAdmin for TypesenseAdminClient {
    async fn readiness(&self) -> Result<BackendReadiness, IndexStoreError> {
        self.wait_until_ready_with(1, Duration::ZERO)
            .await
            .map_err(map_error)?;
        Ok(BackendReadiness {
            backend: "typesense".to_string(),
            ready: true,
            generation: None,
            last_sequence: None,
            acknowledged_readers: 1,
        })
    }

    async fn replace_video(
        &self,
        replacement: VideoReplacement<'_>,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("video_id", replacement.video_id())?;
        validate_idempotency_key(idempotency_key)?;

        // Typesense has no atomic replace primitive. Preserve the existing
        // delete-then-import behavior only for the migration window; Raw will
        // make the equivalent operation durable atomically.
        self.delete_chunks_by_video_id(replacement.video_id())
            .await
            .map_err(map_error)?;
        self.import_chunks_batched(replacement.chunks())
            .await
            .map_err(map_error)?;

        Ok(published_operation(idempotency_key))
    }

    async fn delete_video(
        &self,
        video_id: &str,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("video_id", video_id)?;
        validate_idempotency_key(idempotency_key)?;
        self.delete_chunks_by_video_id(video_id)
            .await
            .map_err(map_error)?;
        Ok(published_operation(idempotency_key))
    }

    async fn upsert_channel(
        &self,
        channel: &ChannelDocument,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("channel id", &channel.id)?;
        validate_idempotency_key(idempotency_key)?;
        self.upsert_youtuber(channel).await.map_err(map_error)?;
        Ok(published_operation(idempotency_key))
    }

    async fn delete_channel(
        &self,
        channel_id: &str,
        cascade: bool,
        idempotency_key: &str,
    ) -> Result<IndexOperation, IndexStoreError> {
        validate_identifier("channel id", channel_id)?;
        validate_idempotency_key(idempotency_key)?;
        if cascade {
            self.delete_chunks_by_youtuber_id(channel_id)
                .await
                .map_err(map_error)?;
        }
        self.delete_youtuber(channel_id).await.map_err(map_error)?;
        Ok(published_operation(idempotency_key))
    }

    async fn operation(&self, operation_id: &str) -> Result<IndexOperation, IndexStoreError> {
        validate_idempotency_key(operation_id)?;
        Err(IndexStoreError::Unavailable {
            backend: "typesense",
            message: "synchronous operations have no durable status log".to_string(),
        })
    }

    async fn video_chunk_count(&self, video_id: &str) -> Result<u64, IndexStoreError> {
        validate_identifier("video_id", video_id)?;
        self.count_chunks_by_video_id(video_id)
            .await
            .map_err(map_error)
    }

    async fn indexed_video_ids(
        &self,
        channel_id: &str,
        cursor: Option<&str>,
        limit: NonZeroU16,
    ) -> Result<VideoIdPage, IndexStoreError> {
        validate_identifier("channel id", channel_id)?;
        if let Some(cursor) = cursor {
            validate_identifier("video cursor", cursor)?;
        }

        let video_ids = self
            .indexed_video_ids_vec_by_youtuber_id(channel_id)
            .await
            .map_err(map_error)?;
        Ok(paginate_video_ids(video_ids, cursor, limit))
    }

    async fn channel_stats(&self, channel_id: &str) -> Result<ChunkStats, IndexStoreError> {
        validate_identifier("channel id", channel_id)?;
        self.chunk_stats_by_youtuber_id(channel_id)
            .await
            .map_err(map_error)
    }
}

fn validate_identifier(label: &str, value: &str) -> Result<(), IndexStoreError> {
    if value.is_empty() {
        return Err(IndexStoreError::InvalidInput(format!(
            "{label} must not be empty"
        )));
    }
    if value.len() > 256 {
        return Err(IndexStoreError::InvalidInput(format!(
            "{label} exceeds 256 bytes"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_' | b'-' | b'.'))
    {
        return Err(IndexStoreError::InvalidInput(format!(
            "{label} contains an unsupported character"
        )));
    }
    Ok(())
}

fn validate_idempotency_key(value: &str) -> Result<(), IndexStoreError> {
    if value.is_empty() {
        return Err(IndexStoreError::InvalidInput(
            "idempotency key must not be empty".to_string(),
        ));
    }
    if value.len() > 256 || !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(IndexStoreError::InvalidInput(
            "idempotency key must be at most 256 visible ASCII bytes".to_string(),
        ));
    }
    Ok(())
}

fn published_operation(idempotency_key: &str) -> IndexOperation {
    IndexOperation {
        id: idempotency_key.to_string(),
        sequence: None,
        state: IndexOperationState::Published,
        acknowledged_readers: 1,
        error: None,
    }
}

fn paginate_video_ids(
    mut video_ids: Vec<String>,
    cursor: Option<&str>,
    limit: NonZeroU16,
) -> VideoIdPage {
    video_ids.sort_unstable();
    let start = cursor.map_or(0, |cursor| {
        video_ids.partition_point(|video_id| video_id.as_str() <= cursor)
    });
    let end = start
        .saturating_add(usize::from(limit.get()))
        .min(video_ids.len());
    let next_cursor = (end < video_ids.len()).then(|| video_ids[end - 1].clone());
    let video_ids = video_ids
        .into_iter()
        .skip(start)
        .take(end - start)
        .collect();

    VideoIdPage {
        video_ids,
        next_cursor,
    }
}

fn map_error(error: TypesenseError) -> IndexStoreError {
    let message = error.to_string();
    match error {
        TypesenseError::InvalidUrl(_) => IndexStoreError::InvalidInput(message),
        TypesenseError::Api { status, .. } if status == StatusCode::CONFLICT => {
            IndexStoreError::Conflict(message)
        }
        TypesenseError::Http(_)
        | TypesenseError::RetryExhausted
        | TypesenseError::Unavailable(_)
        | TypesenseError::Api {
            status: StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS,
            ..
        } => IndexStoreError::unavailable("typesense", message),
        TypesenseError::Api { status, .. } if status.is_server_error() => {
            IndexStoreError::unavailable("typesense", message)
        }
        TypesenseError::Json(_)
        | TypesenseError::ImportFailures { .. }
        | TypesenseError::Api { .. } => IndexStoreError::internal("typesense", message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TypesenseAdminConfig;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };
    use youtube_ingest::TranscriptChunk;

    async fn read_request(socket: &mut TcpStream) -> String {
        let mut request = Vec::new();
        loop {
            let mut buffer = [0_u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0, "connection closed before request completed");
            request.extend_from_slice(&buffer[..read]);

            if let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
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
    }

    async fn respond(socket: &mut TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    fn chunk(video_id: &str) -> TranscriptChunk {
        serde_json::from_value(serde_json::json!({
            "id": format!("{video_id}-0"),
            "youtuber_id": "channel",
            "youtuber_handle": "@channel",
            "youtuber_name": "Channel",
            "channel_id": "UC123",
            "video_id": video_id,
            "title": "Title",
            "text": "Transcript",
            "sequence": 0,
            "t_start_ms": 0,
            "t_end_ms": 1,
            "uploader_id": "channel"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn readiness_reports_the_single_typesense_reader() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut socket).await;
            respond(&mut socket, r#"{"ok":true}"#).await;
        });
        let client = TypesenseAdminClient::new(TypesenseAdminConfig::new(
            format!("http://{address}"),
            "test-key",
        ));

        let readiness = SearchIndexAdmin::readiness(&client).await.unwrap();

        assert!(readiness.ready);
        assert_eq!(readiness.backend, "typesense");
        assert_eq!(readiness.acknowledged_readers, 1);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn replacement_deletes_stale_chunks_before_importing() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut delete_socket, _) = listener.accept().await.unwrap();
            let delete_request = read_request(&mut delete_socket).await;
            assert!(delete_request.starts_with(
                "DELETE /collections/youtube_transcript_chunks/documents?filter_by=video_id%3A%3Dvideo-1"
            ));
            respond(&mut delete_socket, r#"{"num_deleted":1}"#).await;

            let (mut import_socket, _) = listener.accept().await.unwrap();
            let import_request = read_request(&mut import_socket).await;
            assert!(import_request.starts_with(
                "POST /collections/youtube_transcript_chunks/documents/import?action=upsert"
            ));
            assert!(import_request.contains(r#""video_id":"video-1""#));
            respond(&mut import_socket, "{\"success\":true}\n").await;
        });
        let client = TypesenseAdminClient::new(TypesenseAdminConfig::new(
            format!("http://{address}"),
            "test-key",
        ));
        let chunks = [chunk("video-1")];
        let replacement = VideoReplacement::new("video-1", &chunks).unwrap();

        let operation = SearchIndexAdmin::replace_video(&client, replacement, "replace-video-1")
            .await
            .unwrap();

        assert_eq!(operation.state, IndexOperationState::Published);
        assert_eq!(operation.id, "replace-video-1");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn operation_lookup_fails_closed_without_a_durable_log() {
        let client =
            TypesenseAdminClient::new(TypesenseAdminConfig::new("http://127.0.0.1:1", "test-key"));

        let error = SearchIndexAdmin::operation(&client, "repair-123")
            .await
            .unwrap_err();

        assert!(matches!(error, IndexStoreError::Unavailable { .. }));
    }

    #[test]
    fn identifiers_reject_typesense_filter_grammar() {
        assert!(validate_identifier("video_id", "-valid_Id.1").is_ok());
        assert!(matches!(
            validate_identifier("video_id", "video:=[*]"),
            Err(IndexStoreError::InvalidInput(_))
        ));
        assert!(matches!(
            validate_idempotency_key("contains whitespace"),
            Err(IndexStoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn pages_are_stable_and_cursor_exclusive() {
        let limit = NonZeroU16::new(2).unwrap();
        let first = paginate_video_ids(
            vec![
                "video-c".to_string(),
                "video-a".to_string(),
                "video-b".to_string(),
            ],
            None,
            limit,
        );
        assert_eq!(first.video_ids, ["video-a", "video-b"]);
        assert_eq!(first.next_cursor.as_deref(), Some("video-b"));

        let second = paginate_video_ids(
            vec![
                "video-c".to_string(),
                "video-a".to_string(),
                "video-b".to_string(),
            ],
            first.next_cursor.as_deref(),
            limit,
        );
        assert_eq!(second.video_ids, ["video-c"]);
        assert_eq!(second.next_cursor, None);
    }
}
