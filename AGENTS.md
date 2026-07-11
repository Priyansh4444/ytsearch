# Agent Handoff: ytsearch

This repo is an experimental YouTube transcript search system. Treat it as a small production staging system, not a toy demo: changes can affect live Fly machines, Cloudflare Pages previews, search quality, indexing cost, and creator data already indexed in Meilisearch/Typesense.

## Current Shape

`ytsearch` indexes YouTube creator captions into search backends and serves timestamped transcript search through a Solid frontend.

Active staging paths:

| Surface | URL / App | Notes |
| --- | --- | --- |
| Frontend | `https://ytsearch-755.pages.dev` | Production Pages deployment, connected to Typesense. |
| Typesense Fly API | `ytsearch-api-typesense-fly-staging` / `https://ytsearch-api-typesense-fly-staging.fly.dev` | Rust `youtube_api`, backend `typesense`. |
| Typesense engine | `ytsearch-typesense-staging` | Self-hosted Typesense on Fly. |
| Queue API | `ytsearch-index-queue-staging` | Private JSONL-backed index request queue. |
| Cloud indexer | `ytsearch-indexer-staging` | One persistent Typesense queue worker. |

Meilisearch is source-level legacy only; its Fly machines, volume, API, worker,
and indexed data were destroyed. See `docs/legacy/meilisearch-retirement.md`.

Managed Typesense Cloud is intentionally removed from active staging. Do not reintroduce it unless the user explicitly asks.

## Read This First

Use these docs before changing a subsystem:

- [docs/architecture.md](docs/architecture.md): crate boundaries and data flow.
- [docs/indexing-operations-guide.md](docs/indexing-operations-guide.md): indexing, queue, cloud indexer, repair commands.
- [docs/staging-architecture.md](docs/staging-architecture.md): active Fly units, indexer model, reliability rules.
- [docs/staging-observability.md](docs/staging-observability.md): machine checks, channel checks, benchmarks, Axiom fields.
- [docs/search-backend-comparison.md](docs/search-backend-comparison.md): Meili vs Typesense operational tradeoffs.
- [docs/deploy.md](docs/deploy.md): deploy and environment notes.
- [README.md](README.md): local setup and common commands.

## Code Map

Frontend:

- `src/App.tsx`: app-level orchestration, search/index mode, banner/footer.
- `src/api.ts`: frontend HTTP calls.
- `src/types.ts`: API response types used by the frontend.
- `src/components/*`: UI components. Keep feature logic out of components when it belongs in API/state helpers.

Rust crates:

- `crates/youtube_api`: Axum API used by frontends. Search quality, result grouping, rate limit, queue API client fields, Axiom search logs.
- `crates/youtube_cli`: operator CLI for validation, indexing, refresh counts, search, benchmarks.
- `crates/youtube_ingest`: core chunking/domain traits.
- `crates/youtube_search`: shared search trait/query types used by API and backends.
- `crates/youtube_meilisearch`: Meilisearch admin/search implementation. Preferred backend.
- `crates/youtube_typesense`: Typesense admin/search implementation. Still active for Fly comparison, but secondary.
- `crates/youtube_sources`: YouTube metadata source.
- `crates/youtube_ytdlp`: caption extraction via `yt-dlp`.
- `crates/youtube_transcribe`: optional local audio transcription through `voxtype`; cloud indexer does not have Voxtype by default.

Scripts:

- `scripts/index-all-backends-update.sh`: local update for active backends.
- `scripts/index-all-backends-rewipe.sh`: destructive schema/timeframe reindex for active backends.
- `scripts/fly-run-cloud-indexer.sh`: one-off detached cloud indexing.
- `scripts/fly-run-queue-indexer.sh`: starts queue-consuming indexer Machine.
- `scripts/cloud-index-queue-entrypoint.sh`: queue daemon implementation.
- `scripts/check-frontend-backends.sh`: frontend/API smoke check.
- `scripts/check-indexed-channel-status.sh`: per-handle backend/queue status.
- `scripts/check-channel-profile-images.sh`: concise audit of API-owned channel profile images.
- `scripts/check-staging-machines.sh`: Fly machine inventory.
- `scripts/benchmark-search-backends.sh`: local search benchmark.
- `scripts/benchmark-search-globalping.sh`: public multi-location benchmark.

## Important System Behavior

Search:

