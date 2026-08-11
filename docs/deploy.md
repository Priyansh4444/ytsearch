# Deployment

The production target has four components:

| Component | Fly/Cloudflare name | Persistent state |
| --- | --- | --- |
| Raw engine | `ytsearch-raw-production` | 20 GB `raw_mutable_data` volume |
| Public API and queue | `ytsearch-api-raw-production` | 1 GB `raw_api_queue_data` volume |
| On-demand indexer | `ytsearch-indexer-raw-production` | Transcript cache and repair cursor |
| Frontend and scheduler | Cloudflare Worker `ytsearch-search` | Cloudflare Queue wake messages |

The checked-in configuration is the target configuration. The deployed legacy
queue must be copied before the Worker is redeployed; follow
[Typesense retirement](operations/typesense-retirement.md) in order.

## Prerequisites

```bash
flyctl auth whoami
bunx wrangler whoami
```

Generate separate 24-byte-or-longer public and admin keys. Never print existing
production secrets or put them in `VITE_*` variables.

## 1. Raw engine

Create the volume once, stage secrets, then deploy:

```bash
flyctl volumes create raw_mutable_data --app ytsearch-raw-production \
  --region sjc --size 20
flyctl secrets set --app ytsearch-raw-production \
  RAW_SEARCH_API_KEY="$RAW_SEARCH_API_KEY" \
  RAW_SEARCH_ADMIN_API_KEY="$RAW_SEARCH_ADMIN_API_KEY"
flyctl deploy --config fly.raw-search.toml --ha=false
```

Verify the public health endpoint and the authenticated admin readiness route.
Then perform one idempotent replacement, publish it, restart the Machine, and
verify the operation and search result remain visible.

## 2. Raw API queue

Create its volume before deploying or accepting requests:

```bash
flyctl volumes create raw_api_queue_data --app ytsearch-api-raw-production \
  --region sjc --size 1
flyctl secrets set --app ytsearch-api-raw-production \
  RAW_SEARCH_API_KEY="$RAW_SEARCH_API_KEY" \
  YTSEARCH_INDEX_QUEUE_TOKEN="$YTSEARCH_INDEX_QUEUE_TOKEN"
flyctl deploy --config fly.raw-api.toml --ha=false
```

Copy every retained legacy JSONL row before changing the Worker queue origin.
Compare IDs and statuses on both APIs; do not rely only on row counts.

## 3. Indexer

Stage the Raw URL/admin key plus optional YouTube and cookie inputs, then create
exactly one on-demand Machine:

```bash
scripts/fly-set-indexer-secrets.sh
ON_DEMAND=1 scripts/fly-run-queue-indexer.sh
scripts/check-queue-indexers.sh
```

The configured Machine must have `QUEUE_TARGET_BACKEND=raw`, restart policy
`no`, and a persistent transcript cache. A Cloudflare Queue message wakes it;
the API JSONL queue remains authoritative.

## 4. Cloudflare Worker

`wrangler.jsonc` sends both search and queue routes to
`ytsearch-api-raw-production`, controls the one Raw engine Machine, and resolves
the one Raw indexer by its environment tag.

```bash
bun run check:precommit
scripts/cloudflare-worker-deploy.sh
```

The Worker enforces the Pacific search window. `observe` logs desired lifecycle
changes without starting/stopping Machines; `enforce` is production mode.

## Cutover gates

```bash
scripts/check-frontend-backends.sh
scripts/check-production-search-contract.sh
scripts/check-staging-machines.sh
scripts/check-queue-indexers.sh
scripts/check-raw-search-engine.sh
bun run perf:bundle
```

Also verify in a clean browser:

- global `blackhole`;
- `blackhole` scoped to Veritasium and Vsauce;
- the Cursor typo regression;
- all engagement/date sorts;
- exact totals and “show more” disappearance;
- one approved creator request through publication and engine restart;
- initial JavaScript no greater than 30 KB gzip.

Run a bounded concurrent search load while publishing and require zero errors.

## Retirement and rollback

Only after every gate passes, stop the legacy writer/API/engine, mark the exact
Machines with `scripts/deprecate-typesense-fly.sh`, observe a bounded rollback
window, and destroy the resources listed in the retirement ledger.

Before destruction, rollback is a Worker configuration redeploy plus a restart
of the retained legacy engine/API. After destruction, rollback is the previous
verified Raw generation selected through the atomic manifest.

## Secret rules

- `RAW_SEARCH_API_KEY`: Raw engine and Rust API only.
- `RAW_SEARCH_ADMIN_API_KEY`: Raw engine and trusted indexers only.
- `YTSEARCH_INDEX_QUEUE_TOKEN`: Rust API, indexer, and operator tools only.
- `FLYIO_TOKEN`: Cloudflare Worker and authorized operator environment only.
- `YOUTUBE_API_KEY` and cookie files: indexer only.
- Axiom ingest and query keys remain separate.
