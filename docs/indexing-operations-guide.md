# Indexing operations

Raw is the only indexing target. Every command fails closed when the Raw URL or
admin key is missing:

```env
RAW_SEARCH_API_URL=https://ytsearch-raw-production.fly.dev
RAW_SEARCH_ADMIN_API_KEY=replace-with-production-secret
RAW_SEARCH_COLLECTION=youtube_transcript_chunks
INDEX_QUEUE_API_BASE_URL=https://ytsearch-api-raw-production.fly.dev
YTSEARCH_INDEX_QUEUE_TOKEN=replace-with-production-secret
```

Never send the admin key to the frontend, an API response or logs.

## Write and publication model

- A video replacement holds all of that video's chunks and is atomic. Content
  hashes are idempotency keys, so retries are safe.
- A creator command publishes once after its mutations; a multi-creator command
  publishes once at the end.
- Counts and indexed-video queries include durable unpublished mutations; public
  search changes only when a verified generation activates.
- Publication keeps the previous reader live and one rollback generation.
  Never run two publishers against one Raw volume.

## Index one creator

```bash
cargo run --release -p youtube_cli -- validate @handle
cargo run --release -p youtube_cli -- \
  index-youtuber @handle --backend raw --mode update --jobs 1
```

`update` scans newest-first and keeps correct existing videos. Bound an
experiment with `--max-videos`, `--max-new-videos` or `--stop-after-existing`;
`--no-transcript-cache` refetches captions. Rewipe is destructive, needs both
acknowledgements, and must not run while the queue worker indexes that creator:

```bash
ALLOW_DESTRUCTIVE_REWIPE=1 cargo run --release -p youtube_cli -- \
  index-youtuber @handle --backend raw --mode rewipe \
  --allow-destructive-rewipe --jobs 1
```

## Index the full list

```bash
JOBS=1 CHANNEL_JOBS=1 nohup scripts/index-all-raw.sh scripts/indexed-channels.txt \
  > /tmp/ytsearch-index-all.log 2>&1 &
# failure-only output:
JOBS=1 CHANNEL_JOBS=1 nohup scripts/run-index-all-quiet.sh \
  scripts/indexed-channels.txt > /tmp/ytsearch-index-all.quiet.out 2>&1 &
```

`CHANNEL_JOBS` sets creator concurrency, `JOBS` fetch concurrency. Raise either
only after measuring YouTube errors, publisher RSS, publication latency and
search latency. (Schema-6 era, 2026-08-11: the publisher peaked at 447 MiB
alone and 736 MiB beside the live reader; remeasure for schema 9.)

Conference playlists come from a checked-in manifest of official event
playlists; the importer is resumable (state in `data/conference-index-state`)
and records failures without stopping. Change the manifest deliberately and
never swap a playlist for a broad channel feed:

```bash
BACKEND=raw scripts/index-conference-playlists.sh
```

## Cloud indexer

```bash
scripts/fly-set-indexer-secrets.sh                      # admin key, optional YouTube key and cookies
BACKENDS=raw JOBS=1 scripts/fly-run-cloud-indexer.sh scripts/indexed-channels.txt
flyctl logs --app ytsearch-indexer-raw-production
```

- `RAW_SEARCH_API_URL` is set per Machine, not as an app secret: each role may
  target a different reader, and an app-wide value could point an isolated
  publication builder at production.
- `yt-dlp` is primary. `YOUTUBE_API_FALLBACK` is `auto` (stage the key when one
  is configured; the default), `1` (require it) or `0` (remove it); the Data API
  cannot supply captions.
- For an authorized cookie export, set `YT_DLP_COOKIES_FILE` to a file outside
  the repository before running the secret script; it is mounted read-only.
- The transcript cache is `transcript-cache/` at the indexer volume root:
  `/var/lib/ytsearch/transcript-cache` on the queue worker (its volume mounts
  there so it cannot hide the Raw corpus under `/data`) and `/data/...` on
  one-off indexers. Copy permitted cache data with
  `scripts/fly-upload-transcript-cache.sh` and `scripts/fly-download-transcript-cache.sh`.

## Approval queue

