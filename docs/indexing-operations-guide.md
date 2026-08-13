# Indexing operations

Raw is the only supported indexing target. All commands use the shared
`SearchIndexAdmin` contract and fail closed when the Raw URL/admin key is
missing.

## Required environment

```env
RAW_SEARCH_API_URL=https://ytsearch-raw-production.fly.dev
RAW_SEARCH_ADMIN_API_KEY=replace-with-production-secret
RAW_SEARCH_COLLECTION=youtube_transcript_chunks
INDEX_QUEUE_API_BASE_URL=https://ytsearch-api-raw-production.fly.dev
YTSEARCH_INDEX_QUEUE_TOKEN=replace-with-production-secret
```

Never send the Raw admin key to the frontend, public API response, or logs.

## Write and publication model

- One video replacement contains every chunk for that video and is atomic.
- Stable content hashes are idempotency keys, so retrying the same write is safe.
- A creator command publishes once after its video and channel mutations.
- A multi-creator command coalesces publication until the full command finishes.
- Control-plane counts and indexed-video queries include durable unpublished
  mutations; public search changes only after verified generation activation.
- Publication keeps the previous verified reader live and retains one rollback
  generation.

Do not run multiple publishers against one Raw volume.

## Direct indexing

Validate before a large run:

```bash
cargo run --release -p youtube_cli -- validate @handle
```

Index or repair one creator:

```bash
cargo run --release -p youtube_cli -- \
  index-youtuber @handle --backend raw --mode update --jobs 1
```

`update` scans newest-first and preserves correct existing videos. Use
`--max-videos`, `--max-new-videos`, or `--stop-after-existing` for a bounded
experiment. `--no-transcript-cache` forces caption refetching.

Rewipe is destructive and requires two acknowledgements:

```bash
ALLOW_DESTRUCTIVE_REWIPE=1 cargo run --release -p youtube_cli -- \
  index-youtuber @handle --backend raw --mode rewipe \
  --allow-destructive-rewipe --jobs 1
```

Never rewipe while the queue worker is indexing the same creator.

## Full-list migration/update

Run the versioned creator list in a persistent shell/background process:

```bash
JOBS=1 CHANNEL_JOBS=1 nohup \
  scripts/index-all-raw.sh scripts/indexed-channels.txt \
  > /tmp/ytsearch-index-all.log 2>&1 &
tail -f /tmp/ytsearch-index-all.log
```

Quiet failure-only output:

```bash
BACKENDS=raw JOBS=1 CHANNEL_JOBS=1 nohup \
  scripts/run-index-all-quiet.sh scripts/indexed-channels.txt \
  > /tmp/ytsearch-index-all.quiet.out 2>&1 &
```

Start with `JOBS=1`. Increase fetch concurrency only after measuring YouTube
error rate, publisher peak RSS, publication latency, and public search latency.
`CHANNEL_JOBS` controls creator concurrency independently. The bounded
full-corpus publisher peaked at 447.2 MiB standalone and 736 MiB while the old
reader remained live.

## Conference playlists

The checked-in manifest scopes official event playlists. The importer is
resumable and records completed video IDs under `data/conference-index-state`:

```bash
BACKEND=raw scripts/index-conference-playlists.sh
```

Failures are recorded and do not stop later videos. Change the manifest
deliberately; do not replace an event playlist with a broad channel feed.

## Cloud indexer

Stage the Raw admin key plus optional YouTube/cookie inputs:

```bash
scripts/fly-set-indexer-secrets.sh
```

`yt-dlp` is primary. `YOUTUBE_API_KEY` is an optional discovery/metadata
fallback and cannot retrieve captions:

```bash
YOUTUBE_API_FALLBACK=1 scripts/fly-set-indexer-secrets.sh
YOUTUBE_API_FALLBACK=0 scripts/fly-set-indexer-secrets.sh
```

For an authorized cookie export, keep the file outside the repository and let
the secret script mount it read-only inside the indexer:

```bash
export YT_DLP_COOKIES_FILE=/absolute/path/to/youtube-cookies.txt
scripts/fly-set-indexer-secrets.sh
```

Start a one-off full update:

```bash
BACKENDS=raw JOBS=1 \
  scripts/fly-run-cloud-indexer.sh scripts/indexed-channels.txt
```

Monitor with:

```bash
flyctl machine list --app ytsearch-indexer-raw-production
flyctl logs --app ytsearch-indexer-raw-production
```

The transcript cache persists at `/data/transcript-cache`. Copy permitted cache
data with `scripts/fly-upload-transcript-cache.sh` and
`scripts/fly-download-transcript-cache.sh`.

