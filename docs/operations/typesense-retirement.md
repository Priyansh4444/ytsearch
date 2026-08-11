# Typesense retirement ledger

Typesense is no longer a source-code or production-target dependency. Its old
comparison scripts and experiment reports remain only as reproducible evidence
for the Raw relevance and resource claims.

## Known Fly resources

| State | App | Machine | Attached volume | Replacement |
| --- | --- | --- | --- | --- |
| Deprecated; retained for browser gate | `ytsearch-typesense-staging` | `8147deb94937e8` | `vol_v8e6yy17jo63nplv` (10 GB) | `ytsearch-raw-production` |
| Deprecated; retained for browser gate | `ytsearch-typesense-staging` | `d8de466f330468` | `vol_vdezd781q880qm34` (10 GB) | Historical benchmark only |
| Deprecated; retained for browser gate | `ytsearch-api-typesense-fly-staging` | `1855eddf654298` | `vol_vwnydd578odzzo8v` (1 GB) | `ytsearch-api-raw-production` |
| Deprecated; retained for browser gate | `ytsearch-indexer-staging` | `68352d2be17728` | `vol_r68z3gl5wq7jkqd4` (10 GB) | `ytsearch-indexer-raw-production` |

The following unattached volumes are historical and are intentionally outside
the automatic retirement scope: `vol_v8e6wk3g1l6kj15v`,
`vol_458zq611m0yppz14`, and `vol_4qlzgynglnz786gr`. Audit their contents and
ownership separately before deleting them.

## Required order

1. Deploy `ytsearch-raw-production` with its 20 GB mutable volume and matching
   public/admin keys.
2. Prove a replacement mutation is durable, publish it, restart the engine,
   and verify the operation remains visible and idempotent.
3. Create the Raw API's 1 GB queue volume. Copy every pending request from the
   legacy API, including `1786191981-typesense-fly-t3dotgg`, before changing the
   Worker queue origin.
4. Deploy `ytsearch-indexer-raw-production`, process the copied request, and
   prove the resulting channel/video data survives a Raw restart.
5. Deploy the Worker with both API origins set to
   `ytsearch-api-raw-production`. Run browser, search-contract, concurrent-load,
   queue, and 30 KB browser-JavaScript gates.
6. Stop the legacy indexer, API, and engine. Mark their Machines deprecated
   with `scripts/deprecate-typesense-fly.sh`.
7. Observe Raw for a bounded rollback window, then destroy the four known
   Machines, their four attached volumes, and the three legacy apps. Re-run
   `fly apps list`, `fly machines list`, and `fly volumes list` to prove no
   active Typesense resource remains.

Never delete the legacy queue before its request IDs have been copied and
acknowledged by the Raw queue. A successful search-read cutover alone is not a
write-path migration.

## Current execution status (2026-08-10)

Fly authentication is restored. The mutable Raw engine, durable Raw queue API,
Raw indexer, and Raw-only Worker are deployed. All five legacy queue records
were copied with their original IDs. Request
`1786191981-typesense-fly-t3dotgg` completed through Raw, importing 95 videos
and 30,684 chunks, and was acknowledged only after generation 102 activated.
The four remaining copied records preserve their historical failed/rejected
states.

The live restart proof also completed: one synthetic replacement reused the
same operation sequence for an identical idempotency key, survived publication
and restart, and its idempotent deletion survived a second publication and
restart. No synthetic document remains. `search.pronsh.dev` now sends both
search and queue traffic exclusively to `ytsearch-api-raw-production`.

The nine-channel reconciliation published generation 1166. The interrupted
Veritasium video was replayed, generation 1167 published 8,198,020 documents and
33,823 videos, and a clean restart reopened it. Search, queue, bundle, fail-closed
sort validation, last-page, and 75-request concurrent-load gates passed. The
Raw indexer was restored to a five-minute idle shutdown and is stopped.

All four exact legacy Machines now carry `lifecycle=deprecated`,
`deprecated_reason=raw-search-cutover`, and their Raw successor metadata. No
Machine, volume, or app has been destroyed. The two services that were still
running (`8147deb94937e8` and `1855eddf654298`) are now stopped with autostart
disabled; all four legacy Machines are stopped. Production frontend, search,
creator-directory, and queue checks remained healthy after the stop, proving
that no live request depends on Typesense. The retained four attached volumes
total 31 GB and cost about $4.65/month at current Fly list pricing.

The sole remaining irreversible
gate is visual browser automation: T3 snapshot/evaluate/wait timed out on the
production page and failed identically on `example.com`, demonstrating a
tool-wide automation failure. Destruction remains intentionally blocked until
that result can be observed; HTTP or unit-test evidence is not recorded as a
visual pass.
