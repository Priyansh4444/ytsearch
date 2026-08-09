# Raw production cutover

## Current topology

Search reads from <https://search.pronsh.dev> route through the Cloudflare
Worker to `ytsearch-api-raw-production`, then to `ytsearch-raw-production`.
The checked-in production configuration routes index queue and administrative
requests to Raw. The deployed Worker still uses the legacy queue until the
durable queue volume is created and copied; do not deploy the configuration
before that copy is complete.

| Component | Identifier | Allocation or version |
| --- | --- | --- |
| Raw engine | `ytsearch-raw-production` / `7813351a005348` | 4 GiB, 2 performance vCPUs |
| Raw API | `ytsearch-api-raw-production` / `83977eb7d6d748` | 512 MiB, 1 shared vCPU |
| Production Worker | `ytsearch-search` | `ad6252cb-3999-44ef-bdc6-3f3e88129e72` |
| Pre-cutover Worker rollback | `ytsearch-search` | `1b873022-c9df-48c0-a4df-dda0337344a4` |
| Retained Typesense engine | `ytsearch-typesense-staging` / `8147deb94937e8` | 8 GiB, 4 performance vCPUs |
| Retained Typesense API | `ytsearch-api-typesense-fly-staging` / `1855eddf654298` | 512 MiB, 1 shared vCPU |

The production Raw engine reuses the exact validated experimental image
`registry.fly.io/ytsearch-raw-experiment:deployment-01KZJ3SBKM16XSM3CW9EXY6ATC`.
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

The remaining limitation is operational rather than architectural: the new
engine/API volumes, indexer Machine, queue copy, and Worker configuration have
not been deployed because Fly authentication is unavailable in this shell. The
exact fail-closed order and resource ledger are in
[Typesense retirement](typesense-retirement.md).
