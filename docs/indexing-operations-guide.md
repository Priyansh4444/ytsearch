# Indexing Operations Guide

## Frontend URLs

Each preview points search at a different backend API and posts `@handle` index requests to the central Fly queue.

| Frontend | Search API | Queue API |
| --- | --- | --- |
| https://meili.ytsearch-755.pages.dev | `ytsearch-api-meili-staging` | `ytsearch-index-queue-staging` |
| https://typesense.ytsearch-755.pages.dev | `ytsearch-api-typesense-fly-staging` | `ytsearch-index-queue-staging` |

`meili` is the preferred current staging frontend.

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

## Cloud Indexing

Long indexing runs should use a detached Fly Machine instead of SSHing into an API VM or depending on a laptop shell. The Docker image includes the `ytsearch` CLI, `yt-dlp`, and `ffmpeg`, so cloud indexing can fetch public captions with `yt-dlp`. It does not install `voxtype`; missing-caption videos are skipped unless you build a custom image with Voxtype or pre-seed the transcript cache from a local run.

Set the dedicated indexer secrets from local env values:

```bash
scripts/fly-set-indexer-secrets.sh
```

`YOUTUBE_API_KEY` is optional for the indexer path now. The queue daemon and bulk indexers use `yt-dlp` first for creator discovery, metadata, and captions, then fall back to the YouTube Data API only when a key is present and `yt-dlp` cannot resolve a handle or video cleanly.

Keep the fallback opt-in and explicit when setting indexer secrets:

```bash
# Retain yt-dlp as primary and make the Data API available only as fallback.
YOUTUBE_API_FALLBACK=1 scripts/fly-set-indexer-secrets.sh

# Remove a stale/exhausted fallback key from the indexer app.
YOUTUBE_API_FALLBACK=0 scripts/fly-set-indexer-secrets.sh
```

If `yt-dlp` needs browser cookies, export a Netscape-format cookies file path before setting secrets. The script base64-encodes that file into a Fly secret and the launcher mounts it at `/run/secrets/yt-dlp-cookies.txt`:

```bash
export YT_DLP_COOKIES_FILE="$HOME/Downloads/youtube-cookies.txt"
scripts/fly-set-indexer-secrets.sh
```

You can also set `YT_DLP_EXTRACTOR_ARGS` or `YT_DLP_USER_AGENT` locally before running the same secret script. These are consumed by `youtube_ytdlp` when it shells out to `yt-dlp`.

Do not attempt to generate or fabricate a YouTube login cookie in cloud infrastructure. A valid cookie represents an authenticated browser session and is subject to YouTube's controls. The supported recovery paths are: use public captions without cookies, mount an existing authorized-session export through the secret script, or seed `/data/transcript-cache` from a permitted local run. The YouTube Data API fallback supplies discovery and metadata only; it does not provide transcript access.

When one search backend already has canonical transcript chunks, copy them directly instead of downloading captions again:

```bash
# Idempotent upsert; safe to repeat while repairing a partial target.
scripts/copy-meili-to-typesense.sh @t3dotgg

# Deliberately clear this creator from Typesense before copying.
REPLACE=1 scripts/copy-meili-to-typesense.sh @t3dotgg
```

The CLI command behind this wrapper is `ytsearch copy-youtuber-chunks`. It reads paginated canonical `TranscriptChunk` values through the `TranscriptChunkSource` port and writes through the existing backend sink. It also normalizes legacy creator IDs and refreshes target indexed video/chunk counts without contacting YouTube. Meilisearch and Typesense both implement the same export port, so either can repair or migrate the other without changing the copy workflow.

Start a conservative cloud update across the active backends:

```bash
JOBS=1 \
scripts/fly-run-cloud-indexer.sh scripts/indexed-channels.txt
```

For an overnight full rewipe, start one isolated Machine per backend so Meilisearch and Typesense rebuild in parallel. Keep `JOBS=1` per machine unless engine metrics demonstrate more write headroom:

