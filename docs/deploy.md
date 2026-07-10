# Deployment guide

This project has three deployable concerns:

1. **Frontend**: static Solid/Vite app.
2. **Search API**: `youtube_api` Axum service using the Meilisearch search key.
3. **Indexer**: `youtube_cli` one-shot/cron job using YouTube API + Meilisearch master/admin key + optional local tools.

The production pattern is to keep indexing separate from the public API. Website-triggered indexing is intentionally unavailable; use the local CLI or a private operator job to queue indexing work.

## Recommended production setup

| Concern | Recommended host | Why |
| --- | --- | --- |
| Frontend | Cloudflare Pages | Fast static hosting, preview deploys, cheap CDN. |
| API | Fly.io or Railway | Long-running Rust service with private env vars. |
| Indexing | Railway cron/job or Fly machine run manually | Needs `yt-dlp`; may run longer than edge/serverless limits. |
| Search DB | Meilisearch on Fly.io | Self-hosted, persistent volume, low-latency colocated with the API. |
| Errors/logs | Sentry + platform logs; optionally Axiom for Rust logs | Sentry for exceptions, Axiom for structured logs. |
| Analytics | PostHog on frontend | Product analytics without mixing into backend search logs. |

## Environment variables

### API runtime

```env
MEILI_HTTP_ADDR=...
MEILI_SEARCH_API_KEY=...
MEILI_TRANSCRIPTS_INDEX=youtube_transcript_chunks
YTSEARCH_SEARCH_BACKEND=meilisearch
YTSEARCH_API_ADDR=0.0.0.0:8787
RUST_LOG=youtube_api=info,tower_http=info
LOG_FORMAT=json
YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE=120
```

### Indexer runtime

```env
# Optional yt-dlp fallback. Set YOUTUBE_API_FALLBACK=1 when uploading it.
YOUTUBE_API_KEY=...
MEILI_HTTP_ADDR=...
MEILI_MASTER_KEY=...
MEILI_TRANSCRIPTS_INDEX=youtube_transcript_chunks
YTSEARCH_SEARCH_BACKEND=meilisearch
RUST_LOG=youtube_cli=info,youtube_sources=info,youtube_ytdlp=info,youtube_transcribe=info
LOG_FORMAT=json
```

Optional transcription fallback also needs `yt-dlp`, `ffmpeg`, `ffprobe`, and `voxtype` available in the image/machine.



## Cloudflare Pages frontend

Build command:

```bash
bun install --frozen-lockfile
bun run frontend:check
```

Manual Wrangler deploy from this repo:

```bash
export CLOUDFLARE_API_TOKEN='...'
export CLOUDFLARE_PAGES_PROJECT=ytsearch
export VITE_API_BASE_URL=https://ytsearch-api-meili-staging.fly.dev
bun run deploy:frontend
```

`bun run deploy:frontend` reads `.env`, runs `typecheck`, format check, lint, and build before calling `wrangler pages deploy dist`.

Output directory:

```text
dist
```

The repo also includes tracked hook templates under `scripts/hooks/`. Install them locally with:

```bash
bun run hooks:install
```

- `pre-commit`: blocks commits if frontend typecheck/format/lint/build fails.
- `post-commit`: deploys `dist` to Cloudflare Pages using env from `.env`.

To skip deploy for a local commit:

```bash
YTSEARCH_SKIP_DEPLOY_ON_COMMIT=1 git commit ...
```

Set a Pages environment variable if the API is on a separate origin:

```env
VITE_API_BASE_URL=https://api.example.com
```

If using the current relative `/api` calls, add a Cloudflare Pages redirect/proxy rule or host frontend and API behind the same domain.

## Commit messages

Use this format for commits moving forward:

```text
<area of improvement>: <change>
```

Examples:

```text
search backend: add Meilisearch staging adapter
deployment: add Fly staging scripts
observability: send query events to Axiom
```

## Fly.io Meilisearch staging

Meilisearch is a persistent HTTP server backed by local disk. On Fly, run it as a separate app with a volume in the same region as the API. The included `fly.meilisearch.toml` uses the official Meilisearch Docker image, `/meili_data` storage, production mode, and a health check on `/health`.

```bash
export FLYIO_TOKEN='...'
export MEILI_MASTER_KEY='...'
scripts/fly-deploy-meili.sh
```

Set `MEILI_MASTER_KEY` in your shell or `.env`; the deploy script reads both through `scripts/load-env.sh` and stores the value as a Fly secret without printing it.

After the app is healthy, create a restricted search key in Meilisearch for production. For staging, `scripts/fly-set-meili-api-secrets.sh` will use `MEILI_SEARCH_API_KEY` when set, or fall back to `MEILI_MASTER_KEY` when `MEILI_SEARCH_API_KEY` is absent. Then point the API staging app at Meilisearch:

