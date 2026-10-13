# Deployment

The production target has four components:

| Component | Fly/Cloudflare name | Persistent state |
| --- | --- | --- |
| Raw engine | `ytsearch-raw-production` | 20 GB `raw_mutable_data` volume |
| Public API and queue | `ytsearch-api-raw-production` | 1 GB `raw_api_queue_data` volume |
| On-demand indexer | `ytsearch-indexer-raw-production` | Transcript cache and repair cursor |
| Frontend and scheduler | Cloudflare Worker `ytsearch-search` | Cloudflare Queue wake messages |

The checked-in configuration is the target configuration.

## Prerequisites

```bash
flyctl auth login
flyctl auth whoami
bunx wrangler whoami
```

Machines API scripts need an explicit token (the repository never uses the
deprecated `flyctl auth token`). Publication spans apps, so create an
organization token:

```bash
flyctl tokens create org --org ORG_SLUG --expiry 720h
read -rsp 'Fly organization token: ' FLY_API_TOKEN; printf '\n'
export FLY_API_TOKEN
```

Paste it at the hidden prompt; never put it in the repository, shell history,
arguments or logs, and create a new one when it expires. Single-app CI should
use `flyctl tokens create deploy --app APP --expiry 720h`. The Worker needs the
multi-app token as `FLYIO_TOKEN`:

```bash
printf '%s' "$FLY_API_TOKEN" | bunx wrangler secret put FLYIO_TOKEN
```

See Fly's [token guidance](https://fly.io/docs/security/tokens/). Generate
separate public and admin keys of at least 24 bytes, and never print production
secrets or put them in `VITE_*` variables.

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

Verify `/health` and the authenticated admin readiness route, then publish one
idempotent replacement, restart, and confirm it is still visible.

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

Upload notifications need one more secret, a random value shared only with
YouTube's hub. Without it the `/api/websub/youtube` routes answer 404:

```bash
flyctl secrets set --app ytsearch-api-raw-production \
  YTSEARCH_WEBSUB_SECRET="$(openssl rand -hex 32)"
```

The per-visitor search limit needs a key shared only with the Worker, so that
direct callers of the Fly hostname cannot pick their own rate-limit identity.
Put it on the Worker and deploy the Worker first; until the API has it, the API
ignores the Worker's headers. An API with the key and a Worker without it would
rate-limit all website visitors as a few Cloudflare addresses:

```bash
key="$(openssl rand -hex 32)"
printf '%s' "$key" | bunx wrangler secret put API_PROXY_KEY
bun run deploy:frontend
printf 'YTSEARCH_API_PROXY_KEY=%s\n' "$key" |
  flyctl secrets import --app ytsearch-api-raw-production
unset key
```

## 3. Indexer

Stage the Raw URL/admin key plus optional YouTube and cookie inputs, then create
exactly one on-demand Machine:

```bash
scripts/fly-set-indexer-secrets.sh
ON_DEMAND=1 scripts/fly-run-queue-indexer.sh
scripts/check-queue-indexers.sh
```

To move the existing worker to a new indexer image, run
`INDEXER_IMAGE=<image> scripts/fly-run-queue-indexers.sh` while it is stopped,
with the organization token: after reconciling it checks the reader app too.
With an indexer-only deploy token that last check fails as unauthorized, and
the fail-closed cleanup stops the worker again (2026-10-07).

The Machine needs `QUEUE_TARGET_BACKEND=raw`, restart policy `no` and a
persistent transcript cache; a Cloudflare Queue message wakes it, and the API's
JSONL queue stays authoritative.

Daily indexing: with `DAILY_UPDATE_ENABLED=1`, the first wake more than 24 hours
after `/var/lib/ytsearch/daily-update.last-completed` runs `index-youtubers`
over the creator list and publishes once. The worker stops after five idle
minutes, so the Worker's hourly cron (`17 * * * *`) repair wake is what resumes
the daily check. Before claiming it works, verify the cron, the worker config, a
`daily_update_completed` log and the timestamp file.

## 4. Cloudflare Worker

`wrangler.jsonc` routes search and queue traffic to
`ytsearch-api-raw-production`, names the Raw engine Machine, and finds the
indexer by its environment tag.

```bash
bun run check:precommit
scripts/cloudflare-worker-deploy.sh
```

Production uses `always_on`: a five-minute reconciler restarts a stopped
reader, and availability promises 24/7 service. `observe` only reports health;
`enforce` is the optional Pacific-window policy.

## Cutover gates

```bash
scripts/verify-search-release.sh
```

This is read-only. The publication gate publishes a synthetic mutation, so it
is a deliberate operator step: follow
[Raw publication release](operations/raw-publication-release.md), which builds in
a leased detached builder, never in the serving reader. Also verify in a clean
browser:

- global `blackhole`;
- `blackhole` scoped to Veritasium and Vsauce;
- the Cursor typo regression;
- all engagement/date sorts;
- exact totals and “show more” disappearance;
- one approved creator request through publication and engine restart;
- initial JavaScript within the `bun run perf:bundle` budget
  (`scripts/check-bundle-performance.ts`).

Run a bounded concurrent search load while publishing and require zero errors.

## Rollback

Raw retains its previous verified generation through the atomic manifest;
rolling back a publication restores that generation.

## Secret rules

- `RAW_SEARCH_API_KEY`: Raw engine and Rust API only.
- `RAW_SEARCH_ADMIN_API_KEY`: Raw engine and trusted indexers only.
- `YTSEARCH_INDEX_QUEUE_TOKEN`: Rust API, indexer, and operator tools only.
- `FLY_API_TOKEN`: operator/CI environment only; required by direct Machines
  API scripts and never derived through deprecated `flyctl auth token`.
- `FLYIO_TOKEN`: Cloudflare Worker only; use the scoped multi-app token above.
- `YOUTUBE_API_KEY` and cookie files: indexer only.
- Axiom ingest and query keys remain separate.