Frontend requests stay `pending` until reviewed. The API's JSONL file (on its
1 GB volume) is authoritative; Cloudflare Queue messages only wake the indexer.

```bash
scripts/review-index-requests.sh [list | approve REQUEST_ID | reject REQUEST_ID]
BACKENDS=raw QUEUE_TARGET_BACKEND=raw JOBS=1 LIMIT=5 scripts/fly-run-queue-indexer.sh
scripts/check-queue-indexers.sh
```

Approval queues the row and sends a wake; rejection keeps an audit row; requests
deduplicate by handle and backend. The worker serves approved rows before daily
or gap repair, exits after an idle window, and a failed row never blocks later
work.

The daily refresh runs one `ytsearch index-youtubers` over the whole embedded
list: one catalog-run ID, creators in sequence, one publication at the end. One
process per creator cannot satisfy the catalog reconciliation check.

## Fetch failures

Caption fetches are paced process-wide and retry timeouts, connection errors,
429 and 5xx with bounded backoff. If the direct caption URL keeps failing,
`yt-dlp` downloads the English subtitles through the same limits; when neither
has an English track the video is recorded as `no_captions`.

Exhausted videos get a versioned, atomically written record under
`<transcript-cache>/.failures`, classified as bot challenge, throttling,
`scheduled` (premieres and processing), captionless/private/deleted, timeout,
metadata, chunking or Raw import. Retryable categories back off from
`TRANSCRIPT_FAILURE_RETRY_SECONDS` (default 21600, six hours; Raw import
failures retry sooner); terminal ones wait for an operator. A bot challenge or
session rate limit stops the whole batch without publishing; durable writes are
kept, and rerunning the same command resumes after the provider cooldown. Keep a
one-off repair Machine's restart policy at `no`.

Tuning: `YT_DLP_CAPTION_FETCH_ATTEMPTS`, `YT_DLP_CAPTION_MIN_INTERVAL_MS`,
`YT_DLP_RATE_LIMIT_RETRY_BASE_SECONDS`, `VIDEO_TIMEOUT_SECONDS` (queue default
600) and `IDLE_GAP_FILL_MAX_NEW_VIDEOS` (default 10).

## Catalog reconciliation

After a full-list pass publishes, on the queue worker:

```bash
ytsearch failure-report /var/lib/ytsearch/transcript-cache --transcript-failure-retry-seconds 21600
scripts/check-raw-creator-reconciliation.sh
```

The checker takes the creator list from `scripts/indexed-channels.txt` and
compares each creator's discovered video IDs (written per catalog run) with one
generation-pinned Raw snapshot and the failure report. Every discovered video
must be indexed or classified, never both. It fails on stale or unattributed
records, unpublished or concurrent mutations, missing profiles, evidence from an
older run, and retries past their cooldown. Discovery fails closed too: if
either the uploads or the streams tab fails, the creator falls back to the
YouTube API or stays failed. Set `CATALOG_SNAPSHOT_JSON_PATH`,
`FAILURE_REPORT_JSON_PATH` and `DISCOVERY_MANIFEST_DIRECTORY` to check captured
evidence offline.

## Removal

Acknowledge any active queue row and stop the writer first:

```bash
scripts/index-queue-ack.sh REQUEST_ID
cargo run --release -p youtube_cli -- remove-youtuber @handle \
  --backend raw --allow-destructive-remove
```

## Verification

```bash
scripts/check-indexed-channel-status.sh @handle
scripts/check-frontend-backends.sh
scripts/check-raw-search-engine.sh
RUNS=10 scripts/benchmark-search-cases.sh
```

Every publication logs `Built Raw generation N in S s` and `Published required
Raw sequence N in S s, peak memory M MB`, so its cost needs no live watching:
`flyctl logs --app ytsearch-indexer-raw-production --no-tail | grep 'in .* s'`.

For a full migration, record discovered/indexed/skipped/retryable/terminal
counts, durable and published sequences, document/video/channel totals,
publication duration, peak RSS, disk high-water mark, search errors, and the
relevance matrix's totals and first results. Deployment order is in
[Deployment](deploy.md); recovery in [Disaster recovery](disaster-recovery.md).
