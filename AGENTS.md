# Agent Handoff: ytsearch

YouTube transcript search: Rust indexer and API, a Raw inverted-index engine, a
Solid frontend, and a Cloudflare Worker. Treat it as production: changes can
affect live Fly Machines, the Worker, search quality, indexing cost, and indexed
creator data.

| Surface | App / URL | Notes |
| --- | --- | --- |
| Frontend | `https://search.pronsh.dev` | Cloudflare Worker: static app, same-origin API proxy, Fly control |
| Raw API | `ytsearch-api-raw-production` | Rust `youtube_api` plus the authoritative JSONL request queue; always running |
| Raw engine | `ytsearch-raw-production` | Reloadable index and publisher; always on (`SEARCH_SCHEDULE_MODE=always_on`) |
| Raw indexer | `ytsearch-indexer-raw-production` | One on-demand queue worker, woken through Cloudflare Queue |

These three Fly apps are the whole footprint, and `raw` is the only backend id.

## Read first

- [docs/playbooks/](docs/playbooks/README.md): commands, known traps and facts for
  reader performance, dependency updates and frontend reactivity bugs.
- [docs/architecture.md](docs/architecture.md): crates, data flow, credential boundary.
- [docs/indexing-operations-guide.md](docs/indexing-operations-guide.md): indexing, queue, repair.
- [docs/staging-architecture.md](docs/staging-architecture.md) and
  [docs/staging-observability.md](docs/staging-observability.md): production units and checks.
- [docs/deploy.md](docs/deploy.md): deploy commands and secret placement.

Entry points: `src/app.tsx` (orchestration), `src/api.ts` (HTTP), `src/types.ts`
(API types), `worker/index.ts` (Worker). Scripts worth knowing:

- `scripts/raw-schema-differential.sh`: compares full API responses between two
  index generations. Run it before shipping any index format change.
- `scripts/build-raw-generation-from-backup.sh`: rebuilds a generation from the
  base backup archives.
- `scripts/check-channel-profile-images.sh`: audits channel profile images.
- `scripts/perf/` and `bun run deps:outdated`: see the playbooks.

## Behavior to preserve

Search:

- Ranking is best match. Do not switch to "must contain every term" unless asked.
- Very low-signal queries such as `I` do not enforce all terms and use
  newest-for-filter ordering.
- Stopwords such as `on` in `fable on high` must not dominate snippet choice.
- Results group into video cards with multiple timestamp hits per video. Sorting
  happens in the API (`SearchSort` in `crates/youtube_api`), not the backend.

Indexing and queue:

- `update` adds missing videos and preserves existing chunks; prefer it.
  `rewipe` is destructive and needs explicit acknowledgement.
- One Raw writer publishes at a time: keep `JOBS=1 CHANNEL_JOBS=1` unless measured.
- Indexing keeps each creator's video and chunk counts current.
- `scripts/indexed-channels.txt` is the creator list for bulk and daily updates.
- Exactly one queue worker may use the JSONL queue. Queue failures must log and
  never block later requests.
- The cloud indexer image has `yt-dlp` (with Deno for its EJS solver),
  `ffmpeg`, and pinned CPU whisper.cpp/base.en. `TRANSCRIBE_MISSING=1` enables
  captionless fallback with a shared 480-second budget and resumable chunks.
  Voxtype remains the local CLI default. Final transcript caches are portable;
  inaccessible and speech-free videos retain explicit classified outcomes.

## Verify before declaring done

```bash
bun run check:precommit   # what the commit hook runs
bun run check             # the full CI suite
```

Live search quality, after relevance changes:

```bash
curl -fsS 'https://ytsearch-api-raw-production.fly.dev/api/search?q=fable%20on%20high&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at,title_highlight,hit_count:(.hits|length), first:.hits[0].text_highlight})'
curl -fsS 'https://ytsearch-api-raw-production.fly.dev/api/search?q=I&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at})'
```

`q=I` should list Theo's most recently published indexed videos first;
`fable on high` should rank `A proper guide to Fable 5` near the top with
snippets around Fable and high reasoning. Production state:
`scripts/agent-status.sh`, `scripts/check-indexed-channel-status.sh @handle`.

## Working rules

- Prefer existing abstractions (`SearchBackend`, `youtube_ingest` traits) over new ones.
- Keep admin and indexing operations out of `youtube_api`; public search uses
  search-only keys. Never expose admin keys or put secrets in `VITE_*`.
- Do not inspect or print secrets. Load env through the existing scripts; append
  to `.env`, never rewrite it.
- Use `bunx wrangler`. Check Wrangler docs before changing Worker config.
- Verify Fly Machine state before and after changing it.
- Relevance changes need a check for a low-signal query (`I`), a stopword query
  (`fable on high`) and a creator-filtered query.
- Indexing changes need `update` and `rewipe` semantics checked where relevant.
- The worktree may be dirty; do not revert unrelated changes.
- When a step took long because something was not written down, add the script,
  trap or fact to a playbook in `docs/playbooks/` in the same pull request.