```bash
export MEILI_HTTP_ADDR=https://ytsearch-meili-staging.fly.dev
export MEILI_SEARCH_API_KEY='...' # optional for staging if MEILI_MASTER_KEY is set
APP_NAME=ytsearch-api-meili-staging scripts/fly-set-meili-api-secrets.sh
flyctl deploy --config fly.staging.toml --app ytsearch-api-meili-staging --primary-region sjc
```

Index into staging with:

```bash
MEILI_HTTP_ADDR=https://ytsearch-meili-staging.fly.dev \
MEILI_MASTER_KEY='...' \
cargo run --release -p youtube_cli -- init-typesense

MEILI_HTTP_ADDR=https://ytsearch-meili-staging.fly.dev \
MEILI_MASTER_KEY='...' \
cargo run --release -p youtube_cli -- index-youtuber @t3dotgg --max-videos 1 --no-wipe
```

## Fly.io Typesense staging

This branch includes `fly.typesense.toml` for a single-node self-hosted
Typesense staging app. It uses the official Typesense Docker image, persistent
storage at `/typesense-data`, and a health check on `/health`.

```bash
export FLYIO_TOKEN='...'
export TYPESENSE_SELFHOST_API_KEY='...' # generate with: openssl rand -hex 32
scripts/fly-deploy-typesense.sh
```

Point the API or CLI at it with:

```env
YTSEARCH_SEARCH_BACKEND=typesense
TYPESENSE_API_URL=https://ytsearch-typesense-staging.fly.dev
TYPESENSE_ADMIN_API_KEY=...
TYPESENSE_SEARCH_API_KEY=...
```

The self-hosted Typesense API app is separate from the engine app:

```bash
scripts/fly-set-typesense-selfhost-api-secrets.sh
scripts/fly-deploy-typesense-api.sh
curl 'https://ytsearch-api-typesense-fly-staging.fly.dev/api/health'
```

For UI comparison, deploy one Cloudflare Pages preview per backend:

```bash
scripts/cloudflare-pages-deploy-backends.sh
```

Default preview branches:

- `meili`
- `typesense`

## Fly.io API

This repository includes:

- `Dockerfile` for the API plus `ytsearch` CLI binary.
- `fly.staging.toml` configured for app `ytsearch-api-meili-staging` in org `pronsh-sandbox`.
- `scripts/fly-set-secrets.sh` to set API runtime secrets from `.env`.
- `scripts/fly-deploy.sh` to create/deploy the app with `FLYIO_TOKEN` or `FLY_ACCESS_TOKEN`.

Use a Fly org token as an environment variable. `flyctl` expects `FLY_ACCESS_TOKEN`, but the scripts also accept `FLYIO_TOKEN` and map it safely. The Fly scripts now read `.env` and `.env.local` automatically.

```bash
export FLYIO_TOKEN='...'
export MEILI_HTTP_ADDR='...'
export MEILI_SEARCH_API_KEY='...'

scripts/fly-set-secrets.sh
scripts/fly-deploy.sh
```

Equivalent manual commands:

```bash
export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"
flyctl apps create ytsearch-api-meili-staging --org pronsh-sandbox
flyctl secrets set \
  MEILI_HTTP_ADDR='...' \
  MEILI_SEARCH_API_KEY='...' \
  --app ytsearch-api-meili-staging
flyctl deploy --config fly.staging.toml --app ytsearch-api-meili-staging --primary-region sjc
```

Operational checks:

```bash
flyctl status --app ytsearch-api-meili-staging
flyctl logs --app ytsearch-api-meili-staging
curl 'https://ytsearch-api-meili-staging.fly.dev/api/health'
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

Website indexing from the frontend is unavailable. Type `@handle` into the main search field and submit to show the request message. Queue creators locally or from a private operator machine.

Manual update indexing locally:

```bash
cargo run --release -p youtube_cli -- index-youtuber @t3dotgg --backend meilisearch --mode update --jobs 8
```

Manual rewipe indexing locally:

```bash
ALLOW_DESTRUCTIVE_REWIPE=1 cargo run --release -p youtube_cli -- \
  index-youtuber @t3dotgg --backend meilisearch --mode rewipe \
  --allow-destructive-rewipe --jobs 8
