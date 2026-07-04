# Deployment guide

This project has three deployable concerns:

1. **Frontend**: static Solid/Vite app.
2. **Search API**: `youtube_api` Axum service using the Typesense search-only key.
3. **Indexer**: `youtube_cli` one-shot/cron job using YouTube API + Typesense admin key + optional local tools.

Keep the indexer separate from the public API. The API should never receive the Typesense admin key.

## Recommended production setup

| Concern | Recommended host | Why |
| --- | --- | --- |
| Frontend | Cloudflare Pages | Fast static hosting, preview deploys, cheap CDN. |
| API | Fly.io or Railway | Long-running Rust service with private env vars. |
| Indexing | Railway cron/job or Fly machine run manually | Needs `yt-dlp`; may run longer than edge/serverless limits. |
| Search DB | Typesense Cloud | Already supported. |
| Errors/logs | Sentry + platform logs; optionally Axiom for Rust logs | Sentry for exceptions, Axiom for structured logs. |
| Analytics | PostHog on frontend | Product analytics without mixing into backend search logs. |

## Environment variables

### API runtime

```env
TYPESENSE_API_URL=...
TYPESENSE_SEARCH_API_KEY=...
TYPESENSE_COLLECTION=youtube_transcript_chunks
YTSEARCH_API_ADDR=0.0.0.0:8787
RUST_LOG=youtube_api=info,tower_http=info
```

### Indexer runtime

```env
YOUTUBE_API_KEY=...
TYPESENSE_API_URL=...
TYPESENSE_ADMIN_API_KEY=...
TYPESENSE_COLLECTION=youtube_transcript_chunks
RUST_LOG=youtube_cli=info,youtube_sources=info,youtube_ytdlp=info,youtube_transcribe=info
```

Optional transcription fallback also needs `yt-dlp`, `ffmpeg`, `ffprobe`, and `voxtype` available in the image/machine.

## Cloudflare Pages frontend

Build command:

```bash
bun install --frozen-lockfile
bun run build
```

Output directory:

```text
dist
```

Set a Pages environment variable if the API is on a separate origin:

```env
VITE_API_BASE_URL=https://api.example.com
```

If using the current relative `/api` calls, add a Cloudflare Pages redirect/proxy rule or host frontend and API behind the same domain.

## Fly.io API

Example `Dockerfile` for API:

```Dockerfile
FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p youtube_api

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/youtube-api /usr/local/bin/youtube-api
ENV YTSEARCH_API_ADDR=0.0.0.0:8787
EXPOSE 8787
CMD ["youtube-api"]
```

Deploy:

```bash
fly launch --name ytsearch-api --dockerfile Dockerfile --no-deploy
fly secrets set TYPESENSE_API_URL='...' TYPESENSE_SEARCH_API_KEY='...'
fly deploy
```

## Railway API

Use the same Dockerfile or Railway Rust/Nixpacks builder.

Start command:

```bash
./target/release/youtube-api
```

Set:

```env
YTSEARCH_API_ADDR=0.0.0.0:${PORT}
TYPESENSE_API_URL=...
TYPESENSE_SEARCH_API_KEY=...
```

If Railway does not expand `${PORT}` inside env reliably, set the service start command with the literal provided port or adapt the API to read `PORT` as fallback.

## Indexing jobs

Manual full/resume indexing:

```bash
cargo run --release -p youtube_cli -- index-youtuber @t3dotgg --no-wipe --skip-existing --jobs 8
```

Destructive clean reindex:

```bash
cargo run --release -p youtube_cli -- index-youtuber @t3dotgg --jobs 8
```

Single video:

```bash
cargo run --release -p youtube_cli -- index-video 'https://www.youtube.com/watch?v=yfxDdQo2cyI'
```

Optional local fallback:

```bash
cargo run --release -p youtube_cli -- index-youtuber @t3dotgg --transcribe-missing --jobs 2
```

Use lower `--jobs` with transcription fallback because local audio transcription is CPU/GPU heavy.

## Deployment permutations

| Frontend | API | Indexer | Notes |
| --- | --- | --- | --- |
| Cloudflare Pages | Fly.io | Railway job | Best default: CDN frontend, stable API, easy scheduled indexing. |
| Cloudflare Pages | Railway | Railway job | Simplest all-dashboard setup. |
| Cloudflare Pages | Fly.io | Fly manual machine | Good if you prefer one Rust-oriented runtime. |
| Railway static | Railway API | Railway job | Easiest but less CDN-focused. |
| Cloudflare Pages | Cloudflare Workers | Not recommended for current Rust API | Current Axum API is not Worker-native. Use a Worker only as a reverse proxy. |

## Observability

### Logs to Axiom

Recommended low-friction path: keep structured `tracing` logs on stdout and configure Fly/Railway log drains to Axiom. This avoids hardcoding log exporters in app code.

Useful log filter:

```env
RUST_LOG=youtube_api=info,tower_http=info,youtube_cli=info,youtube_ytdlp=info,youtube_transcribe=info
```

### Sentry

Recommended next step: add `sentry` + `sentry-tracing` to `youtube_api` and initialize from `SENTRY_DSN`. Do not block deploy on this if platform logs are already drained.

### PostHog

Add PostHog only in the frontend. Track coarse events such as:

- search submitted
- channel selected
- result opened
- snippet selected

Do not send transcript text unless you have a clear privacy/legal basis.

## Production checklist

```bash
cargo fmt --all --check
cargo test
cargo clippy --all-targets --all-features
bun run format:check
bun run lint
bun run build
```

Operational checks:

```bash
curl 'https://api.example.com/api/health'
curl 'https://api.example.com/api/youtubers'
curl 'https://api.example.com/api/search?q=react&youtuber=@t3dotgg&per_page=3'
```
