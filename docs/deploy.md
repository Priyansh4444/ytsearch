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

`flyctl auth token` is deprecated and the repository does not call it. Commands
that use Fly's direct Machines API require an explicit, scoped token. Create an
organization token because publication coordinates more than one app:

```bash
flyctl tokens create org --org ORG_SLUG --expiry 720h
read -rsp 'Fly organization token: ' FLY_API_TOKEN; printf '\n'
export FLY_API_TOKEN
```

Paste the newly created token at the hidden prompt. Do not put it in the
repository, shell history, command arguments, or logs. A token is shown only at
creation time; when it expires or Fly rejects it, create a replacement and
rerun the command. For single-app CI, prefer the narrower
`flyctl tokens create deploy --app APP --expiry 720h`. The Cloudflare Worker
needs the multi-app token under its existing secret name:

```bash
printf '%s' "$FLY_API_TOKEN" | bunx wrangler secret put FLYIO_TOKEN
```

These commands follow Fly's current [token
guidance](https://fly.io/docs/security/tokens/) and [CLI integration
guidance](https://fly.io/docs/flyctl/integrating/).

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

The Worker cron at `17 * * * *` also sends a repair wake. The queue worker owns
the daily catalog pass: `DAILY_UPDATE_ENABLED=1`, a zero initial delay, and a
persisted `/var/lib/ytsearch/daily-update.last-completed` timestamp cause the
first idle wake after each 24-hour interval to run `index-youtubers` over the
embedded creator list and publish one generation. The worker may stop after
five idle minutes; the hourly wake is required to resume the daily check.
Verify the cron, the worker configuration, a `daily_update_completed` log, and
the persisted timestamp before claiming daily indexing is operating.

## 4. Cloudflare Worker

`wrangler.jsonc` sends both search and queue routes to
`ytsearch-api-raw-production`, controls the one Raw engine Machine, and resolves
the one Raw indexer by its environment tag.

```bash
bun run check:precommit
scripts/cloudflare-worker-deploy.sh
```

Production uses `always_on`: the five-minute reconciler restarts a stopped Raw
reader and the availability contract explicitly promises 24/7 service.
`observe` reports health without lifecycle changes; `enforce` is the optional
Pacific-window policy.

## Cutover gates

```bash
scripts/verify-search-release.sh
```

This command is non-mutating. The full-corpus publication isolation benchmark
remains an explicit operator action because it creates and publishes an
idempotent synthetic mutation. Follow
[Raw publication release](operations/raw-publication-release.md). The release
command uses one exact isolated reader and one exclusively leased detached
builder; it never runs generation construction inside the serving reader.

Also verify in a clean browser:

- global `blackhole`;
- `blackhole` scoped to Veritasium and Vsauce;
- the Cursor typo regression;
- all engagement/date sorts;
- exact totals and “show more” disappearance;
- one approved creator request through publication and engine restart;
- initial JavaScript no greater than 30 KB gzip.

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