```bash
ALLOW_DESTRUCTIVE_REWIPE=1 ONE_OFF_NO_VOLUME=1 BACKENDS=meilisearch MODE=rewipe JOBS=1 MEILI_JOBS=1 \
  INDEXER_MACHINE_NAME=ytsearch-rewipe-meili-$(date +%Y%m%d) \
  scripts/fly-run-cloud-indexer.sh scripts/indexed-channels.txt

ALLOW_DESTRUCTIVE_REWIPE=1 ONE_OFF_NO_VOLUME=1 BACKENDS=typesense-fly MODE=rewipe JOBS=1 TYPESENSE_JOBS=1 \
  INDEXER_MACHINE_NAME=ytsearch-rewipe-typesense-$(date +%Y%m%d) \
  scripts/fly-run-cloud-indexer.sh scripts/indexed-channels.txt
```

Both one-off workers use 2 shared CPUs, 4 GB RAM, and 1 GB swap by default. They are removed when complete. Do not run a backend's rewipe concurrently with a non-idle queue worker for that backend.

The launcher defaults to a temporary 2 shared CPU / 4 GB Machine, a 10 GB rootfs overlay, and a 10 GB transcript-cache volume. The Machine is created with `--rm`, so it is removed when the job exits; the transcript-cache volume remains for future runs. Override with `INDEXER_MEMORY_MB`, `INDEXER_CPUS`, `INDEXER_ROOTFS_SIZE_GB`, or `INDEXER_VOLUME_SIZE_GB` only for a specific large run.

Watch it:

```bash
flyctl machine list --app ytsearch-indexer-staging
flyctl logs --app ytsearch-indexer-staging
```

The indexer app uses a Fly volume mounted at `/data`; the caption cache lives at `/data/transcript-cache`. To seed the cloud cache from a local caption run:

```bash
scripts/fly-upload-transcript-cache.sh data/transcript-cache
```

To copy the cloud cache back locally:

```bash
scripts/fly-download-transcript-cache.sh data/transcript-cache
```

Use local Voxtype only when you explicitly need audio transcription fallback:

```bash
cargo run --release -p youtube_cli -- index-youtuber @somehandle \
  --backend meilisearch --mode update --jobs 1 --transcribe-missing
scripts/fly-upload-transcript-cache.sh data/transcript-cache
```

After that upload, cloud indexing can reuse the cached caption/transcript JSON for those videos without running Voxtype.

The shared runner is `scripts/index-all-backends.sh`. The update and rewipe wrappers set `MODE` for you:

```bash
scripts/index-all-backends-update.sh scripts/indexed-channels.txt
ALLOW_DESTRUCTIVE_REWIPE=1 scripts/index-all-backends-rewipe.sh scripts/indexed-channels.txt
```

`index-all-backends-update.sh` scans the full channel by default so older holes are eventually repaired. Use `EXISTING_TAIL=50` or `STOP_AFTER_EXISTING=50` only when you intentionally want a faster newest-first check, and `MAX_VIDEOS=10` to cap a test run.

By default it attempts `meilisearch typesense-fly`. Scope a run with `BACKENDS`:

