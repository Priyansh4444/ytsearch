# ytsearch

A Rust + SolidJS workspace for indexing YouTube creator transcripts into Typesense and searching exact timestamped moments.

Staging runs one public Pages site (`https://ytsearch-755.pages.dev`), one
Typesense API/queue machine, one Typesense engine, and one bounded-concurrency
indexer. Meilisearch is source-level legacy only; do not follow historical
Meilisearch commands below. See `docs/staging-architecture.md` for the live
topology and `docs/legacy/meilisearch-retirement.md` for the retirement record.

The current stack:

- Rust workspace for YouTube metadata, transcript ingestion, Typesense indexing/search, and a small HTTP API.
- SolidJS + Vite + TypeScript + Tailwind frontend using Bun.
- Self-hosted Typesense search storage.
- `yt-dlp` caption extraction and creator discovery as the primary source, with an optional YouTube Data API fallback.
- Optional local audio transcription fallback through `voxtype` when captions are unavailable.

> Legal/compliance note: this project is experimental software. It is not legal advice. You are responsible for complying with YouTube's Terms of Service, YouTube API Services Terms and Developer Policies, copyright law, privacy law, and the rights of creators whose content you process.

## Workspace layout

```text
crates/youtube_api          Rust HTTP API for frontend search
crates/youtube_cli          CLI for validation, indexing, and search
crates/youtube_client       Internal YouTube Data API v3 wrapper
crates/youtube_ingest       Core transcript chunking/domain model
crates/youtube_jsonl        JSONL chunk persistence helpers
crates/youtube_semantic     Experimental embedding/semantic helpers
crates/youtube_search       Shared search backend traits, query type, and indexed creator document
crates/youtube_meilisearch  Meilisearch admin/search clients
crates/youtube_sources      YouTube metadata sources + source fallback combinators
crates/youtube_ytdlp        yt-dlp public caption transcript source
crates/youtube_transcribe   yt-dlp audio extraction + voxtype transcription fallback
crates/youtube_typesense    Typesense admin/search clients used for comparison and rollback
src/                        SolidJS frontend
```

## Required environment

Create a local `.env` file from `.env.example`. It is intentionally gitignored.

```env
# Optional YouTube Data API fallback for metadata/discovery only.
YOUTUBE_API_KEY=...
YTSEARCH_SEARCH_BACKEND=typesense
TYPESENSE_API_URL=https://ytsearch-typesense-staging.fly.dev
TYPESENSE_SEARCH_API_KEY=...
```

Optional:

```env
TYPESENSE_SELFHOST_API_URL=https://ytsearch-typesense-staging.fly.dev
TYPESENSE_SELFHOST_API_KEY=...
TYPESENSE_COLLECTION=youtube_transcript_chunks
YTSEARCH_API_ADDR=127.0.0.1:8787
RUST_LOG=youtube_api=info,youtube_cli=info,youtube_sources=info,youtube_ytdlp=info,youtube_transcribe=info,tower_http=info
LOG_FORMAT=json
VITE_API_BASE_URL=https://api.example.com
YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE=120
YT_DLP_COOKIES_FILE=/run/secrets/youtube-cookies.txt
YT_DLP_EXTRACTOR_ARGS=youtube:player_client=default,mweb
```

## Install dependencies

```bash
bun install
```

Install local git hooks to block TypeScript/build errors before commits and deploy the frontend to Cloudflare Pages after each commit:

```bash
bun run hooks:install
```

The hooks and deploy scripts read `.env` automatically. For Cloudflare Pages deploys, set:

```env
CLOUDFLARE_API_TOKEN=...
CLOUDFLARE_PAGES_PROJECT=ytsearch
CLOUDFLARE_PAGES_BRANCH=main
VITE_API_BASE_URL=https://ytsearch-api-typesense-fly-staging.fly.dev
VITE_INDEX_QUEUE_API_BASE_URL=https://ytsearch-api-typesense-fly-staging.fly.dev
```

Set `YTSEARCH_SKIP_DEPLOY_ON_COMMIT=1` to skip a post-commit deploy.

System tools used by optional ingestion paths:

```bash
yt-dlp --version
ffmpeg -version
ffprobe -version
voxtype --help
```

`voxtype` is only needed if you use `--transcribe-missing`.

## Run the app

Terminal 1, Rust API in release mode:

```bash
bun run api
```

