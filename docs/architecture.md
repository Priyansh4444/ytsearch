# Architecture

`ytsearch` is split around replaceable boundaries so search, metadata, caption extraction, transcription, chunking, and storage can evolve independently.

## Crates

| Crate | Responsibility | Hotswap boundary |
| --- | --- | --- |
| `youtube_client` | Small typed wrapper around YouTube Data API v3 list endpoints. | Replace if moving to generated OpenAPI clients or another metadata provider. |
| `youtube_sources` | YouTube Data API metadata source plus metadata fallback composition. | Implements `VideoMetadataSource`; does not know about `yt-dlp`. |
| `youtube_ytdlp` | `yt-dlp` backed public caption transcript source. Parses `json3`/`vtt` into timestamped segments. | Implements `TranscriptSource`; can be replaced by any captions provider. |
| `youtube_transcribe` | Optional local audio fallback using `yt-dlp` audio extraction, `ffmpeg`, `ffprobe`, and `voxtype`. | Implements `TranscriptSource`; use only when captions are missing and you are allowed to transcribe. |
| `youtube_ingest` | Core domain model, ports/traits, and transcript chunker. | Defines `VideoMetadataSource`, `TranscriptSource`, `TranscriptChunkSource`, `ChunkSink`, and `EmbeddingSource`. |
| `youtube_time` | Dependency-free parsing for RFC 3339 publication timestamps and YouTube durations. | Keeps narrow time conversion logic shared without a general-purpose date-time dependency. |
| `youtube_search` | Shared query type, indexed creator document, and search backend trait. | API-facing hotswap boundary for search backends. |
| `youtube_typesense` | Typesense schema, admin import/delete/upsert, and search client. | Active backend; implements `ChunkSink` and `TranscriptSearchBackend`. |
| `youtube_cli` | Operator CLI for validation, indexing, direct video indexing, and CLI search. | Wires sources/sinks together; no core parsing logic should live here. |
| `youtube_api` | Axum HTTP API for frontend search and indexed channel listing. | Uses `youtube_search::TranscriptSearchBackend`; keep admin actions out of this service. |
| `youtube_semantic` | Experimental embedding/vector-search abstractions. | Keep optional until semantic search is production-ready. |

## Data flow

```mermaid
flowchart TD
    CLI[youtube_cli] --> YT[YouTube Data API via youtube_client]
    CLI --> META[youtube_sources metadata]
    CLI --> CAP[youtube_ytdlp captions]
    CLI --> VOX[youtube_transcribe optional fallback]
    META --> ING[youtube_ingest chunker]
    CAP --> ING
    VOX --> ING
    ING --> SEARCHADMIN[youtube_typesense admin import]
    API[youtube_api search-only API] --> SEARCH[youtube_search backend trait]
    SEARCH --> TYPESENSE[youtube_typesense search client]
    WEB[Solid frontend] --> API
```

## Transcript strategy

1. Fetch public captions with `youtube_ytdlp::YtDlpTranscriptSource`.
2. Parse captions into `TranscriptSegment` values.
3. Chunk into searchable windows with `youtube_ingest::Chunker`.
4. Optional fallback: `youtube_transcribe::VoxtypeTranscriptSource` only when captions are unavailable.

## UI structure

The frontend is intentionally split by cognitive load:

- `src/app.tsx` orchestrates route/channel/search state.
- `src/api.ts` owns HTTP calls.
- `src/types.ts` owns frontend API types.
- `src/utils.ts` owns formatting/routing/highlight helpers.
- `src/components/*` owns view components.

Avoid putting new feature logic directly into `app.tsx` unless it is app-level orchestration.