- API search is implemented in `crates/youtube_api/src/main.rs`.
- Frontend search calls `/api/search`.
- The API groups raw transcript hits into video cards, but still allows multiple timestamp hits per video.
- Very low-signal queries, such as `I`, should not enforce all terms. Current Meili behavior treats them as newest-for-filter using backend sort.
- Do not switch to strict “must contain every term” matching unless the user explicitly asks. The desired behavior is best match.
- For queries like `fable on high`, stopwords such as `on` should not dominate snippet choice. Prefer stronger terms/highlighted title/transcript matches.
- Meili supports `published_at` sorting in the current index settings. Typesense Fly does not have full parity for newest ordering because the current schema lacks a proper numeric published timestamp sort field.

Indexing:

- `update` means add missing videos and preserve existing chunks.
- `rewipe` deletes a creator's chunks before rebuilding and requires explicit destructive acknowledgement. Prefer `update` or `copy-youtuber-chunks` from the other backend when possible.
- Heavy indexing should default conservative: `JOBS=1` in cloud, `JOBS=3` or `JOBS=4` for Meili on current staging if local.
- High Meili write concurrency can produce 408/502s or disk pressure because Meili creates update files.
- Count refresh matters. After indexing, run `refresh-youtuber-counts(-list)` or the wrapper scripts so frontend channel cards match backend chunks. Use `refresh-youtuber(s)` only when profile metadata/public counts/avatars also need YouTube enrichment.
- `scripts/indexed-channels.txt` is the active creator list for bulk update/daily queue update.

Queue:

- Frontends can submit an index request for a handle. The queue API stores JSONL on a Fly volume.
- Only one persistent queue daemon should run against the JSONL queue. Multiple daemons can race.
- The active queue indexer should process both `meilisearch` and `typesense-fly`.
- Queue failures should log and not permanently block the queue. The queue daemon emits structured stderr logs and best-effort Axiom ingestion when Axiom env vars are present.
- Current daemon command pattern:

```bash
QUEUE_DAEMON=1 \
DAILY_UPDATE_ENABLED=1 \
ACK_FAILED_REQUESTS=1 \
BACKENDS="meilisearch typesense-fly" \
JOBS=1 \
scripts/fly-run-queue-indexer.sh
```

Cloud indexing:

- Cloud image has `yt-dlp`, its EJS solver package, Deno, and `ffmpeg`.
- Cloud image does not have `voxtype`; missing-caption videos are skipped unless transcripts were cached locally and uploaded.
- If YouTube needs auth/cookies, use `scripts/fly-set-indexer-secrets.sh` with `YT_DLP_COOKIES_FILE` pointing to a Netscape-format cookies file. The script stages raw-Machines secrets; recreate workers to consume them. Do not print cookie contents.
- `YOUTUBE_API_FALLBACK=auto` stages the API key as a base64 file secret. Entrypoints load it only inside the process so `yt-dlp` remains primary and Machine config never exposes the key.
- Transcript cache lives on the indexer volume at `/data/transcript-cache`.

Observability:

- `youtube_api` emits Axiom `search_query` events when `AXIOM_TOKEN`/`AXIOM_API_KEY` and `AXIOM_DATASET` are configured.
- Important fields include `duration_ms`, `backend_search_ms`, `first_backend_search_ms`, `result_processing_ms`, `backend_search_calls`, `backend_raw_hits`, `returned_videos`, `returned_hits`, `response_body_bytes`, `client_country`, and `cf_ray`.
- Queue failures are structured as `component=cloud-queue-indexer` events in logs and best-effort Axiom ingestion.

## Environment And Secrets

Do not inspect or print secrets unless the user explicitly asks. Prefer loading env through existing scripts.

Common env names:

- YouTube: `YOUTUBE_API_KEY`
- Meili: `MEILI_HTTP_ADDR`, `MEILI_MASTER_KEY`, `MEILI_SEARCH_API_KEY`, `MEILI_TRANSCRIPTS_INDEX`
- Typesense Fly: `TYPESENSE_SELFHOST_API_URL`, `TYPESENSE_SELFHOST_API_KEY`, `TYPESENSE_API_URL`, `TYPESENSE_ADMIN_API_KEY`, `TYPESENSE_SEARCH_API_KEY`
- API backend selector: `YTSEARCH_SEARCH_BACKEND=meilisearch|typesense`
- Queue: `INDEX_QUEUE_API_BASE_URL`, `YTSEARCH_INDEX_QUEUE_TOKEN`
- Axiom: `AXIOM_TOKEN` or `AXIOM_API_KEY`, `AXIOM_DATASET`, optional `AXIOM_URL`/`AXIOM_EDGE_URL`/`AXIOM_DOMAIN`
- Cloudflare: `CLOUDFLARE_API_TOKEN` or logged-in `bunx wrangler`

Use `scripts/load-env.sh` indirectly through wrappers. If appending missing env values, append to `.env`; do not rewrite or sort the file.

