# Indexing Operations Guide

## Frontend URLs

The active Pages deployment points search at the Typesense API and posts
`@handle` index requests to the central Fly queue.

| Frontend | Search API | Queue API |
| --- | --- | --- |
| https://ytsearch-755.pages.dev | `ytsearch-api-typesense-fly-staging` | `ytsearch-api-typesense-fly-staging` |

The Pages production deployment is the active Typesense frontend.

## One-Off Full Update

Run this in a terminal or background job. It updates the active Typesense backend, then refreshes profile counts. Keep a single writer while staging uses one Typesense node:

```bash
JOBS=1 nohup scripts/index-all-backends-update.sh scripts/indexed-channels.txt \
  > /tmp/ytsearch-index-all.log 2>&1 &
```

Quiet mode, which only prints on failure:

```bash
JOBS=1 nohup scripts/run-index-all-quiet.sh scripts/indexed-channels.txt \
  > /tmp/ytsearch-index-all.quiet.out 2>&1 &
```

Tail it:

```bash
tail -f /tmp/ytsearch-index-all.log
```

Raise indexing concurrency only after adding an ingest replica or proving the
engine has write headroom under production searches. The current single-node
Typesense engine became unavailable when three concurrent writers accumulated.

## Scoped Conference Playlists

`scripts/conference-playlists.tsv` contains official, event-only playlists for
NVIDIA GTC 2026, Google I/O 2026 All Sessions, and Apple WWDC26 Sessions. Run
the resumable single-writer importer with:

```bash
BACKEND=typesense scripts/index-conference-playlists.sh
```

Completed video IDs are recorded under `data/conference-index-state`, and
captions use the normal transcript cache. Failures are logged and do not stop
later videos. Update the manifest deliberately for a new conference year; do
not replace these scopes with broad channel handles.

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

The shared `TranscriptChunkSource` and sink ports remain available for a future
backend migration. Repair Typesense from the transcript cache or a permitted
YouTube re-fetch.

Start a conservative cloud update against the active Typesense backend:

```bash
JOBS=1 \
scripts/fly-run-cloud-indexer.sh scripts/indexed-channels.txt
```

For an overnight full rewipe, keep `JOBS=1` and stop the persistent worker
first. Rewipes are destructive and should not compete with public search:

```bash
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
  --backend typesense --mode update --jobs 1 --transcribe-missing
scripts/fly-upload-transcript-cache.sh data/transcript-cache
```

After that upload, cloud indexing can reuse the cached caption/transcript JSON for those videos without running Voxtype.

The shared runner is `scripts/index-all-backends.sh`. The update and rewipe wrappers set `MODE` for you:

```bash
scripts/index-all-backends-update.sh scripts/indexed-channels.txt
ALLOW_DESTRUCTIVE_REWIPE=1 scripts/index-all-backends-rewipe.sh scripts/indexed-channels.txt
```

`index-all-backends-update.sh` scans the full channel by default so older holes are eventually repaired. Use `EXISTING_TAIL=50` or `STOP_AFTER_EXISTING=50` only when you intentionally want a faster newest-first check, and `MAX_VIDEOS=10` to cap a test run.

The active wrapper targets `typesense-fly`. Keep the explicit backend when
running it from an operator shell:

```bash
BACKENDS="typesense-fly" scripts/index-all-backends-update.sh scripts/indexed-channels.txt
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

The active list includes Apple Developer (WWDC), DEF CON Conference, Google for Developers (Google I/O), NeetCode, Pawel Code Stuff, and Ryan Carniato. Apple Developer and Google for Developers are temporarily broad-channel sources; playlist-scoped ingestion is required before claiming conference-only coverage. Ryan's historical streams are not guaranteed by the `/videos` channel feed and need the same scoped source support.

The default update script runs:

1. Self-hosted Typesense-on-Fly update.
2. Count refresh after the update.

## Queue Workflow

When someone enters `@somehandle` in the frontend, the app posts:

```http
POST https://ytsearch-api-typesense-fly-staging.fly.dev/api/index-requests
```

The queue is a simple JSONL file on a Fly volume at `/data/index-requests.jsonl`.
Each frontend request creates one `typesense-fly` row with `pending` status and
is deduplicated by handle plus target backend. Pending requests are inert: the
indexer only reads `queued` and `running` rows.

Review pending requests interactively:

```bash
scripts/review-index-requests.sh
```

The CLI shows each handle, request ID, backend, request time, and note. Choose
`a` to approve, `r` to reject, `s` to leave the row pending, or `q` to stop.
Approving changes the row to `queued` and sends the Cloudflare wake message that
starts the on-demand indexer. Rejecting retains a terminal `rejected` row for
the frontend and operator audit without starting a Machine.

Non-interactive forms are available for automation and direct review:

```bash
scripts/review-index-requests.sh list
scripts/review-index-requests.sh approve REQUEST_ID
scripts/review-index-requests.sh reject REQUEST_ID
```

The review CLI defaults to `https://search.pronsh.dev` so approval passes
through the Worker wake producer. Override `INDEX_REVIEW_API_BASE_URL` only with
another deployment that implements the same approval-and-wake route.

