# Agent Handoff: ytsearch

This repo is an experimental YouTube transcript search system. Treat it as a small production staging system, not a toy demo: changes can affect live Fly machines, the Cloudflare Worker frontend, search quality, indexing cost, and creator data already indexed in the Raw search engine.

## Current Shape

`ytsearch` indexes YouTube creator captions into search backends and serves timestamped transcript search through a Solid frontend.

Active paths (production):

| Surface | URL / App | Notes |
| --- | --- | --- |
| Frontend | `https://search.pronsh.dev` | Canonical Cloudflare Worker frontend and same-origin API proxy. |
| Raw API | `ytsearch-api-raw-production` / `https://ytsearch-api-raw-production.fly.dev` | Rust `youtube_api` plus the authoritative JSONL request queue. Always running. |
| Raw engine | `ytsearch-raw-production` | Reloadable inverted index and mutation publisher. Always on: the Worker's cron keeps it started under `SEARCH_SCHEDULE_MODE=always_on` (`wrangler.jsonc`). |
| Raw indexer | `ytsearch-indexer-raw-production` | One on-demand queue worker, woken through Cloudflare Queue. |

`raw` is the only accepted backend id (`scripts/fly-run-queue-indexer.sh` rejects
anything else; `.env.example` ships `YTSEARCH_SEARCH_BACKEND=raw`).

These three apps are the whole Fly footprint.

## Read This First

Before a task, check [docs/playbooks/](docs/playbooks/README.md) for a playbook
that covers it: reader performance, dependency updates, frontend reactivity
bugs. Each lists the commands, the traps that cost time before, and facts not
worth re-deriving.

Use these docs before changing a subsystem:

- [docs/architecture.md](docs/architecture.md): crate boundaries and data flow.
- [docs/indexing-operations-guide.md](docs/indexing-operations-guide.md): indexing, queue, cloud indexer, repair commands.
- [docs/staging-architecture.md](docs/staging-architecture.md): active Fly units, indexer model, reliability rules.
- [docs/staging-observability.md](docs/staging-observability.md): machine checks, channel checks, benchmarks, Axiom fields.
- [docs/deploy.md](docs/deploy.md): deploy and environment notes.
- [README.md](README.md): local setup and common commands.

## Code Map

Frontend:

- `src/app.tsx`: app-level orchestration, search/index mode, banner/footer.
- `src/api.ts`: frontend HTTP calls.
- `src/types.ts`: API response types used by the frontend.
- `src/components/*`: UI components. Keep feature logic out of components when it belongs in API/state helpers.

Rust crates:

- `crates/youtube_api`: Axum API used by frontends. Search quality, result grouping, sorting, rate limit, queue API, Axiom search logs.
- `crates/youtube_cli`: operator CLI for validation, indexing, search, benchmarks.
- `crates/youtube_ingest`: core chunking/domain traits.
- `crates/youtube_search_service`: the `SearchBackend` trait and shared query/result types.
- `crates/youtube_search_client`: the client the API uses to call the search backend.
- `crates/youtube_raw_search`: the active Raw search engine (mutable inverted index, publication, admin surface).
- `crates/youtube_index_store`: publication/receipt bookkeeping for the Raw index.
- `crates/youtube_client`: YouTube metadata client.
- `crates/youtube_sources`: YouTube metadata source used by ingestion.
- `crates/youtube_time`: shared timestamp helpers used by ingest and sources.
- `crates/youtube_ytdlp`: caption extraction via `yt-dlp`.
- `crates/youtube_transcribe`: optional local audio transcription through `voxtype`; the cloud indexer does not have Voxtype by default.

Scripts:

- `scripts/index-all-backends-update.sh`: local update of the Raw backend.
- `scripts/index-all-backends-rewipe.sh`: destructive schema/timeframe reindex of the Raw backend.
- `scripts/fly-run-cloud-indexer.sh`: one-off detached cloud indexing.
- `scripts/fly-run-queue-indexer.sh`: starts queue-consuming indexer Machine.
- `scripts/cloud-index-queue-entrypoint.sh`: queue daemon implementation.
- `scripts/check-frontend-backends.sh`: frontend/API smoke check.
- `scripts/check-indexed-channel-status.sh`: per-handle backend/queue status.
- `scripts/check-channel-profile-images.sh`: concise audit of API-owned channel profile images.
- `scripts/check-staging-machines.sh`: Fly machine inventory.
- `scripts/benchmark-search-backends.sh`: local search benchmark.
- `scripts/raw-schema-differential.sh`: compares full API responses between two index generations; run it before shipping any index format change.
- `scripts/build-raw-generation-from-backup.sh`: rebuilds a Raw generation from the base backup archives.
- `scripts/check-rust-public-surface.sh`: fails when a Rust item is more public than its uses need (`cargo hawk`).
- `scripts/perf/`: reader profiling and A/B comparison on the real index (`reader-build.sh`, `reader-ab.sh`, `reader-profile.sh`) and bundle-size attribution (`bundle-diff.sh`). See [docs/playbooks/reader-performance.md](docs/playbooks/reader-performance.md).
- `scripts/deps-outdated.ts` (`bun run deps:outdated`): JS updates on every dist-tag and Rust major versions.

