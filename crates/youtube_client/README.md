# youtube_client

Internal, narrow YouTube Data API v3 client: `list` requests for the four
resources the indexer uses (`channels`, `playlistItems`, `search` and `videos`).
Responses stay schema-light as `serde_json::Value`; deserialize into local
structs where a caller needs a stable shape.

```rust
use youtube_client::{ListResource, Query, YouTubeClient};

# async fn example() -> youtube_client::Result<()> {
let youtube = YouTubeClient::with_api_key(std::env::var("YOUTUBE_API_KEY").unwrap())?;

let page = youtube
    .list(ListResource::Videos)
    .query(
        Query::new()
            .part(["snippet", "contentDetails", "statistics", "status"])
            .ids(["dQw4w9WgXcQ"])
            .fields("items(id,snippet/title,statistics),nextPageToken"),
    )
    .page()
    .await?;

println!("{:#?}", page.items);
# Ok(())
# }
```
