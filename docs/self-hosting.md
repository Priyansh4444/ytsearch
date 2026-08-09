# Self-hosting

ytsearch runs one mutable Raw engine, one Rust API, one queue worker, and the
Solid frontend. Docker Compose is the supported local topology.

## Setup

Install Rust stable, Bun 1.3.14, Docker Compose, OpenSSL, and `yt-dlp`.
`ffmpeg` is required only for permitted local transcription.

```bash
bun run setup
docker compose up -d --build
docker compose ps
```

`bun run setup` creates `.env` only when it does not exist and generates
separate Raw and queue credentials. To configure by hand:

```env
RAW_SEARCH_API_URL=http://127.0.0.1:8108
RAW_SEARCH_API_KEY=replace-with-a-long-random-value
RAW_SEARCH_ADMIN_API_KEY=replace-with-another-long-random-value
RAW_SEARCH_COLLECTION=youtube_transcript_chunks
INDEX_QUEUE_API_BASE_URL=http://127.0.0.1:8787
YTSEARCH_INDEX_QUEUE_TOKEN=replace-with-a-long-random-value
```

Check the stack:

```bash
curl -fsS http://localhost:8108/health
curl -fsS http://localhost:8787/api/health
curl -fsS http://localhost:8787/api/youtubers
```

Open `http://localhost:3000`.

## Persistent state

Compose creates three named volumes:

- `raw-mutable-data`: SQLite WAL, published generations, and rollback state;
- `queue-data`: authoritative JSONL approval queue;
- `transcript-cache`: fetched captions, failure cooldowns, and repair cursor.

Back up all three. The immutable base corpus files used by
`Dockerfile.raw-search` must also be available to rebuild the image. Production
data under `.search-experiment/` is intentionally gitignored and is not part of
a clean source checkout; self-hosters must build their own base generation and
source archives before building that image.

## Index a creator

```bash
docker compose exec worker ytsearch \
  index-youtuber @handle --backend raw --mode update --jobs 1
```

The command writes complete-video replacements with stable idempotency keys and
publishes once after the channel batch. `update` is the normal mode. Rewipe is
destructive and requires both the CLI flag and environment acknowledgement.

## Queue operations

Frontend submissions remain pending until an operator reviews them:

```bash
scripts/review-index-requests.sh
```

Keep exactly one queue worker for a JSONL queue file. Public submissions default
to six requests per client per minute and 10,000 retained rows; tune
`YTSEARCH_INDEX_REQUEST_RATE_LIMIT_PER_MINUTE` and
`YTSEARCH_INDEX_QUEUE_MAX_ITEMS` deliberately.

Periodic full-list repair is opt-in:

```bash
DAILY_UPDATE_ENABLED=1 DAILY_UPDATE_INITIAL_DELAY_SECONDS=0 \
  docker compose up -d worker
```

## Bare-process development

Start Raw through Compose, then run API and frontend separately:

```bash
docker compose up -d raw-search init-backend
bun run api
bun run dev
```

Raw administration requires `RAW_SEARCH_ADMIN_API_KEY`. Public API processes
should receive only `RAW_SEARCH_API_KEY` unless they explicitly host a trusted
administration route.

## Cookies and transcription

Use only cookies from an authorized session and only for content you may
process. Keep exports outside the repository and mount them read-only:

```yaml
services:
  worker:
    environment:
      YT_DLP_COOKIES_FILE: /run/secrets/yt-dlp-cookies.txt
    volumes:
      - /absolute/path/to/cookies.txt:/run/secrets/yt-dlp-cookies.txt:ro
```

The YouTube Data API is a discovery/metadata fallback; it does not provide
captions. Do not use transcription or cookies to bypass access controls.

## Recovery

Restore the base corpus, Raw mutable volume, queue volume, and transcript cache
into an isolated stack first. Require generation integrity, exact queue IDs,
known searches, one publish/restart mutation, and a clean browser check before
routing traffic. See [Disaster recovery](disaster-recovery.md).
