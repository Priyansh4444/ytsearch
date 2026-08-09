# Production observability

## Health and inventory

```bash
scripts/agent-status.sh
FORMAT=pretty scripts/agent-status.sh
scripts/check-staging-machines.sh
scripts/check-queue-indexers.sh
scripts/check-indexed-channel-status.sh @vsauce @t3dotgg
scripts/check-raw-search-engine.sh
curl -fsS https://ytsearch-api-raw-production.fly.dev/api/index-status | jq .
```

The compact agent status exits nonzero when a required surface is unhealthy.
Use `DETAILS=1` for Machine tables and
`VERBOSE=1 LINES=100 scripts/summarize-indexer-logs.sh` for bounded logs.

Exactly one Machine should have `QUEUE_TARGET_BACKEND=raw`. A stopped indexer is
healthy while idle. Failed queue rows remain visible and must not block newer
work.

## Raw engine signals

The public `/health` route proves the active reader is serving. Authenticated
admin endpoints expose readiness, generation, durable/published sequence,
operation state, counts, and publication integrity. Alert on:

- active generation absent or unreadable;
- durable sequence ahead of published sequence longer than the publication SLO;
- repeated failed operations or publications;
- RSS approaching the 4 GiB allocation;
- mutable volume above 80%;
- search errors above zero;
- queue rows stuck `running` after the indexer stops.

Publication is expected to raise RSS temporarily. The measured full-corpus peak
is 2.46 GiB and steady local post-trim RSS is 138.45 MiB. The last remote warmed
production reader measurement was 356.25 MiB.

## Indexing events

Indexer logs include request ID, creator, backend, outcome, retries, and repair
phase. Scheduled maintenance emits `daily_update_started`,
`daily_update_completed`, or `daily_update_failed`. The Worker emits
`indexer_repair_wake_enqueued`, `indexer_woken`, or `indexer_wake_failed`.

Raw lifecycle events use the `raw_search_schedule_*` prefix. Observation mode
must not mutate Fly state; enforcement mode must stop the indexer before Raw.

## Search telemetry

The API sends `search_query` events to Axiom only when the ingest key
`AXIOM_API_KEY` (or legacy `AXIOM_TOKEN`) and `AXIOM_DATASET` exist. Operator
queries use a separate `AXIOM_API_QUERY` credential.

Useful fields include:

- `duration_ms`, `backend_search_ms`, `result_processing_ms`;
- `backend_search_calls`, `backend_raw_hits`, `backend_grouped_hits`;
- `returned_videos`, `returned_hits`, `response_body_bytes`;
- `client_country` and `cf_ray`.

```bash
scripts/axiom-search-report.sh
WINDOW=7d LIMIT=200 scripts/axiom-recent-searches.sh
```

The bounded recent-query report omits IP addresses, user agents, countries, and
request IDs.

## Search and browser checks

```bash
API_BASE_URL=https://ytsearch-api-raw-production.fly.dev \
  QUERY=react YOUTUBER=@t3dotgg RUNS=10 scripts/benchmark-search-backends.sh
scripts/check-frontend-backends.sh
bun run perf:bundle
```

Before and after publication, run the fixed relevance matrix and a concurrent
load. Require zero response errors, exact totals, no stale “show more” control,
and an initial browser bundle no larger than 30 KB gzip.
