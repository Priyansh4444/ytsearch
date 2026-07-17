# Self-Hosting

ytsearch runs with one self-hosted Typesense node, the Rust API, a queue worker,
and the Solid frontend. The only provider-specific pieces are a reachable
Typesense URL and a process supervisor for the API and worker.

## Minimal configuration

Copy `.env.example` to `.env` and set:

```env
TYPESENSE_SELFHOST_API_URL=http://127.0.0.1:8108
TYPESENSE_SELFHOST_API_KEY=replace-me
TYPESENSE_COLLECTION=youtube_transcript_chunks
INDEX_QUEUE_API_BASE_URL=http://127.0.0.1:8787
YTSEARCH_INDEX_QUEUE_TOKEN=replace-me
VITE_API_BASE_URL=http://127.0.0.1:8787
VITE_INDEX_QUEUE_API_BASE_URL=http://127.0.0.1:8787
```

`yt-dlp` is the primary metadata and caption source. Set `YOUTUBE_API_KEY`
only when YouTube Data API fallback is desired. Cookies can be supplied with
`YT_DLP_COOKIES_FILE` when YouTube requires verification.

## Run locally

### Docker Compose

The smallest complete deployment uses the checked-in Compose file. Set two
private values and start the stack. An idempotent one-shot service creates the
empty Typesense collections on first boot. The worker polls approved queue
requests but does not run bulk channel updates unless explicitly enabled:

```bash
export TYPESENSE_SELFHOST_API_KEY="$(openssl rand -hex 24)"
export YTSEARCH_INDEX_QUEUE_TOKEN="$(openssl rand -hex 24)"
export PUBLIC_API_BASE_URL=http://localhost:8787
docker compose up -d --build
docker compose ps
curl -fsS http://localhost:8108/health
curl -fsS http://localhost:8787/api/health
curl -fsS http://localhost:8787/api/youtubers
```

To enable the periodic bulk update list, opt in when starting Compose:

```bash
DAILY_UPDATE_ENABLED=1 DAILY_UPDATE_INITIAL_DELAY_SECONDS=0 docker compose up -d worker
```

Open `http://localhost:3000`. Persistent named volumes retain Typesense data,
the JSONL queue, and transcript cache. Only the frontend and API ports need to
be public; keep Typesense private outside local development.

Set `PUBLIC_API_BASE_URL` to the browser-reachable HTTPS API origin before
building on a remote host. Optional `YOUTUBE_API_KEY`, `YT_DLP_COOKIES_FILE`,
`VITE_POSTHOG_KEY`, and `VITE_POSTHOG_HOST` values pass through without being
required.

### Bare processes

Start Typesense, then run the API and frontend in separate terminals:

```bash
bun install
bun run api
bun run dev
```

Run one persistent queue worker with `QUEUE_DAEMON=1`, `QUEUE_LOOP=1`, and
`DAILY_UPDATE_ENABLED=1`. Keep exactly one worker attached to a queue file.
For a first update, set `DAILY_UPDATE_INITIAL_DELAY_SECONDS=0`; subsequent
updates use `DAILY_UPDATE_INTERVAL_SECONDS` (normally 86400).

Bootstrap a creator directly with:

```bash
cargo run -p youtube_cli -- index-youtuber @handle --backend typesense --mode update
```

For a bare-process installation, initialize empty collections once before
starting the API:

```bash
cargo run -p youtube_cli -- init-backend --backend typesense
```

Queue administration routes require `YTSEARCH_INDEX_QUEUE_TOKEN`.
