# Typesense retirement ledger

Typesense is no longer a source-code or production-target dependency. Its old
comparison scripts and experiment reports remain only as reproducible evidence
for the Raw relevance and resource claims.

## Known Fly resources

| State | App | Machine | Attached volume | Replacement |
| --- | --- | --- | --- | --- |
| Deprecated; pending final repair gates | `ytsearch-typesense-staging` | `8147deb94937e8` | `vol_v8e6yy17jo63nplv` (10 GB) | `ytsearch-raw-production` |
| Deprecated; pending final repair gates | `ytsearch-typesense-staging` | `d8de466f330468` | `vol_vdezd781q880qm34` (10 GB) | Historical benchmark only |
| Deprecated; pending final repair gates | `ytsearch-api-typesense-fly-staging` | `1855eddf654298` | `vol_vwnydd578odzzo8v` (1 GB) | `ytsearch-api-raw-production` |
| Deprecated; pending final repair gates | `ytsearch-indexer-staging` | `68352d2be17728` | `vol_r68z3gl5wq7jkqd4` (10 GB) | `ytsearch-indexer-raw-production` |

The same indexer app owns three unattached historical volumes. They are now
inside the exact retirement scope because their complete contents were audited
and preserved:

- `vol_v8e6wk3g1l6kj15v` held 4,163 transcript JSON files. All were merged into
  the active Raw indexer cache without overwriting newer files; 4,068 were new.
- `vol_458zq611m0yppz14` held two transcript files, both duplicates of the first
  volume.
- `vol_4qlzgynglnz786gr` held 3,469 transcript files. All 3,469 are now present
  in the Raw cache: 2,779 absent from the active index were imported first, then
  a cache-to-cache audit imported the remaining 468 missing files.

The Raw cache now holds 8,829 transcript files. The retirement script verifies
the complete seven-volume inventory before its first deletion; an unexpected
Machine or volume makes it fail closed.

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
   Machines, all seven verified legacy volumes, and the three legacy apps. Re-run
   `fly apps list`, `fly machines list`, and `fly volumes list` to prove no
   active Typesense resource remains.

The final destructive step is encoded in
`scripts/destroy-deprecated-typesense-fly.sh`. It resolves every exact Machine,
volume attachment, stopped state, deprecation marker, successor, complete app
Machine set, and complete app volume set before deleting the first resource.
Its verification-only mode has passed against the live Fly inventory.

Never delete the legacy queue before its request IDs have been copied and
acknowledged by the Raw queue. A successful search-read cutover alone is not a
write-path migration.

## Current execution status (2026-08-11)

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

The shared T3 browser now passes the production visual gate: the page reports
24/7 availability, renders twelve result cards without an alert, supports every
active sort, and returns 21 creator-scoped `blackhole` videos for Veritasium.
The exact destruction set also passes its live verification-only gate. Actual
destruction remains blocked only on the active 61-creator classified repair,
its one-publication 2 GB memory gate, and the post-publication restart/search/
load/browser checks.
