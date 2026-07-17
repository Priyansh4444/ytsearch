# Staging Observability

## Machine and queue health

```bash
scripts/agent-status.sh
FORMAT=pretty scripts/agent-status.sh
scripts/summarize-indexer-logs.sh
scripts/check-staging-machines.sh
scripts/check-queue-indexers.sh
scripts/check-indexed-channel-status.sh @vsauce @t3dotgg
curl -fsS https://ytsearch-typesense-staging.fly.dev/health
curl -fsS https://ytsearch-api-typesense-fly-staging.fly.dev/api/index-status | jq .
```

The agent snapshot is one compact JSON line by default and exits nonzero when
any required surface is unhealthy. Use `DETAILS=1` for machine tables,
`VERBOSE=1 scripts/check-typesense-fly.sh` for the expanded engine report, or
`VERBOSE=1 LINES=100 scripts/summarize-indexer-logs.sh` for bounded raw logs.

Exactly one `typesense-fly` worker should be started. The API and Typesense
engine remain warm. A failed channel row is retained with `failed` status and
must not block later queue work.

## Search telemetry

The API emits `search_query` events to Axiom when `AXIOM_API_KEY` (or
`AXIOM_TOKEN`) and `AXIOM_DATASET` are configured. Useful fields include:

- `duration_ms`, `backend_search_ms`, and `result_processing_ms`
- `backend_search_calls`, `backend_raw_hits`, and `backend_grouped_hits`
- `returned_videos`, `returned_hits`, and `response_body_bytes`
- `client_country` and `cf_ray`

Aggregate them without printing raw queries or credentials:

```bash
scripts/axiom-search-report.sh
```

The token needs Axiom query/read permission; ingest-only tokens correctly
receive HTTP 403 from the report script.

## Frontend analytics

PostHog is opt-in through `VITE_POSTHOG_KEY`. The frontend emits page/search
events, video opens, timestamp selections, moment expansion, and per-moment
feedback. Browser builds never receive backend or queue credentials.

## Search checks

```bash
API_BASE_URL=https://ytsearch-api-typesense-fly-staging.fly.dev \
QUERY=react YOUTUBER=@t3dotgg RUNS=10 scripts/benchmark-search-backends.sh
scripts/check-frontend-backends.sh
```
