# Deployment

ytsearch needs four processes: Typesense, `youtube-api`, one queue worker, and
a static frontend. The supported provider-neutral path is `compose.yaml`; see
`docs/self-hosting.md`.

## Fly staging

Active apps:

- `ytsearch-typesense-staging`: persistent Typesense engine.
- `ytsearch-api-typesense-fly-staging`: public API and JSONL queue owner.
- `ytsearch-indexer-staging`: one on-demand update/queue worker, woken by the
  Cloudflare Queue consumer and stopped after an idle window.

Deploy the API:

```bash
flyctl deploy --config fly.typesense-api.toml \
  --app ytsearch-api-typesense-fly-staging --primary-region sjc
```

Stage worker secrets without printing them, then deliberately recreate the
worker so raw Machine file secrets are mounted:

```bash
YOUTUBE_API_FALLBACK=auto scripts/fly-set-indexer-secrets.sh
scripts/check-queue-indexers.sh
scripts/fly-run-queue-indexers.sh
```

The public API remains warm. Typesense follows the scheduled availability
policy described in `docs/staging-architecture.md`. The indexer uses
conservative single-writer concurrency because indexing and public search share
one engine.

## Cloudflare Pages

The browser build needs only public values:

```env
VITE_API_BASE_URL=https://api.example.com
VITE_INDEX_QUEUE_API_BASE_URL=https://api.example.com
VITE_POSTHOG_KEY=
VITE_POSTHOG_HOST=https://us.i.posthog.com
```

The public Worker only serves `PUBLIC_APP_ORIGIN` (configured as
`https://search.pronsh.dev`) and localhost development hosts. The Rust API CORS
allowlist is controlled by comma-separated `YTSEARCH_ALLOWED_ORIGINS` and also
always permits `localhost`, `127.0.0.1`, and `::1` for local development. Do not
add Pages preview or `workers.dev` origins to production.

Deploy the production branch:

```bash
scripts/cloudflare-worker-deploy.sh
```

The committed Worker configuration starts in observation mode:

```json
"SEARCH_SCHEDULE_MODE": "observe"
```

Observation mode neither blocks search nor starts/stops Fly Machines. Before
changing it to `enforce`:

1. Observe at least 48 hours of `typesense_schedule_observed` events.
2. Confirm `FLY_TYPESENSE_MACHINE_ID` is the production Machine, not a
   benchmark Machine.
3. Exercise start, health, and graceful stop against an isolated Machine.
4. Confirm the production node is healthy before noon for one scheduled cycle.
5. Deploy `fly.typesense.toml` with automatic starts disabled.
6. Change the Worker mode to `enforce` and deploy it.
7. Observe two complete prewarm/open/stop cycles.

Emergency rollback:

```bash
flyctl machine start MACHINE_ID --app ytsearch-typesense-staging
```

Then restore `auto_start_machines=true`, `min_machines_running=1`, change the
Worker to observation mode, and redeploy both configurations.

Never expose Typesense admin keys, queue tokens, YouTube keys, cookies, or
Axiom tokens in `VITE_*` variables.

## Checks

```bash
scripts/check-frontend-backends.sh
scripts/check-staging-machines.sh
scripts/check-queue-indexers.sh
curl -fsS https://ytsearch-typesense-staging.fly.dev/health
curl -fsS https://search.pronsh.dev/api/availability | jq .
```
