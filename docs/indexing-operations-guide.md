# Indexing Operations Guide

## Frontend URLs

Each preview points search at a different backend API and posts `@handle` index requests to the central Fly queue.

| Frontend | Search API | Queue API |
| --- | --- | --- |
| https://frontend-meilisearch.ytsearch-755.pages.dev | `ytsearch-api-meili-staging` | `ytsearch-index-queue-staging` |
| https://frontend-typesense-cloud.ytsearch-755.pages.dev | `ytsearch-api-pronsh-sandbox` | `ytsearch-index-queue-staging` |
| https://frontend-typesense-fly.ytsearch-755.pages.dev | `ytsearch-api-typesense-fly-staging` | `ytsearch-index-queue-staging` |

## One-Off Full Update

Run this in a terminal or background job. It updates all three indexes, then refreshes profile counts in all three:

```bash
JOBS=8 nohup scripts/index-all-backends-update.sh scripts/indexed-channels.txt \
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

The script runs:

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
cargo run --release -p youtube_cli -- refresh-youtubers scripts/indexed-channels.txt --backend meilisearch
cargo run --release -p youtube_cli -- refresh-youtubers scripts/indexed-channels.txt --backend typesense
TYPESENSE_API_URL=https://ytsearch-typesense-staging.fly.dev \
TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
cargo run --release -p youtube_cli -- refresh-youtubers scripts/indexed-channels.txt --backend typesense
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

Global:

```bash
QUERY=fable YOUTUBER=@t3dotgg scripts/benchmark-search-globalping.sh
```
