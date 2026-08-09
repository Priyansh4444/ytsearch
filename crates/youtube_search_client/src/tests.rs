use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
fn query_contract_preserves_raw_ranking_parameters() {
    let mut url = Url::parse("https://raw.invalid/search").unwrap();
    append_search_query_params(
        &mut url,
        &TextSearchQuery::new("black hole")
            .query_by("text,title")
            .youtuber_handle("@veritasium")
            .window(20, 11),
    );
    let pairs = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        pairs.get("q").map(|value| value.as_ref()),
        Some("black hole")
    );
    assert_eq!(
        pairs.get("filter_by").map(|value| value.as_ref()),
        Some("youtuber_handle:=@veritasium")
    );
    assert_eq!(
        pairs.get("query_by_weights").map(|value| value.as_ref()),
        Some("4,6")
    );
    assert_eq!(pairs.get("offset").map(|value| value.as_ref()), Some("20"));
    assert_eq!(pairs.get("limit").map(|value| value.as_ref()), Some("11"));
}

#[test]
fn config_debug_redacts_keys_and_rejects_non_http_urls() {
    let config = SearchClientConfig::new("https://raw.invalid", "top-secret");
    let debug = format!("{config:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("top-secret"));
    assert!(SearchClient::new(SearchClientConfig::new("file:///tmp/index", "key")).is_err());
}

#[tokio::test]
async fn search_uses_the_raw_auth_header() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0_u8; 8_192];
        let read = socket.read(&mut request).await.unwrap();
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(
            request.starts_with("GET /collections/youtube_transcript_chunks/documents/search?")
        );
        assert!(request.contains("x-raw-search-key: raw-key"));
        let body = r#"{"found":0,"hits":[]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let client = SearchClient::new(SearchClientConfig::new(
        format!("http://{address}"),
        "raw-key",
    ))
    .unwrap();

    let result = client
        .search(&TextSearchQuery::new("blackhole"))
        .await
        .unwrap();

    assert_eq!(result["found"], 0);
    server.await.unwrap();
}