The API machine owns this JSONL volume. Multiple queue consumers would need
shared locking or a real queue/DB; otherwise two machines can race on the same
request log. If this needs horizontal scale, move queue storage to Cloudflare
Queues plus D1/R2, Fly Postgres, or another single-writer service first.

Pull all retained request rows locally, including pending and terminal rows:

```bash
scripts/index-queue-pull.sh
```

Drain queued requests on a one-off Fly indexer Machine. Scope the worker to one target backend so it only acks that backend's queue rows:

```bash
scripts/fly-set-indexer-secrets.sh
BACKENDS="typesense-fly" QUEUE_TARGET_BACKEND=typesense-fly JOBS=1 LIMIT=5 scripts/fly-run-queue-indexer.sh
flyctl logs --app ytsearch-indexer-staging
```

Keep exactly one configured on-demand Machine. The launcher is idempotent: it
starts the existing stopped worker and creates one only when none exists:

```bash
scripts/check-queue-indexers.sh
JOBS=1 QUEUE_POLL_INTERVAL_SECONDS=30 scripts/fly-run-queue-indexers.sh
scripts/check-queue-indexers.sh
```

After the request queue empties, the worker exits after five idle minutes. The
Cloudflare Queue consumer restarts it only after an operator approves another
request. This keeps the transcript cache volume persistent without paying for
an idle VM.

`fly-run-queue-indexer.sh` refuses a missing target or a multi-backend scope. Do not run two daemons with the same `QUEUE_TARGET_BACKEND`. If channel validation or indexing fails, the row is retained with `failed` status and no longer blocks newer work. Re-submit the handle after correcting the spelling, cookies, or upstream issue to create a fresh retry row.

### Typesense publication-sort migration

Typesense uses the numeric `published_at_timestamp` field for newest and oldest
results. Existing chunks acquire it through non-destructive update ingestion.
A destructive source re-extraction remains available only with both
`--allow-destructive-rewipe` and `ALLOW_DESTRUCTIVE_REWIPE=1` in the wrappers.

For a deliberate bounded repair window, temporarily enable the update pass.
This is not the staging default and keeps the Machine running until stopped:

```bash
DAILY_UPDATE_ENABLED=1 \
DAILY_UPDATE_INITIAL_DELAY_SECONDS=0 \
DAILY_UPDATE_INTERVAL_SECONDS=300 \
DAILY_UPDATE_LIST_SOURCE=scripts/indexed-channels.txt \
JOBS=1 \
scripts/fly-run-queue-indexers.sh
```

Each repair pass runs `index-youtubers --mode update` and then
`refresh-youtuber-counts-list`. Count reconciliation reads only backend chunks
and does not contact YouTube. Run `refresh-youtubers` separately when profile
metadata, public counts, or avatars need enrichment. `EXISTING_TAIL=0` scans the
full channel list so older missing captions can eventually be repaired; use a
positive value only for a faster newest-first pass. The interval is a cooldown
after a complete pass, not a timer that interrupts active indexing.

`scripts/fly-set-indexer-secrets.sh` uses Fly's staged-secret path because the indexers are raw Machines rather than a Fly Launch process group. Cookie and optional YouTube API values are stored as base64 file secrets. Recreate each worker deliberately after staging changes; launchers mount the files and entrypoints load the API key only into the child process environment. The cloud image includes Deno and `yt-dlp-ejs` for current YouTube JavaScript challenges.

Index one request at your leisure:

```bash
cargo run --release -p youtube_cli -- index-youtuber @somehandle --backend typesense --mode update --jobs 1
TYPESENSE_API_URL=https://ytsearch-typesense-staging.fly.dev \
TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
cargo run --release -p youtube_cli -- index-youtuber @somehandle --backend typesense --mode update --jobs 8
```

Cancel the queue row first if the channel is currently running, then explicitly
remove its profile and transcript chunks:

```bash
scripts/index-queue-ack.sh REQUEST_ID
TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
cargo run --release -p youtube_cli -- remove-youtuber @somehandle \
  --backend typesense --allow-destructive-remove
```

The acknowledgement flag is mandatory. Stop or restart the single worker before
removing a currently active channel so it cannot continue writing after the
deletion.

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
API_BASE_URL="$TYPESENSE_FLY_API_BASE_URL" scripts/export-indexed-channels-list.sh /tmp/typesense-fly-channels.txt
cat /tmp/*-channels.txt
```

Smoke-check that active frontend variants and their backing search APIs are reachable:

```bash
scripts/check-frontend-backends.sh
```

## Deploys

Self-hosted Typesense engine and API:

```bash
scripts/fly-deploy-typesense.sh
scripts/fly-set-typesense-selfhost-api-secrets.sh
scripts/fly-deploy-typesense-api.sh
```

Canonical frontend Worker:

```bash
scripts/cloudflare-worker-deploy.sh
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
