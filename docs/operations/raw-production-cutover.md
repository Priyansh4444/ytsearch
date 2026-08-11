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
| Raw indexer | `ytsearch-indexer-raw-production` / `8e426df7e305e8` | 4 GiB, 4 shared vCPUs; stops after five idle minutes |
| Production Worker | `ytsearch-search` | `5e4f60d7-8036-4ecb-9329-233a09925354` |
| Pre-cutover Worker rollback | `ytsearch-search` | `1b873022-c9df-48c0-a4df-dda0337344a4` |
| Deprecated Typesense engine | `ytsearch-typesense-staging` / `8147deb94937e8` | Stopped; pending exact-resource destruction after the active full repair gate |
| Deprecated Typesense API | `ytsearch-api-typesense-fly-staging` / `1855eddf654298` | Stopped; pending exact-resource destruction after the active full repair gate |

The production Raw engine runs the bounded publisher image
`registry.fly.io/ytsearch-raw-production:raw-bounded-2ddbb5b` at digest
`sha256:f6b81ddbf6625d6ca108ae7117ee1ee0658f26011b36c4ac758bdd883434315c`.
The API facade reuses the contract-compatible image
`registry.fly.io/ytsearch-api-raw-experiment:deployment-01KZ12A540RH0Q39WTW4XPDM0A`.

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

After the complete mutable migration, generation 1167 contains 8,198,020
documents, 33,823 videos, 61 channels, and 294,414 terms. The nine-channel
drift repair published generation 1166; the one write interrupted by an earlier
deployment (`g20JZ2HNZaw`) was replayed as 66 chunks and published as generation
1167. A clean Machine restart re-opened that generation, and the video is
publicly searchable.

The final public contract returned 200 for relevance, combined engagement,
views, likes, comments, and both publication-date sorts; an unknown sort
returned 400. The true final `blackhole` page returned five videos and
`has_next_page=false`. A 75-request, five-concurrent run had zero errors and a
308 ms p95. The redeployed initial route is 24,343 bytes gzip.

T3's collaborative snapshot/evaluate channel remained unavailable after the
deployment and failed identically on `example.com`. Therefore the final visual
browser gate is explicitly not claimed. The legacy resources are marked
deprecated and all four Machines are stopped. Autostart is disabled on the two
former services. Their volumes are retained until that one gate can be
observed; this rollback storage costs about $4.65/month.

## Rollback

1. Set `API_BASE_URL` in `wrangler.jsonc` back to
   `https://ytsearch-api-typesense-fly-staging.fly.dev`.
2. Restore the Typesense scheduler machine variables in `wrangler.jsonc`.
3. Run `bun run deploy:frontend` and execute
   `scripts/check-frontend-backends.sh` with the temporary expected search API.
4. Verify `blackhole`, `blackhole/@veritasium`, and the Cursor regression URL in
   a clean browser before declaring rollback complete.

The pre-cutover Worker version is retained above as forensic evidence, but a
configuration redeploy is preferred because it keeps current frontend fixes.

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
