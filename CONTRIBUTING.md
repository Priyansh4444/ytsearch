# Contributing to ytsearch

Changes should keep self-hosting safe, reproducible, and independent of the
maintainer's infrastructure. You do not need Fly, Cloudflare, Axiom or YouTube
accounts: tests use fixtures and mocked HTTP.

Set up with `bun run setup` and run the stack as described in the
[README](README.md). The default Compose worker processes only approved queue
requests; scheduled bulk indexing stays off.

## Where things live

| Area | Start here | Focused check |
| --- | --- | --- |
| Frontend | `src/app.tsx`, `src/components/` | `bun run frontend:check` |
| Cloudflare Worker | `worker/index.ts` | `bun run test:worker` |
| Public API | `crates/youtube_api` | `cargo test -p youtube_api` |
| Indexing CLI | `crates/youtube_cli` | `cargo test -p youtube_cli` |
| Raw query engine | `crates/youtube_raw_search` | `cargo test -p youtube_raw_search` |
| Raw indexing boundary | `crates/youtube_index_store` | `cargo test -p youtube_index_store` |
| Operations | `scripts/`, `docs/` | `bash -n scripts/<changed-script>.sh` |

The [architecture guide](docs/architecture.md) shows how these fit together.
For a large change, open an issue describing the problem first.

## Making changes

- Keep branches and commits to one concern, and add tests for behavior changes.
- Never commit API keys, cookies, creator data, local databases or `.env` files.
- Prefer non-destructive `update` indexing; rewipe and removal require explicit
  acknowledgement by design.
- Public API processes get search-only credentials; admin credentials belong
  only to trusted indexing or initialization processes.

## Checks

```bash
bun run check:precommit   # formatting, lint, types, Clippy, unit tests, build, budgets
bun run check             # adds workspace tests, browser flows, dependency policy, shell checks
bun run hooks:install     # run the fast gate before each commit
```

Useful focused commands: `bun run test:frontend:watch`, `bun run check:responsive`
(mocked browser flows; run `bunx playwright install chromium` once), and
`bun run perf:frontend` (measures the deployed site). For UI work on real data
without a local backend, run
`YTSEARCH_DEV_API=https://ytsearch-api-raw-production.fly.dev bun run dev`. For local deployment
changes, also verify the clean Compose path and health checks in the
[self-hosting guide](docs/self-hosting.md).

## Pull requests

Explain the user-visible outcome, the tests you ran, and any migration or
operational impact. Keep unrelated cleanup separate from behavior changes. Do
not include secrets, cookies, creator data or production logs in issues,
commits or screenshots, and report vulnerabilities privately (see
[SECURITY.md](SECURITY.md)).