## Deploy Commands

Rust API deploys:

```bash
flyctl deploy --config fly.staging.toml --app ytsearch-api-meili-staging --primary-region sjc
flyctl deploy --config fly.typesense-api.toml --app ytsearch-api-typesense-fly-staging --primary-region sjc
```

Frontend aliases:

```bash
scripts/cloudflare-pages-deploy-backends.sh
```

Cloud indexer/queue daemon:

```bash
scripts/fly-set-indexer-secrets.sh
QUEUE_DAEMON=1 DAILY_UPDATE_ENABLED=1 ACK_FAILED_REQUESTS=1 scripts/fly-run-queue-indexer.sh
```

Before replacing the queue daemon, confirm it is idle:

```bash
flyctl machines list -a ytsearch-indexer-staging
flyctl logs --app ytsearch-indexer-staging --machine MACHINE_ID --no-tail
```

## Verification Commands

Run these before declaring work done:

```bash
cargo fmt --check
cargo test -p youtube_api -p youtube_meilisearch -p youtube_typesense -p youtube_search
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
curl -fsS 'https://ytsearch-api-meili-staging.fly.dev/api/search?q=fable%20on%20high&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at,title_highlight,hit_count:(.hits|length), first:.hits[0].text_highlight})'
curl -fsS 'https://ytsearch-api-meili-staging.fly.dev/api/search?q=I&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at})'
```

Expected current Meili behavior:

- `q=I&youtuber=@t3dotgg` should show newest Theo videos, including July 2026 videos if indexed.
- `q=fable on high&youtuber=@t3dotgg` should rank `A proper guide to Fable 5` near the top, with transcript snippets around Fable/high reasoning.

For channel/index status:

```bash
scripts/check-indexed-channel-status.sh @vsauce @thePrimagen @t3dotgg
```

For machines/cost sanity:

```bash
scripts/check-staging-machines.sh
flyctl machines list -a ytsearch-indexer-staging
```

## Current Known Sharp Edges

- Typesense Fly needs `published_at_timestamp` on old chunks for newest ordering on low-signal queries. Backfill it with a non-destructive Meili-to-Typesense chunk copy when Meili has the complete creator.
- Meili can become slow during heavy writes. Keep indexing concurrency conservative and watch disk/update-file pressure.
- The queue API uses JSONL on one Fly volume. Do not scale it horizontally without replacing storage/locking.
- Cloud indexing cannot run Voxtype unless the image is extended. Use public captions, cookies, or uploaded transcript cache.
- `yt-dlp` may need cookies for age-restricted or bot-challenged videos.
- The repo may have a dirty worktree. Do not revert unrelated changes.

## Working Rules For Future Agents

- Prefer existing abstractions over new ones: `youtube_search::TranscriptSearchBackend`, `youtube_ingest` traits, backend crates, then API/CLI wiring.
- Keep admin/indexing operations out of `youtube_api`; API should use search-only keys for public search.
- Keep frontend env limited to public API URLs and queue URL. Never expose admin keys.
- Use `rg` first for code search.
- Use `apply_patch` for manual edits.
- Use `bunx wrangler`, not plain `wrangler`, unless the binary is definitely installed.
- If changing Cloudflare Workers/Pages config or commands, check Wrangler docs/schema first.
- If changing live Fly machines, verify machine state before and after.
- If changing indexing behavior, test both `update` and `rewipe` semantics where relevant.
- If changing search relevance, include at least one live or fixture check for:
  - low-signal query (`I`)
  - phrase/term query with stopwords (`fable on high`)
  - filtered creator query (`youtuber=@t3dotgg`)

## Good First Commands In A New Session

```bash
git status --short
rg -n "TODO|FIXME|typesense cloud|TYPESENSE_CLOUD|require_all_terms|matchingStrategy"
scripts/check-frontend-backends.sh
scripts/check-staging-machines.sh
```

Then read the relevant doc listed in "Read This First" before editing.


CURRENTLY This change was implemented: I changed the queue so each frontend request becomes separate backend-specific queue rows, so the two daemons do not race or steal each other’s acknowledgements. I migrated the old backlog into split rows and removed the old shared daemon.

Also removed `YOUTUBE_API_KEY` from the indexer secret set and recreated the machines, so the new runs are using `yt-dlp` first without falling back into the exhausted YouTube API quota. Logs confirm `yt-dlp` discovery is active. Some videos are still being skipped because YouTube is asking `yt-dlp` for cookies/bot confirmation; that will need cookies if we want those captions.
But I want to remove it and also have YOUTUBE as a secondary option incase `yt-dlp` fails or is not available.
