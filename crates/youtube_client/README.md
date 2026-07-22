# youtube_client

Internal YouTube Data API v3 client.

The crate covers the current resource and method families exposed by Google's
YouTube Data API v3 discovery document while keeping responses schema-light.
Use `serde_json::Value` for broad coverage, or deserialize into local structs
when a caller needs a stable shape.

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