## Important System Behavior

Search:

- API search is implemented in `crates/youtube_api/src/main.rs`.
- Frontend search calls `/api/search`.
- The API groups raw transcript hits into video cards, but still allows multiple timestamp hits per video.
- Very low-signal queries, such as `I`, should not enforce all terms and should use newest-for-filter ordering.
- Do not switch to strict “must contain every term” matching unless the user explicitly asks. The desired behavior is best match.
- For queries like `fable on high`, stopwords such as `on` should not dominate snippet choice. Prefer stronger terms/highlighted title/transcript matches.
- Result ordering/sorting is applied by the API layer (`SearchSort` in `crates/youtube_api`), not by the search backend.

Indexing:

- `update` means add missing videos and preserve existing chunks.
- `rewipe` deletes a creator's chunks before rebuilding and requires explicit destructive acknowledgement. Prefer `update` when possible.
- Heavy indexing should default conservative: `JOBS=1 CHANNEL_JOBS=1`, because one Raw writer publishes at a time (see `docs/indexing-operations-guide.md`).
- Indexing rewrites each creator's profile document (indexed video and chunk counts) as it runs, so channel cards stay in step with backend chunks.
- `scripts/indexed-channels.txt` is the active creator list for bulk update/daily queue update.

Queue:

- Frontends can submit an index request for a handle. The queue API stores JSONL on a Fly volume, one row per request.
- Only one configured queue daemon may use the JSONL queue. It stays stopped while idle; Cloudflare wakes it after an approved request, and it exits after five idle minutes (`ON_DEMAND=1` sets `IDLE_EXIT_AFTER_SECONDS=300`). Multiple machines can race.
- The active queue indexer processes `raw`.
- Queue failures should log and not permanently block the queue. The queue daemon emits structured stderr logs and best-effort Axiom ingestion when Axiom env vars are present.
- Current daemon command pattern:

```bash
ON_DEMAND=1 \
QUEUE_DAEMON=1 \
DAILY_UPDATE_ENABLED=0 \
ACK_FAILED_REQUESTS=0 \
QUEUE_TARGET_BACKEND=raw \
BACKENDS="raw" \
JOBS=1 \
scripts/fly-run-queue-indexer.sh
```

Cloud indexing:

- Cloud image has `yt-dlp`, its EJS solver package, Deno, and `ffmpeg`.
- Cloud image does not have `voxtype`; missing-caption videos are skipped unless transcripts were cached locally and uploaded.
- If YouTube needs auth/cookies, use `scripts/fly-set-indexer-secrets.sh` with `YT_DLP_COOKIES_FILE` pointing to a Netscape-format cookies file. The script stages raw-Machines secrets; recreate workers to consume them. Do not print cookie contents.
- `YOUTUBE_API_FALLBACK=auto` stages the API key as a base64 file secret (`YOUTUBE_API_KEY_FILE_B64`). Entrypoints load it only inside the process so `yt-dlp` remains primary and Machine config never exposes the key. The Data API is only a metadata/discovery fallback: it cannot supply captions, so some videos need authorized cookies or a permitted local transcript cache.
- Transcript cache lives in `transcript-cache/` on the indexer volume: `/var/lib/ytsearch/transcript-cache` on the queue worker, `/data/transcript-cache` on one-off cloud indexers.

Observability:

- `youtube_api` emits Axiom `search_query` events when `AXIOM_TOKEN`/`AXIOM_API_KEY` and `AXIOM_DATASET` are configured.
- Important fields include `duration_ms`, `backend_search_ms`, `first_backend_search_ms`, `result_processing_ms`, `backend_search_calls`, `backend_raw_hits`, `returned_videos`, `returned_hits`, `response_body_bytes`, `client_country`, and `cf_ray`.
- Queue failures are structured as `component=cloud-queue-indexer` events in logs and best-effort Axiom ingestion.

## Environment And Secrets

Do not inspect or print secrets unless the user explicitly asks. Prefer loading env through existing scripts.

Common env names:

