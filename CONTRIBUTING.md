# Contributing to ytsearch

Thanks for helping improve ytsearch. Changes should keep self-hosting safe,
reproducible, and independent of the project's staging infrastructure.

## Development setup

Install Rust stable, Bun, Docker with Compose, and `yt-dlp`. Fork and clone the
repository, then install frontend dependencies:

```bash
bun install --frozen-lockfile
```

For the complete local stack, follow [the self-hosting guide](docs/self-hosting.md).
The default Compose worker only polls approved queue requests; scheduled bulk
indexing is disabled unless you explicitly opt in.

## Making changes

- Create a focused branch and keep commits scoped to one concern.
- Add or update tests for behavior changes.
- Never commit API keys, cookies, creator data, local databases, or `.env` files.
- Prefer non-destructive `update` indexing. Rewipe and removal commands require
  explicit acknowledgement by design.
- Keep public API processes on search-only credentials. Admin credentials belong
  only in trusted indexing or initialization processes.

Run the complete repository check before opening a pull request:

```bash
bun run check:all
```

Browser-level core-flow and responsive checks use mocked API responses, so they
are deterministic and do not require staging access:

```bash
bunx playwright install chromium # first run only
bun run check:responsive
```

For changes to local deployment, also verify the clean Compose path:

```bash
export TYPESENSE_SELFHOST_API_KEY="$(openssl rand -hex 24)"
export YTSEARCH_INDEX_QUEUE_TOKEN="$(openssl rand -hex 24)"
export PUBLIC_API_BASE_URL=http://localhost:8787
docker compose up -d --build
curl -fsS http://localhost:8108/health
curl -fsS http://localhost:8787/api/health
curl -fsS http://localhost:8787/api/youtubers
docker compose down
```

## Pull requests

Explain the user-visible outcome, tests performed, and any migration or
operational impact. Do not include secrets or production log data in issues,
commits, screenshots, or pull requests.