## Approval queue

Frontend requests post to the same production origin and remain `pending` until
reviewed. The API stores authoritative JSONL on its 1 GB volume; Cloudflare
Queue messages only wake the indexer.

```bash
scripts/review-index-requests.sh
scripts/review-index-requests.sh list
scripts/review-index-requests.sh approve REQUEST_ID
scripts/review-index-requests.sh reject REQUEST_ID
```

Approval changes a row to `queued` and emits a wake. Rejection retains a
terminal audit row. Requests deduplicate by handle and target backend.

Create or resume the one Raw queue Machine:

```bash
BACKENDS=raw QUEUE_TARGET_BACKEND=raw JOBS=1 LIMIT=5 \
  scripts/fly-run-queue-indexer.sh
scripts/fly-run-queue-indexers.sh
scripts/check-queue-indexers.sh
```

The worker checks approved rows before daily/gap repair, exits after an idle
window, and is restarted by another durable wake. A failed row remains visible
and does not block later work.

## Fetch reliability

Caption requests are process-wide paced. Timeouts, connection failures, 429,
and 5xx responses use bounded exponential retry. Exhausted video IDs create
small, versioned JSON records under `/data/transcript-cache/.failures`. Writes
are file-synced and atomically renamed so a Machine stop cannot leave a partial
record. Old text markers remain readable and are upgraded when that video fails
again.

The ledger distinguishes bot challenges, throttling, future premieres,
videos that YouTube is still processing, captionless/private/deleted videos,
timeouts, metadata failures, chunking, and Raw imports. Premiere and processing
responses share the retryable `scheduled` category. Retryable categories use
bounded exponential backoff with stable per-video jitter; deterministic
terminal failures stay deferred until an operator clears the record or
indexing capabilities change. The configured default base remains six hours
(`TRANSCRIPT_FAILURE_RETRY_SECONDS=21600`), while Raw import failures retry
sooner.

YouTube can return a usable caption track in video metadata while rejecting the
track's short-lived direct URL. After bounded direct-fetch retries, the indexer
now asks `yt-dlp` to download English subtitles or automatic captions for that
same video. The fallback reuses the normal parser, cache, timeout, pacing,
failure classifier, and batch circuit breaker; it is not a second unbounded
fetch path. A successful fallback is cached exactly like a direct response.
When both paths report no English track, the video is recorded as
`no_captions` instead of repeatedly blocking completion.

The production canaries were `mWPWTy7quYU` from John Hammond and
`itSJ66QSzyA` from Asianometry. Both failed through the direct track URL and
were recovered through the bounded `yt-dlp` fallback. The completed
Asianometry pass classified all 697 public videos: 686 searchable and 11 with
no English captions.

A bot challenge or session-level rate limit is a batch circuit breaker. The
CLI aborts outstanding video and creator tasks, retains every already durable
idempotent mutation, and does not publish the incomplete generation. Keep a
one-off Fly repair Machine's restart policy at `no`; restart it only after the
provider cooldown. The same complete-list command then skips durable work and
resumes the remaining gaps.

Useful controls:

- `YT_DLP_CAPTION_FETCH_ATTEMPTS`;
- `YT_DLP_CAPTION_MIN_INTERVAL_MS`;
- `YT_DLP_RATE_LIMIT_RETRY_BASE_SECONDS`;
- `VIDEO_TIMEOUT_SECONDS` (queue default: 600);
- `IDLE_GAP_FILL_MAX_NEW_VIDEOS` (default: 10).

The end-of-run summary reports both newly observed and deferred failure
categories. Unavailable newest videos do not starve older repairable gaps.

## Removal

Cancel/ack any active queue row and stop the single writer before deletion:

```bash
scripts/index-queue-ack.sh REQUEST_ID
cargo run --release -p youtube_cli -- remove-youtuber @handle \
  --backend raw --allow-destructive-remove
```

Deletion removes the profile and all video chunks through durable Raw mutations,
then publishes. The acknowledgement flag is mandatory.

## Verification

```bash
scripts/check-indexed-channel-status.sh @handle
scripts/check-frontend-backends.sh
scripts/check-raw-search-engine.sh
RUNS=10 scripts/benchmark-search-cases.sh
```

For each full migration, record:

- discovered, attempted, indexed, skipped, retryable, and terminal video counts;
- durable and published Raw sequences;
- document/video/channel totals before and after;
- publication duration, peak RSS, disk high-water mark, and search error count;
- browser result totals and first-result ordering for the fixed relevance matrix.

The production deployment/cutover order is in [Deployment](deploy.md); recovery
and rollback are in [Disaster recovery](disaster-recovery.md).