```bash
BACKENDS="meilisearch" scripts/index-all-backends-update.sh scripts/indexed-channels.txt
BACKENDS="meilisearch typesense-fly" scripts/index-all-backends-update.sh scripts/indexed-channels.txt
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
2. Self-hosted Typesense-on-Fly update.
3. Count refresh for active backends.

## Queue Workflow

When someone enters `@somehandle` in any frontend, the app posts:

```http
POST https://ytsearch-index-queue-staging.fly.dev/api/index-requests
```

The queue is a simple JSONL file on a Fly volume at `/data/index-requests.jsonl`. Frontend requests are expanded into one queued row per active backend (`meilisearch` and `typesense-fly`) and deduplicated by handle plus target backend.

The queue app should stay as one warm machine while it uses this JSONL volume. Multiple queue machines would need shared locking or a real queue/DB; otherwise two machines can race on the same request log. If this needs horizontal scale, move the queue storage to Cloudflare Queues plus D1/R2, Fly Postgres, or another single-writer service first.

Pull queued requests locally:

```bash
scripts/index-queue-pull.sh
```

Drain queued requests on a one-off Fly indexer Machine. Scope the worker to one target backend so it only acks that backend's queue rows:

```bash
scripts/fly-set-indexer-secrets.sh
BACKENDS="meilisearch" QUEUE_TARGET_BACKEND=meilisearch JOBS=1 LIMIT=5 scripts/fly-run-queue-indexer.sh
flyctl logs --app ytsearch-indexer-staging
```

Keep both indexes building automatically with two persistent polling Machines. The launcher is idempotent: it verifies healthy named workers, starts only missing workers, and refuses to overlap a legacy shared worker:

```bash
scripts/check-queue-indexers.sh
JOBS=1 QUEUE_POLL_INTERVAL_SECONDS=30 scripts/fly-run-queue-indexers.sh
scripts/check-queue-indexers.sh
```

`fly-run-queue-indexer.sh` refuses a missing target or a multi-backend scope. Do not run two daemons with the same `QUEUE_TARGET_BACKEND`. If a backend fails, only that backend's request remains queued and is retried on a later pass.

### Typesense publication-sort migration

Typesense now uses the numeric `published_at_timestamp` field for newest and oldest results. Adding the field to an existing collection is additive, but old chunks do not acquire a value automatically. Prefer non-destructive Meilisearch-to-Typesense chunk copies to backfill existing creators. The upserted canonical chunks include the derived timestamp without deleting the Typesense copy first:

```bash
ytsearch copy-youtuber-chunks @t3dotgg \
  --source-backend meilisearch --target-backend typesense --batch-size 5000
```

Run large copies during lower write load. A destructive source re-extraction remains available only with both `--allow-destructive-rewipe` and `ALLOW_DESTRUCTIVE_REWIPE=1` in the wrappers.

Enable a daily caption/video update pass from YouTube for the indexed-channel list on the same daemon:

```bash
DAILY_UPDATE_ENABLED=1 \
DAILY_UPDATE_INITIAL_DELAY_SECONDS=86400 \
DAILY_UPDATE_INTERVAL_SECONDS=86400 \
DAILY_UPDATE_LIST_SOURCE=scripts/indexed-channels.txt \
JOBS=1 \
scripts/fly-run-queue-indexers.sh
```

In the split daemon setup, each daily update runs `index-youtubers --mode update` and then `refresh-youtuber-counts-list` for its own backend. Count reconciliation reads only backend chunks and does not contact YouTube. Run `refresh-youtubers` separately when profile metadata, public counts, or avatars need enrichment. `DAILY_UPDATE_INITIAL_DELAY_SECONDS=86400` means the first daily update starts about one day after the daemon launches; set it to `0` only when you want an immediate catch-up run. `EXISTING_TAIL=0` scans the full channel list so older missing captions can eventually be repaired; use a positive value only for a faster newest-first pass.

Index one request at your leisure:

```bash
cargo run --release -p youtube_cli -- index-youtuber @somehandle --backend meilisearch --mode update --jobs 8
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
API_BASE_URL="$TYPESENSE_FLY_API_BASE_URL" scripts/export-indexed-channels-list.sh /tmp/typesense-fly-channels.txt
cat /tmp/*-channels.txt
```

Smoke-check that active frontend variants and their backing search APIs are reachable:

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

For machine status, indexed channel verification, queue checks, and the latest recorded staging benchmark run, see [staging-observability.md](staging-observability.md).

Local:

```bash
QUERY=fable YOUTUBER=@t3dotgg RUNS=10 scripts/benchmark-search-backends.sh
```

The local/multi-region backend benchmark prints raw curl timings and a summary table with `min_ms`, `avg_ms`, `p50_ms`, `p95_ms`, and `max_ms` per backend. Keep a raw run file with:

```bash
RESULTS_FILE=/tmp/ytsearch-benchmark.$(date +%s).log QUERY=fable YOUTUBER=@t3dotgg RUNS=20 scripts/benchmark-search-backends.sh
```

For the representative query suite, including BatteryGate, Linux exploit, stopword-heavy, and low-signal cases:

```bash
RUNS=10 scripts/benchmark-search-cases.sh
```

Global:

```bash
QUERY=fable YOUTUBER=@t3dotgg scripts/benchmark-search-globalping.sh
```