Terminal 2, frontend dev server:

```bash
bun run dev
```

Open:

```text
http://localhost:3000
```

The Vite dev server proxies `/api` to `http://127.0.0.1:8787`.

## Index a creator

Website-triggered indexing is intentionally unavailable. When a handle starting with `@` is submitted in the UI, it shows a short request message instead of starting a public job.

Queue/index creators locally with the CLI so credentials, cookies, retries, and quota stay operator-controlled.

Validate a handle:

```bash
cargo run -p youtube_cli -- validate @t3dotgg
```

Index captions into Meilisearch. The default operational mode is `update`,
which keeps existing chunks and only adds videos that are not already indexed:

```bash
cargo run -p youtube_cli -- index-youtuber @t3dotgg --backend meilisearch --mode update
```

Use `rewipe` only when you intentionally accept deleting a creator's existing
chunks before replacement captions are confirmed. It is fail-closed and requires
an explicit acknowledgement:

```bash
ALLOW_DESTRUCTIVE_REWIPE=1 cargo run -p youtube_cli -- \
  index-youtuber @t3dotgg --backend meilisearch --mode rewipe --allow-destructive-rewipe
```

Thin wrapper scripts are also available:

```bash
scripts/index-youtuber-meili-update.sh @t3dotgg --jobs 8
ALLOW_DESTRUCTIVE_REWIPE=1 scripts/index-youtuber-meili-rewipe.sh @t3dotgg --jobs 8
scripts/index-youtuber-typesense-update.sh @t3dotgg --jobs 8
ALLOW_DESTRUCTIVE_REWIPE=1 scripts/index-youtuber-typesense-rewipe.sh @t3dotgg --jobs 8
```

Typesense uses the same modes with `--backend typesense`.

To index every handle from a plain text list, one creator per line:

```bash
cargo run -p youtube_cli -- index-youtubers scripts/indexed-channels.txt --backend meilisearch --mode update --jobs 8
```

To export the current indexed creator list from staging:

```bash
scripts/export-indexed-channels-list.sh indexed-channels.txt
```

By default, the raw CLI reindexing command deletes existing transcript chunks for that YouTuber before importing the new version. For a non-destructive smoke test:

```bash
cargo run -p youtube_cli -- index-youtuber @t3dotgg --mode update --max-videos 1
```

Use local audio transcription only when public captions are missing:

```bash
cargo run -p youtube_cli -- index-youtuber @t3dotgg --transcribe-missing
```

Optional `voxtype` engine override:

```bash
cargo run -p youtube_cli -- index-youtuber @t3dotgg \
  --transcribe-missing \
  --voxtype-engine whisper
```

Remove a creator locally from Meilisearch:

```bash
cargo run -p youtube_cli -- remove-youtuber @oldhandle
```

## Search from CLI

```bash
cargo run -p youtube_cli -- search resident --youtuber @t3dotgg --per-page 5
```

## Search from HTTP API

```bash
curl 'http://127.0.0.1:8787/api/search?q=resident&youtuber=@t3dotgg&per_page=5'
```

The API uses the Meilisearch search key. The master/admin key is only used by indexing commands.
Set `YTSEARCH_SEARCH_BACKEND=typesense` to use the deprecated Typesense backend during rollback.

## Benchmark search backends

Compare the Meilisearch staging API with the legacy Typesense API:

```bash
QUERY=fable YOUTUBER=@t3dotgg RUNS=20 scripts/benchmark-search-backends.sh
```

Run the maintained relevance/latency query suite:

```bash
RUNS=10 scripts/benchmark-search-cases.sh
```

To run the same benchmark from multiple Fly regions, provide region codes and a
Fly app that can run `curl` over SSH:

```bash
FLY_REGIONS="sjc iad lhr nrt syd" \
BENCH_FLY_APP=ytsearch-api-meili-staging \
QUERY=fable \
YOUTUBER=@t3dotgg \
RUNS=20 \
scripts/benchmark-search-backends.sh
```

If you want independent public probes instead of Fly SSH, use Globalping:

```bash
QUERY=fable YOUTUBER=@t3dotgg scripts/benchmark-search-globalping.sh
```

## Deploy frontend variants

Each Cloudflare Pages preview is built with a different `VITE_API_BASE_URL`:

```bash
scripts/cloudflare-pages-deploy-backends.sh
```

Default preview branches:

