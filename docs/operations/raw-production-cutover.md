# Raw production cutover

Current allocation costs, load gates, and the SQLite control-plane decision are
recorded in [Raw production cost and persistence decision](raw-production-cost.md).

## Current topology

Search reads from <https://search.pronsh.dev> route through the Cloudflare
Worker to `ytsearch-api-raw-production`, then to `ytsearch-raw-production`.
The checked-in production configuration routes index queue and administrative
requests to Raw. The durable queue was copied before the Worker origin changed;
there is no Typesense dependency in the live read or write path.

| Component | Identifier | Allocation or version |
| --- | --- | --- |
| Raw engine | `ytsearch-raw-production` / `d89303da029328` | 2 GiB, 2 shared vCPUs; 512 MiB swap recovery margin |
| Raw API | `ytsearch-api-raw-production` / `8e7204a7694d28` | 512 MiB, 1 shared vCPU |
| Raw indexer | `ytsearch-indexer-raw-production` / `8e426df7e305e8` | 2 GiB, 2 shared vCPUs; stopped/on demand, five-minute idle exit |
| Production Worker | `ytsearch-search` | Cloudflare deployment; exact version recorded by each deployment log |
| Deprecated Typesense engine | `ytsearch-typesense-staging` / `8147deb94937e8` | Stopped and preserved with benchmark Machine `d8de466f330468` |
| Deprecated Typesense API | `ytsearch-api-typesense-fly-staging` / `1855eddf654298` | Stopped and preserved |
| Deprecated Typesense indexer | `ytsearch-indexer-staging` / `68352d2be17728` | Stopped and preserved |

Image digests and Worker version IDs are deployment outputs, not stable topology.
Resolve them from Fly and Cloudflare immediately before rollback instead of
copying an old value from this document.

## Promotion gates

- Typesense documents: 8,142,650; Raw documents: 8,142,650.
- Typesense distinct video IDs: 32,822; Raw distinct video IDs: 32,822.
- Sorted video-ID set difference: zero.
- Creator directories: 61 on both API facades with matching channel counts.
- Full Raw contract: authentication, aliases, channel scope, typo tolerance,
  pagination, date sorting, and engagement sorting all passed.
- Shared-browser check: no response error; Raw totals, first result, and all
  seven sort controls rendered correctly.
- Public load: 75 requests, five-way concurrency, zero errors, 366 ms p95.
- Post-load Raw RSS: 373,096,448 bytes; engine-reported search errors: zero.
- Initial-route JavaScript: 24,342 bytes gzip, below the 30 KB limit.

The current pre-rollout production baseline on 2026-08-13 is generation 2144
with 8,322,725 documents on the 2 GiB shared Raw Machine. Earlier generation
1166/1167 repair results remain historical evidence in the experiment reports,
not a statement of current production state.

The last verified browser build remains below the 30 KB initial-JavaScript
ceiling. Treat old 308 ms/75-request and generation-1167 results as historical
gates; each rollout must record fresh latency, ordering, highlight, cache, and
browser evidence. The four legacy Machines are stopped and deprecated. Their
seven volumes total 61 GB ($9.15/month); they remain preserved unless the user
makes a new explicit destruction decision. Current complete pricing is in
[Raw production cost and persistence decision](raw-production-cost.md).

## Rollback

Rollback is intentionally reversible because the deprecated resources remain
stopped and preserved:

1. Set `API_BASE_URL` in `wrangler.jsonc` back to
   `https://ytsearch-api-typesense-fly-staging.fly.dev`.
2. Restore the Typesense scheduler machine variables in `wrangler.jsonc`.
3. Run `bun run deploy:frontend` and execute
   `scripts/check-frontend-backends.sh` with the temporary expected search API.
4. Verify `blackhole`, `blackhole/@veritasium`, and the Cursor regression URL in
   a clean browser before declaring rollback complete.

A configuration redeploy is preferred to a stale Worker binary because it keeps
current frontend and cache-correctness fixes.

For Raw reader binary rollback, stop the on-demand indexer first, roll the
reader back, and start only a worker whose publication protocol matches the
reader. Protocol v1 also fails closed per mutation, so a race or incorrect
ordering cannot append writes that the active reader cannot publish.

## Mutable write path

Raw now stores idempotent video/channel mutations in SQLite WAL, publishes a
verified full generation through an atomic manifest, retains one rollback
generation, and swaps readers without stopping the public server. Control-plane
counts include pending durable mutations, and publication resumes after a
restart. A full 8,142,650-document mutation/publish/restart experiment passed;
see [the full-corpus report](../experiments/search/raw-mutable-full-corpus-2026-08-09.md).

SQLite is only the durable mutation/control plane. Public queries continue to
read the immutable mmap inverted index and perform no SQL query or SQLite lock.
The exact retirement state is in [Typesense retirement](typesense-retirement.md).
