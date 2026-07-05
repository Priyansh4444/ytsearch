# Indexing Operations Guide

## Frontend URLs

Each preview points search at a different backend API and posts `@handle` index requests to the central Fly queue.

| Frontend | Search API | Queue API |
| --- | --- | --- |
| https://frontend-meilisearch.ytsearch-755.pages.dev | `ytsearch-api-meili-staging` | `ytsearch-index-queue-staging` |
| https://frontend-typesense-cloud.ytsearch-755.pages.dev | `ytsearch-api-pronsh-sandbox` | `ytsearch-index-queue-staging` |
| https://frontend-typesense-fly.ytsearch-755.pages.dev | `ytsearch-api-typesense-fly-staging` | `ytsearch-index-queue-staging` |

## One-Off Full Update

Run this in a terminal or background job. It updates every configured backend, then refreshes profile counts:

```bash
JOBS=4 nohup scripts/index-all-backends-update.sh scripts/indexed-channels.txt \
  > /tmp/ytsearch-index-all.log 2>&1 &
```

Quiet mode, which only prints on failure:

```bash
JOBS=4 nohup scripts/run-index-all-quiet.sh scripts/indexed-channels.txt \
  > /tmp/ytsearch-index-all.quiet.out 2>&1 &
```

Tail it:

```bash
tail -f /tmp/ytsearch-index-all.log
```

For Meilisearch, prefer `JOBS=3` or `JOBS=4` on the current Fly staging VM. Higher concurrency can create large update temp files and trigger disk pressure.

The shared runner is `scripts/index-all-backends.sh`. The update and rewipe wrappers set `MODE` for you:

```bash
scripts/index-all-backends-update.sh scripts/indexed-channels.txt
scripts/index-all-backends-rewipe.sh scripts/indexed-channels.txt
```

`index-all-backends-update.sh` scans newest-first and stops after 50 consecutive already-indexed videos by default. This is sustainable for normal incremental indexing because YouTube upload playlists are newest-first. Use `EXISTING_TAIL=100` to scan deeper before stopping, `MAX_VIDEOS=10` to cap a test run, or `ALL_VIDEOS=1` to scan the full channel.

By default it attempts `meilisearch typesense-cloud typesense-fly`. Scope a run with `BACKENDS`:

```bash
BACKENDS="meilisearch typesense-cloud" scripts/index-all-backends-update.sh scripts/indexed-channels.txt
```

Run the full local validation suite before a deploy or long index run:

```bash
bun run check:all
```

If `typesense-fly` is selected but `TYPESENSE_SELFHOST_API_KEY` or `TYPESENSE_SELFHOST_API_URL` is missing, that backend is skipped instead of failing the entire all-backends update.

Update mode now does two layers of work avoidance:

1. It checks videos newest-first and stops after a configurable tail of already-indexed videos. For uncapped small batches without the tail shortcut, it can still use per-worker existing checks.
2. It caches fetched `yt-dlp` transcript segments under `data/transcript-cache`, so retrying after a backend import failure does not refetch captions.

Use `--no-transcript-cache` when you need to force a fresh caption fetch.

The default update script runs:

1. Meilisearch update.
2. Typesense Cloud update.
3. Self-hosted Typesense-on-Fly update.
4. Count refresh for all three backends.

## Queue Workflow

When someone enters `@somehandle` in any frontend, the app posts:

```http
POST https://ytsearch-index-queue-staging.fly.dev/api/index-requests
```

The queue is a simple JSONL file on a Fly volume at `/data/index-requests.jsonl`.

Pull queued requests locally:

```bash
scripts/index-queue-pull.sh
```

Index one request at your leisure:

```bash
cargo run --release -p youtube_cli -- index-youtuber @somehandle --backend meilisearch --mode update --jobs 8
cargo run --release -p youtube_cli -- index-youtuber @somehandle --backend typesense --mode update --jobs 8
TYPESENSE_API_URL=https://ytsearch-typesense-staging.fly.dev \
TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
cargo run --release -p youtube_cli -- index-youtuber @somehandle --backend typesense --mode update --jobs 8
```

Ack the request after the indexes are updated:

```bash
scripts/index-queue-ack.sh REQUEST_ID
```

## Repair Wrong Indexed Counts

The count bug was caused by writing per-run counts into the creator document instead of recounting the backend. The CLI now recounts distinct `video_id` values and chunks from the backend before upserting the YouTuber profile.

Repair all known creators without reindexing:

```bash
scripts/refresh-all-backend-counts.sh scripts/indexed-channels.txt
```

Quickly check what each frontend/API currently reports:

```bash
API_BASE_URL="$MEILI_API_BASE_URL" scripts/export-indexed-channels-list.sh /tmp/meili-channels.txt
API_BASE_URL="$TYPESENSE_CLOUD_API_BASE_URL" scripts/export-indexed-channels-list.sh /tmp/typesense-cloud-channels.txt
API_BASE_URL="$TYPESENSE_FLY_API_BASE_URL" scripts/export-indexed-channels-list.sh /tmp/typesense-fly-channels.txt
cat /tmp/*-channels.txt
```

Smoke-check that all three frontend variants and their backing search APIs are reachable:

```bash
scripts/check-frontend-backends.sh
```

## Deploys

Queue app:

```bash
scripts/fly-deploy-index-queue.sh
```

Self-hosted Typesense engine and API:

```bash
scripts/fly-deploy-typesense.sh
scripts/fly-set-typesense-selfhost-api-secrets.sh
scripts/fly-deploy-typesense-api.sh
```

All frontend variants:

```bash
scripts/cloudflare-pages-deploy-backends.sh
```

## Benchmarks

Local:

```bash
QUERY=fable YOUTUBER=@t3dotgg RUNS=10 scripts/benchmark-search-backends.sh
```

The local/multi-region backend benchmark prints raw curl timings and a summary table with `min_ms`, `avg_ms`, `p50_ms`, `p95_ms`, and `max_ms` per backend. Keep a raw run file with:

```bash
RESULTS_FILE=/tmp/ytsearch-benchmark.$(date +%s).log QUERY=fable YOUTUBER=@t3dotgg RUNS=20 scripts/benchmark-search-backends.sh
```

Global:

```bash
QUERY=fable YOUTUBER=@t3dotgg scripts/benchmark-search-globalping.sh
```
