# Typesense retirement ledger

Typesense is no longer a source-code or production-target dependency. Its old
comparison scripts and experiment reports remain only as reproducible evidence
for the Raw relevance and resource claims.

## Known Fly resources

| State | App | Machine | Attached volume | Replacement |
| --- | --- | --- | --- | --- |
| Retire after cutover | `ytsearch-typesense-staging` | `8147deb94937e8` | `vol_v8e6yy17jo63nplv` (10 GB) | `ytsearch-raw-production` |
| Retire after cutover | `ytsearch-typesense-staging` | `d8de466f330468` | `vol_vdezd781q880qm34` (10 GB) | Historical benchmark only |
| Retire after queue drain | `ytsearch-api-typesense-fly-staging` | `1855eddf654298` | `vol_vwnydd578odzzo8v` (1 GB) | `ytsearch-api-raw-production` |
| Retire after queue drain | `ytsearch-indexer-staging` | `68352d2be17728` | `vol_r68z3gl5wq7jkqd4` (10 GB) | `ytsearch-indexer-raw-production` |

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

A bounded nine-channel drift reconciliation is running before the irreversible
stage. After it publishes, the remaining gates are a Raw restart, differential
corpus checks, browser/search/load checks, and the explicit retirement steps 6
and 7 above. The deprecation script remains a no-op unless its exact
acknowledgement is supplied.
