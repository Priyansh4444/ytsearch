# Deployment

ytsearch needs four processes: Typesense, `youtube-api`, one queue worker, and
a static frontend. The supported provider-neutral path is `compose.yaml`; see
`docs/self-hosting.md`.

## Fly staging

Active apps:

- `ytsearch-typesense-staging`: persistent Typesense engine.
- `ytsearch-api-typesense-fly-staging`: public API and JSONL queue owner.
- `ytsearch-indexer-staging`: one persistent update/queue worker.

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

The public API and Typesense engine remain warm. The worker uses conservative
single-writer concurrency because indexing and public search share one engine.

## Cloudflare Pages

The browser build needs only public values:

```env
VITE_API_BASE_URL=https://api.example.com
VITE_INDEX_QUEUE_API_BASE_URL=https://api.example.com
VITE_POSTHOG_KEY=
VITE_POSTHOG_HOST=https://us.i.posthog.com
```

Deploy the production branch:

```bash
TYPESENSE_PAGES_BRANCH=main \
TYPESENSE_FLY_API_BASE_URL=https://ytsearch-api-typesense-fly-staging.fly.dev \
scripts/cloudflare-pages-deploy-backends.sh
```

Never expose Typesense admin keys, queue tokens, YouTube keys, cookies, or
Axiom tokens in `VITE_*` variables.

## Checks

```bash
scripts/check-frontend-backends.sh
scripts/check-staging-machines.sh
scripts/check-queue-indexers.sh
curl -fsS https://ytsearch-typesense-staging.fly.dev/health
```