```

Thin wrappers are split by backend and mode:

```bash
scripts/index-youtuber-meili-update.sh @t3dotgg --jobs 8
ALLOW_DESTRUCTIVE_REWIPE=1 scripts/index-youtuber-meili-rewipe.sh @t3dotgg --jobs 8
scripts/index-youtuber-typesense-update.sh @t3dotgg --jobs 8
ALLOW_DESTRUCTIVE_REWIPE=1 scripts/index-youtuber-typesense-rewipe.sh @t3dotgg --jobs 8
```

`update` keeps existing chunks and only adds new videos.
`rewipe` deletes the creator transcript chunks before replacement data is confirmed. Prefer `update` or `copy-youtuber-chunks`; rewipe requires explicit CLI and wrapper acknowledgements.

Index multiple creators from a plain text file, one handle per line:

```bash
cargo run --release -p youtube_cli -- index-youtubers scripts/indexed-channels.txt --backend meilisearch --mode update --jobs 8
```

Export the current indexed creator list from the API into a plain text file:

```bash
scripts/export-indexed-channels-list.sh indexed-channels.txt
```

The exported format is intentionally simple so it can be edited by hand:

```text
@t3dotgg | Theo - t3.gg | 973 indexed
@rasmic | Ras Mic | 124 indexed
```

Manual indexing on Fly with the same deployed image:

```bash
export FLYIO_TOKEN='...'
flyctl secrets set \
  MEILI_MASTER_KEY='...' \
  MEILI_HTTP_ADDR='...' \
  --app ytsearch-api-meili-staging
scripts/fly-run-indexer.sh -- index-youtuber @t3dotgg --backend meilisearch --mode update --jobs 4
```

The Fly image includes `yt-dlp` and `ffmpeg`. `voxtype` is not installed in the image, so `--transcribe-missing` still needs a custom image layer if you want local transcription fallback on Fly.

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
| Cloudflare Pages | Cloudflare Workers | Not recommended for current Rust API | Current Axum API is not Worker-native. Use a Worker only as a reverse proxy. Meilisearch itself should run on Fly or another container host unless Cloudflare Containers is available for the account. |

## Benchmark search latency

Use the included curl benchmark against local, staging, and production APIs:

```bash
API_BASE_URL=http://127.0.0.1:8787 QUERY=react RUNS=20 scripts/benchmark-search.sh
API_BASE_URL=https://ytsearch-api-meili-staging.fly.dev QUERY=react RUNS=20 scripts/benchmark-search.sh
API_BASE_URL=https://ytsearch-api-typesense-fly-staging.fly.dev QUERY=react RUNS=20 scripts/benchmark-search.sh
```

Compare `time_starttransfer` for perceived responsiveness and `time_total` for full response completion. Keep the API and Meilisearch in the same Fly region, leave one machine running, and use public cache headers already emitted by `/api/search` for repeat-query UX.

## Typesense rollback

Typesense remains available through the deprecated `youtube_typesense` crate. To roll back without changing API/frontend code, set:

```env
YTSEARCH_SEARCH_BACKEND=typesense
TYPESENSE_API_URL=...
TYPESENSE_SEARCH_API_KEY=...
TYPESENSE_COLLECTION=youtube_transcript_chunks
```

CLI indexing/search commands use the same `YTSEARCH_SEARCH_BACKEND` selector. Prefer `meilisearch` for new staging and production deployments.

## Observability

### Logs to Axiom

Rust services emit normal pretty logs by default and structured JSON logs when `LOG_FORMAT=json`. The Fly app sets `LOG_FORMAT=json`, which makes stdout suitable for Axiom ingestion. API responses also set cache headers: search/channel list responses are briefly public-cacheable, while health is `no-store`.

Fly sends app stdout/stderr to Fly logs. CLI indexing logs should be captured from the local/private operator environment that runs them. Search query events are sent directly to Axiom when `AXIOM_DATASET` and either `AXIOM_API_KEY` or `AXIOM_TOKEN` are set on the API app.

To export Fly platform logs to Axiom too, run Fly Log Shipper in the same org. The log shipper supports Axiom through the `AXIOM_TOKEN` and `AXIOM_DATASET` secrets.

```bash
export FLYIO_TOKEN='...'
export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"

mkdir -p /tmp/ytsearch-log-shipper
cd /tmp/ytsearch-log-shipper
flyctl launch \
  --image flyio/log-shipper:latest \
  --name ytsearch-axiom-logs-pronsh-sandbox \
  --org pronsh-sandbox \
  --no-public-ips \
  --no-deploy
flyctl secrets set \
  ORG=pronsh-sandbox \
  ACCESS_TOKEN="$FLYIO_TOKEN" \
  SUBJECT='logs.ytsearch-api-meili-staging.>' \
  AXIOM_TOKEN='...' \
  AXIOM_DATASET='ytsearch' \
  --app ytsearch-axiom-logs-pronsh-sandbox \
  --org pronsh-sandbox
flyctl deploy --app ytsearch-axiom-logs-pronsh-sandbox
```

For YouTube bot-check failures from `yt-dlp`, deploy with a current `yt-dlp` build and configure authorized extraction options as needed:

```bash
flyctl secrets set \
  YT_DLP_EXTRACTOR_ARGS='youtube:player_client=default,mweb' \
  --app ytsearch-api-meili-staging
```

If you use a cookies file, mount/provision it securely and set `YT_DLP_COOKIES_FILE`; do not commit cookies or tokens.

Useful log filter:

```env
RUST_LOG=youtube_api=info,tower_http=info,youtube_cli=info,youtube_ytdlp=info,youtube_transcribe=info
LOG_FORMAT=json
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
