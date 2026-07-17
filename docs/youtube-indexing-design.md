# YouTube Transcript Indexing Design

## Goal

Build a pipeline that can index YouTubers, their videos, and searchable transcript
chunks. The first search backend is Typesense for raw text search, but the code
should allow future backends such as a custom search server or a semantic vector
store.

The important rule is that ingestion produces stable documents first. Publishing
those documents to Typesense, a custom server, or a vector database is a separate
step.

## Data Model

The primary search unit is a `TranscriptChunk`.

Each chunk belongs to:

- One YouTuber.
- One YouTube channel.
- One YouTube video.
- One timestamp range inside that video.

The first version uses 5-second windows. This keeps search results precise enough
to jump directly into the moment where someone said something.



Core fields:

```text
id                "{video_id}_{sequence}"
youtuber_id       stable local id, e.g. "t3dotgg"
youtuber_handle   YouTube handle, e.g. "@t3dotgg"
youtuber_name     display name
channel_id        YouTube channel id
video_id          YouTube video id
title             video title
text              transcript text for this chunk
prev_text         previous chunk text for context
next_text         next chunk text for context
sequence          zero-based chunk index
t_start_ms        chunk start timestamp
t_end_ms          chunk end timestamp
thumbnail         best available video thumbnail
published_at      API timestamp
upload_date       compact date for compatibility with existing docs
embedding         optional vector for semantic search
```

This matches the shape found in `temp_research/typesense_response.json`, while
adding YouTuber-level fields for filtering and grouping.

## Crate Boundaries

### `youtube_client`

Owns YouTube Data API calls.

Responsibilities:

- Resolve handles.
- Fetch channel data.
- Fetch video metadata.
- Fetch playlist uploads.

It should not know about Typesense, transcript chunking, or embeddings.

### `youtube_ingest`

Owns searchable document generation.

Responsibilities:

- Normalize transcript segments.
- Build 5-second transcript chunks.
- Attach video and YouTuber metadata.
- Return clear ingestion errors.

It defines traits for swappable infrastructure:

- `VideoMetadataSource`
- `TranscriptSource`
- `ChunkSink`
- `EmbeddingSource`

This keeps the pipeline replaceable without making callers learn many small
types.

### `youtube_sources`

Owns concrete source adapters for ingestion ports.

Responsibilities:

- Implement `VideoMetadataSource` with YouTube Data API metadata via `youtube_client`.
- Compose metadata and transcript fallbacks in priority order.
- Keep provider-specific parsing outside the ingest core.

This crate may later split into provider-specific crates, such as
`youtube_source_ytdlp` or `youtube_source_whisper`, if the adapters grow large.

### `youtube_typesense`

Owns Typesense collection creation, document import, and grouped text queries.

Responsibilities:

- Define the Typesense schema.
- Import chunks in batches.
- Run grouped text queries.

This currently uses plain `reqwest` because Typesense import is a simple HTTP
boundary.

### `youtube_semantic`

Owns embedding helpers and semantic/vector search boundaries.

Responsibilities:

- Provide `EmbeddingSource` implementations.
- Store vectors.
- Query vectors.
- Return chunk ids or full chunks.

This depends on `youtube_ingest`, not the other way around.

## Error Handling

Errors should describe what failed and which fallback is sensible.

Current error categories:

- `MetadataUnavailable`: YouTube API metadata failed.
- `TranscriptUnavailable`: transcript provider failed.
- `EmptyTranscript`: transcript exists but has no usable text.
- `Sink`: publishing failed.

The pipeline should not hide these errors. A caller can match them and choose a
fallback:

```rust
match ingest.ingest_video(video_id).await {
    Ok(chunks) => publish(chunks).await?,
    Err(IngestError::MetadataUnavailable { .. }) => try_yt_dlp_metadata(video_id).await?,
    Err(IngestError::TranscriptUnavailable { .. }) => run_local_transcriber(video_id).await?,
    Err(IngestError::EmptyTranscript { .. }) => run_local_transcriber(video_id).await?,
    Err(error) => return Err(error.into()),
}
```