- YouTube: `YOUTUBE_API_KEY`
- Raw search: `RAW_SEARCH_API_URL`, `RAW_SEARCH_ADMIN_API_KEY`, `RAW_SEARCH_API_KEY`, `RAW_SEARCH_COLLECTION`
- API backend selector: `YTSEARCH_SEARCH_BACKEND=raw`
- Raw indexer/queue machine: `QUEUE_TARGET_BACKEND=raw`, `INDEXER_APP_NAME`, `INDEXER_VOLUME_NAME`, `INDEXER_IMAGE`
- Queue: `INDEX_QUEUE_API_BASE_URL`, `YTSEARCH_INDEX_QUEUE_TOKEN`
- Axiom: `AXIOM_TOKEN` or `AXIOM_API_KEY`, `AXIOM_DATASET`, optional `AXIOM_URL`/`AXIOM_EDGE_URL`/`AXIOM_DOMAIN`
- Cloudflare: `CLOUDFLARE_API_TOKEN` or logged-in `bunx wrangler`

Use `scripts/load-env.sh` indirectly through wrappers. If appending missing env values, append to `.env`; do not rewrite or sort the file.

## Deploy Commands

Rust API deploys:

```bash
flyctl deploy --config fly.raw-api.toml --app ytsearch-api-raw-production --primary-region sjc
```

Frontend Worker:

```bash
scripts/cloudflare-worker-deploy.sh
```

Cloud indexer/queue daemon:

```bash
scripts/fly-set-indexer-secrets.sh
scripts/fly-run-queue-indexers.sh
```

Before replacing the queue daemon, confirm it is idle:

```bash
flyctl machines list -a ytsearch-indexer-raw-production
flyctl logs --app ytsearch-indexer-raw-production --machine MACHINE_ID --no-tail
```

## Verification Commands

Run these before declaring work done:

```bash
cargo fmt --check
cargo test --workspace
bun run build
scripts/check-frontend-backends.sh
sh -n scripts/cloud-index-queue-entrypoint.sh scripts/fly-run-queue-indexer.sh
```

For frontend-only changes:

```bash
bun run frontend:check
```

For live search quality checks:

```bash
curl -fsS 'https://ytsearch-api-raw-production.fly.dev/api/search?q=fable%20on%20high&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at,title_highlight,hit_count:(.hits|length), first:.hits[0].text_highlight})'
curl -fsS 'https://ytsearch-api-raw-production.fly.dev/api/search?q=I&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at})'
```

Expected current behavior:

- `q=I&youtuber=@t3dotgg` should show Theo's most recently published indexed videos first.
- `q=fable on high&youtuber=@t3dotgg` should rank `A proper guide to Fable 5` near the top, with transcript snippets around Fable/high reasoning.

For channel/index status:

```bash
scripts/check-indexed-channel-status.sh @vsauce @thePrimagen @t3dotgg
```

For machines/cost sanity:

```bash
scripts/check-staging-machines.sh
flyctl machines list -a ytsearch-indexer-raw-production
```

## Current Known Sharp Edges

- Publisher-based index swaps: one Raw writer publishes at a time. Keep indexing concurrency conservative.
- The queue API uses JSONL on one Fly volume. Do not scale it horizontally without replacing storage/locking.
- Cloud indexing cannot run Voxtype unless the image is extended. Use public captions, cookies, or uploaded transcript cache.
- `yt-dlp` may need cookies for age-restricted or bot-challenged videos.
- The repo may have a dirty worktree. Do not revert unrelated changes.

## Working Rules For Future Agents

- Prefer existing abstractions over new ones: `youtube_search_service::SearchBackend`, `youtube_ingest` traits, backend crates, then API/CLI wiring.
- Keep admin/indexing operations out of `youtube_api`; API should use search-only keys for public search.
- Keep frontend env limited to public API URLs and queue URL. Never expose admin keys.
- Use `rg` first for code search.
- Make targeted edits; do not reformat or rewrite unrelated code.
- Use `bunx wrangler`, not plain `wrangler`, unless the binary is definitely installed.
- If changing Cloudflare Worker config or commands, check the Wrangler docs and schema first.
- If changing live Fly machines, verify machine state before and after.
- If changing indexing behavior, test both `update` and `rewipe` semantics where relevant.
- If changing search relevance, include at least one live or fixture check for:
  - low-signal query (`I`)
  - phrase/term query with stopwords (`fable on high`)
  - filtered creator query (`youtuber=@t3dotgg`)

- Leave the next session faster than this one: when a step took long because
  something was not written down, add the script, trap or fact to a playbook
  in `docs/playbooks/` in the same pull request (see its README).

## Good First Commands In A New Session

```bash
git status --short
scripts/agent-status.sh
scripts/summarize-indexer-logs.sh
```

Then read the relevant doc listed in "Read This First" before editing.
