# Staging Architecture

Typesense is the only active search backend. The four always-on Fly Machines
are deliberately separated so indexing load cannot take down public search.

| Fly app | Purpose | Keeps running? |
| --- | --- | --- |
| `ytsearch-api-typesense-fly-staging` | Public search API | Yes |
| `ytsearch-typesense-staging` | Typesense engine and data volume | Yes |
| `ytsearch-index-queue-staging` | Private JSONL queue API | Yes |
| `ytsearch-indexer-staging` | One Typesense queue/indexing daemon and transcript cache | Yes while the queue or daily update is active |

Cloudflare Pages is outside Fly and serves `https://ytsearch-755.pages.dev`.

```mermaid
flowchart LR
  Browser --> Pages[Cloudflare Pages]
  Pages --> API[Typesense search API]
  API --> Engine[Typesense engine]
  Pages --> Queue[Queue API]
  Queue --> Worker[Typesense queue worker]
  Worker --> YouTube[yt-dlp / YouTube]
  Worker --> Engine
```

## Reliability And Cost Rules

1. Keep exactly one persistent queue worker. JSONL has no distributed lock.
2. The worker uses 2 shared CPUs and 4 GB RAM for sustained `yt-dlp` work, a
   10 GB encrypted transcript-cache volume, and a one-request queue pass to
   avoid a large creator starving later requests.
3. The worker uses `yt-dlp` first. A staged YouTube Data API key is a metadata
   discovery fallback only; it cannot retrieve transcript captions.
4. Public search and Typesense stay warm. Stop the worker only after confirming
   the queue is empty and pausing daily updates; never stop the API or engine
   as a cost-saving measure while the frontend is expected to serve traffic.
5. Update mode preserves existing chunks. Refresh indexed-count documents after
   a completed bulk pass so the frontend's `indexed / public` count stays true.

## Operator Surfaces

| Task | Command |
| --- | --- |
| Machine inventory | `scripts/check-staging-machines.sh` |
| Queue-worker invariant | `scripts/check-queue-indexers.sh` |
| Start the worker | `scripts/fly-run-queue-indexers.sh` |
| Per-channel status | `scripts/check-indexed-channel-status.sh @handle` |
| Refresh channel counts | `scripts/refresh-all-backend-counts.sh` |
| Search benchmark | `QUERY=react RUNS=10 scripts/benchmark-search-backends.sh` |

Meilisearch is source-level legacy only. Its Fly engine, API, worker, volume,
and indexed data were destroyed; see `docs/legacy/meilisearch-retirement.md`.