This makes failures explicit without permanently coupling the crate to every
fallback strategy.

## Transcript Fallback Strategy

Start with the cheapest and most reliable source, then fall back:

1. YouTube captions or transcript endpoint.
2. `yt-dlp` metadata/caption extraction.
3. Local speech-to-text model.
4. Manual or custom repair path.

Each fallback should implement `TranscriptSource`. The indexer can then compose
providers in priority order without changing the chunker.

## Metadata Fallback Strategy

Start with YouTube Data API:

1. `videos.list` for title, duration, channel, thumbnails, publish date.
2. `channels.list` for handle and channel-level data.
3. `yt-dlp` if the API fails or quota is exhausted.
4. Cached local metadata if both network paths fail.

Each fallback should implement `VideoMetadataSource`.

## Typesense Text Search

Collection name:

```text
youtube_transcript_chunks
```

Search fields:

```text
text,title,youtuber_name,youtuber_handle
```

Filter/facet fields:

```text
youtuber_id,youtuber_handle,channel_id,video_id,upload_date,published_at
```

Recommended query shape:

```json
{
  "q": "react server components",
  "query_by": "text,title,youtuber_name",
  "filter_by": "youtuber_handle:=@t3dotgg",
  "group_by": "video_id",
  "group_limit": 5
}
```

Grouped results let the UI show videos first and matching timestamp snippets
inside each video.

## Semantic Search

Semantic search should be an optional second index, not a replacement for raw
text search.

Recommended approach:

1. Keep `TranscriptChunk.id` as the shared join key.
2. Store raw searchable text in Typesense.
3. Store embeddings either:
   - in Typesense if vector search is good enough for the use case,
   - in a dedicated vector database,
   - or in a custom server later.
4. Query both systems and merge results by chunk id.

Why separate text and semantic search:

- Text search is precise for names, code identifiers, quotes, and exact terms.
- Semantic search is better for vague memory, paraphrases, and concepts.
- Keeping both lets the UI offer hybrid results.

The `EmbeddingSource` trait is intentionally small:

```rust
async fn embed_chunks(&self, chunks: &mut [TranscriptChunk]) -> Result<()>;
```

That supports OpenAI embeddings, local models, or a custom embedding service
without changing the ingest crate.

## Pipeline

Single video:

1. Fetch video metadata.
2. Fetch transcript.
3. Normalize transcript.
4. Build 5-second chunks.
5. Optionally add embeddings.
6. Write JSONL backup.
7. Upsert to text search.
8. Upsert to semantic search.

Channel indexing:

1. Resolve YouTuber handle to channel id.
2. Read the channel uploads playlist.
3. Compare upload video ids against already-indexed video ids.
4. Ingest only new or changed videos.
5. Record per-video ingest status.

## Operational Notes

Keep a local JSONL export even when publishing to a hosted search backend. It gives you:

- Reproducible imports.
- Easier debugging.
- A migration path to another backend.
- A backup if a provider returns different data later.

Indexing should be idempotent. Re-running the same video should produce the same
chunk ids and replace the same documents.

## Current Implementation

The workspace is split into focused crates:

- `youtube_client`: internal YouTube Data API v3 wrapper.
- `youtube_ingest`: stable document model, 5-second chunker, ingest orchestration, and ports.
- `youtube_sources`: concrete metadata source using `youtube_client`, fallback source composition, and in-memory transcript source for tests/dev.
- `youtube_typesense`: Typesense schema, document import, grouped text query helper, and `ChunkSink` implementation.
- `youtube_semantic`: embedding source helpers plus an in-memory vector store boundary.

The `youtube_ingest` crate provides:

- `TranscriptChunk`
- `VideoMetadata`
- `Youtuber`
- `TranscriptSegment`
- `Chunker::five_seconds()`
- `VideoIngestor`
- Swappable source/sink traits
- Unit tests for chunking and clear empty-transcript errors

Next implementation step:

Add production transcript adapters:

- YouTube caption/transcript source.
- `yt-dlp` transcript fallback.
- Optional local speech-to-text fallback.