- `meili` -> `https://ytsearch-api-meili-staging.fly.dev`
- `typesense` -> `https://ytsearch-api-typesense-fly-staging.fly.dev`

See `docs/indexing-operations-guide.md` for the queue workflow, active backend update command, and count-repair commands.

## Notes From Today

The main performance issue was Meilisearch doing extra work to dedupe video hits in the API. The fix was to use backend-side distincting on `video_id` and fetch only the fields the API needs.

Cloudflare Pages deployment works against the `ytsearch` project, which serves `ytsearch-755.pages.dev`. The staging preview branch I used is `meilisearch-fly-staging`.

Wrangler works in this repo through `bunx wrangler`, but in non-interactive sessions it still needs Cloudflare auth unless an existing local login is present.

## Frontend behavior

The Solid frontend:

- Lists indexed creators from `/api/youtubers` so the homepage shows what is searchable.
- Uses the browser History API (`pushState`, `replaceState`, and `popstate`) so channel URLs and the back button behave like normal navigation.
- Redirects unknown creator URLs back to `/` after indexed creators load.
- Switches to index mode when the query starts with `@`, then explains website indexing is unavailable and links to tag `@notpronsh` on X/Twitter.
- Groups search results by video to avoid duplicate rows from nearby transcript chunks.
- Shows a small set of matching snippets per video; clicking a snippet moves the embedded player to that timestamp.
- Supports relevance/newest/oldest sorting with Meilisearch typo tolerance left enabled.
- Uses `youtube-nocookie.com` embeds and opens timestamped YouTube links in a new tab.

## Architecture and deployment

- See `docs/architecture.md` for crate responsibilities and hotswappable boundaries.
- See `docs/deploy.md` for Cloudflare/Fly/Railway deployment options, indexing commands, and observability recommendations.

## Logging

Rust services use `tracing` / `tracing-subscriber`. Set `LOG_FORMAT=json` in production to emit structured stdout logs that Fly/Railway log drains can ship to Axiom. The API also adds simple in-memory rate limits and cache headers: `/api/youtubers` and `/api/search` are briefly cacheable, while `/api/health` is `no-store`.

Examples:

```bash
RUST_LOG=info bun run api
LOG_FORMAT=json RUST_LOG=youtube_api=info,tower_http=info bun run api
RUST_LOG=youtube_cli=debug,youtube_sources=debug,youtube_transcribe=debug cargo run -p youtube_cli -- index-youtuber @t3dotgg --max-videos 1 --no-wipe
```

## Compliance and policy considerations

This project touches YouTube content and YouTube API data. Important policy points to understand:

- YouTube's Terms of Service allow viewing content and using embeddable YouTube players, subject to their terms.
- YouTube's Terms restrict downloading, reproducing, distributing, or otherwise using YouTube content except as expressly authorized by the service or with permission.
- YouTube API Services Developer Policies require API clients to be transparent, link to YouTube Terms, provide a privacy policy, protect user/API data, and follow data retention requirements.
- YouTube API Developer Policies state that API clients must not download, import, backup, cache, or store copies of YouTube audiovisual content without YouTube's prior written approval.
- YouTube API Developer Policies state that API clients must not scrape YouTube Applications or obtain scraped YouTube data/content except where permitted, such as public search engines following `robots.txt` or with prior written permission.
- The official YouTube Captions API can download caption tracks only when the authenticated user has permission to edit the video; it is not a general public transcript API.
- Embedded YouTube players may cause data to be collected/shared with YouTube/Google. Keep autoplay disabled unless you have a clear privacy basis.

Because of this, the audio fallback should be treated as a tool for personal, authorized, research, or otherwise legally permitted use. Do not use this project to infringe copyrights, bypass access controls, or violate platform terms.

See `docs/privacy-notice.md` for a project privacy/compliance template.

## Data retention

Generated local data is stored under `data/` and gitignored. This may include audio files and transcript caches when `--transcribe-missing` is used.

For production use, implement retention/deletion policies appropriate to your legal basis and YouTube API obligations. For YouTube API non-authorized data, the Developer Policies generally require deletion or refresh within 30 days.

## License

Code in this repository is licensed under the MIT License. See `LICENSE`.

The MIT License applies to this project's source code only. It does not grant rights to YouTube videos, audio, captions, transcripts, thumbnails, creator names, logos, or other third-party content/data indexed or displayed by the application.
