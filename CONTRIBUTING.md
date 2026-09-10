# Contributing to ytsearch

Thanks for helping improve ytsearch. Changes should keep self-hosting safe,
reproducible, and independent of the project's staging infrastructure.

## Before you start

You can work locally without access to the maintainer's Fly, Cloudflare, Axiom,
or YouTube accounts. Most frontend and API tests use fixtures or mocked HTTP
responses and do not contact staging.

Install Rust stable, Bun, Docker with Compose, OpenSSL, and `yt-dlp`. Then fork
and clone the repository and run:

```bash
bun run setup
```

This installs locked dependencies and creates a private `.env` with random
local credentials. It preserves an existing `.env`, so the command is safe to
run again.

Start the complete local stack:

```bash
docker compose up -d --build
```

Open `http://localhost:3000`. The default worker processes only approved queue
requests; scheduled bulk indexing remains disabled. Read the
[self-hosting guide](docs/self-hosting.md) when you need persistence, cookies,
or deployment details.

## Find your way around

| Area | Start here | Focused check |
| --- | --- | --- |
| Frontend | `src/app.tsx`, `src/components/` | `bun run frontend:check` |
| Cloudflare Worker | `worker/index.ts` | `bun run test:worker` |
| Public API | `crates/youtube_api` | `cargo test -p youtube_api` |
| Indexing CLI | `crates/youtube_cli` | `cargo test -p youtube_cli` |
| Raw query engine | `crates/youtube_raw_search` | `cargo test -p youtube_raw_search` |
| Raw indexing boundary | `crates/youtube_index_store` | `cargo test -p youtube_index_store` |
| Operations and deployment | `scripts/`, `docs/` | `sh -n scripts/<changed-script>.sh` |

The [architecture guide](docs/architecture.md) explains how these areas fit
together. If you are unsure where a change belongs, open an issue with the
problem you want to solve before writing a large patch.

## Making changes

- Create a focused branch and keep commits scoped to one concern.
- Add or update tests for behavior changes.
- Never commit API keys, cookies, creator data, local databases, or `.env` files.
- Prefer non-destructive `update` indexing. Rewipe and removal commands require
  explicit acknowledgement by design.
- Keep public API processes on search-only credentials. Admin credentials belong
  only in trusted indexing or initialization processes.

During development, run the smallest focused check from the table above. Before
opening a pull request, run:

```bash
bun run check:precommit
```

This fast gate covers formatting, linting, type checks, strict Clippy, unit
tests, the production build, and bundle budgets. Run the complete suite before
submitting substantial or cross-cutting changes:

```bash
bun run check
```

The complete suite also runs workspace tests, browser flows, dependency policy,
and shell checks. Install the optional repository hook once per clone to run the
fast gate before each commit:

```bash
bun run hooks:install
```

Browser-level core-flow and responsive checks use mocked API responses, so they
are deterministic and do not require staging access:

```bash
bunx playwright install chromium # first run only
bun run check:responsive
```

For changes to local deployment, also verify the clean Compose path:

```bash
export RAW_SEARCH_ADMIN_API_KEY="$(openssl rand -hex 24)"
export RAW_SEARCH_API_KEY="$(openssl rand -hex 24)"
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
operational impact. Keep pull requests focused; separate unrelated cleanup from
behavior changes. The pull request template includes the required safety and
verification checks.

Do not include secrets, cookies, creator data, production log data, or private
maintainer notes in issues, commits, screenshots, or pull requests. Use the
repository's private security reporting flow for vulnerabilities.
