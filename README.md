# ytsearch

YouTube transcript indexing and timestamped search built with Rust, Typesense,
`yt-dlp`, and SolidJS.

The active stack uses self-hosted Typesense only. Meilisearch was retired; the
historical removal and migration context is preserved in
[`docs/legacy/meilisearch-retirement.md`](docs/legacy/meilisearch-retirement.md).
Use that retirement commit or a legacy fork if you need to build an adapter for
old Meilisearch data.

## Quick start

The provider-neutral deployment runs Typesense, the Rust API, one persistent
queue worker, and the static frontend:

```bash
export TYPESENSE_SELFHOST_API_KEY="$(openssl rand -hex 24)"
export YTSEARCH_INDEX_QUEUE_TOKEN="$(openssl rand -hex 24)"
export PUBLIC_API_BASE_URL=http://localhost:8787
docker compose up -d --build
curl -fsS http://localhost:8787/api/youtubers
```

Open `http://localhost:3000`. See [the self-hosting guide](docs/self-hosting.md)
for public deployment, cookies, persistence, and bare-process operation.

## Local development

```bash
bun install
cp .env.example .env
bun run api
```

In a second terminal:

```bash
bun run dev
```

The frontend runs at `http://localhost:3000` and proxies `/api` to the Rust API
at `http://127.0.0.1:8787`.

Minimum backend environment:

```env
YTSEARCH_SEARCH_BACKEND=typesense
TYPESENSE_API_URL=http://127.0.0.1:8108
TYPESENSE_SEARCH_API_KEY=replace-me
TYPESENSE_ADMIN_API_KEY=replace-me
TYPESENSE_COLLECTION=youtube_transcript_chunks
YTSEARCH_INDEX_QUEUE_TOKEN=replace-me
```

`yt-dlp` is the primary discovery and caption source. `YOUTUBE_API_KEY` is an
optional metadata/discovery fallback; it cannot retrieve captions. Set
`YT_DLP_COOKIES_FILE` when an authorized browser session is required.

## Indexing

Frontend creator requests wait for operator approval before an indexer starts:

```bash
scripts/review-index-requests.sh
```

See the operations guide for non-interactive approve/reject commands.

Validate and update a creator without deleting existing chunks:

```bash
cargo run --release -p youtube_cli -- validate @handle
cargo run --release -p youtube_cli -- \
  index-youtuber @handle --backend typesense --mode update --jobs 1
```

Refresh counts from existing backend chunks:

```bash
cargo run --release -p youtube_cli -- \
  refresh-youtuber-counts @handle --backend typesense
```

Destructive rewipe is fail-closed and requires explicit acknowledgement:

```bash
ALLOW_DESTRUCTIVE_REWIPE=1 cargo run --release -p youtube_cli -- \
  index-youtuber @handle --backend typesense --mode rewipe \
  --allow-destructive-rewipe --jobs 1
```

Official event-only playlists are recorded in
[`scripts/conference-playlists.tsv`](scripts/conference-playlists.tsv) and can
be imported resumably with `scripts/index-conference-playlists.sh`.

## Verification

```bash
cargo fmt --check
bun run test                 # Cargo workspace + frontend component tests
bun run test:frontend:watch  # focused frontend development
bun run test:frontend:coverage
bun run frontend:check
bun run check:responsive       # mocked browser flows at phone/tablet/desktop widths
scripts/check-rust-dependency-policy.sh # reject duplicate native TLS/OpenSSL clients
sh -n scripts/cloud-index-queue-entrypoint.sh scripts/fly-run-queue-indexer.sh
```

Agent-oriented production diagnostics are intentionally bounded:

```bash
scripts/doctor.sh                       # health + one next action on failure
scripts/agent-status.sh                 # one compact JSON line
scripts/summarize-indexer-logs.sh       # latest progress + up to 3 errors
scripts/check-staging-machines.sh       # one invariant summary line
FORMAT=pretty scripts/agent-status.sh   # human-readable when needed
VERBOSE=1 scripts/check-typesense-fly.sh
```

Reproducible optimization experiments use the bounded protocol in
[`autoresearch/program.md`](autoresearch/program.md). Establish a correctness,
serialization, and live-search baseline with:

```bash
RUNS=10 scripts/autoresearch-baseline.sh
```

The isolated Karpathy-style vector experiment can be initialized with
`scripts/autoresearch-init.sh <tag>`; its fixed evaluator is described in
[`autoresearch/program.md`](autoresearch/program.md).

Measured baselines and the isolated infrastructure experiment queue are in
[`docs/autoresearch.md`](docs/autoresearch.md).
The research-backed retrieval and performance review, including current
measurements and the promotion roadmap, is in
[`docs/performance-research.md`](docs/performance-research.md).
Vector-store alternatives, the local Qdrant recall/latency benchmark, and the
video-frame embedding research are in
[`docs/vector-search-and-multimodal-research-2026-07-17.md`](docs/vector-search-and-multimodal-research-2026-07-17.md).
Cold-start, warm concurrent, highly available, and semantic-service cost models
are in
[`docs/cold-warm-concurrency-costs-2026-07-17.md`](docs/cold-warm-concurrency-costs-2026-07-17.md).

Operational details are in
[`docs/indexing-operations-guide.md`](docs/indexing-operations-guide.md),
[`docs/staging-architecture.md`](docs/staging-architecture.md), and
[`docs/staging-observability.md`](docs/staging-observability.md).

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) for the local
workflow and safety requirements. Report vulnerabilities privately according to
[SECURITY.md](SECURITY.md). This project is available under the [MIT License](LICENSE).
