# Typesense retirement ledger

Typesense is no longer on the live read or write path. It remains a deprecated,
stopped rollback and differential-testing artifact. Its source is retained on
the `typesense-fly-selfhost-comparison` archive branch; its comparison scripts,
compatibility notes, and experiment reports remain reproducible evidence for
Raw relevance and resource claims. Deprecation alone does not authorize
deletion. The active Raw-only migration authorizes the exact retirement set
only after the final creator, publication, recovery, and inventory gates pass.

The table below is a historical ledger. The fail-closed executable inventory is
[`config/fly-production-resources.json`](../../config/fly-production-resources.json);
deprecation and preservation scripts derive their exact sets from that file.

## Inventory drift observed 2026-09-22

The read-only exact-set verifier failed on the legacy API volume: it expected
`vol_vwnydd578odzzo8v`, but Fly reports `vol_vgnmemxz8gkmoe14`, attached to
the same stopped deprecated Machine `1855eddf654298`. The stopped cold-benchmark
Machine `d8de466f330468` also has a new 10 GB attached volume
`vol_42k57jld6k8exy74` in place of `vol_vdezd781q880qm34`. The legacy indexer
app now lists two different unattached 10 GB volumes,
`vol_v3g1oqd9pgz603w4` and `vol_40ox6zgje9pd08k4`, instead of the two
historical IDs in the table below. The other attached legacy IDs remain as
listed. All four deprecated Machines were stopped during this observation.

The current legacy apps still provision 61 GB in total, or $9.15 per month at
Fly's $0.15/GB-month list rate, before snapshots and stopped root filesystems.
The replacement volume contents and lineage have not been verified; the Fly
volume metadata does not provide a source snapshot ID. Keep the configured
exact-set verifier failing until a read-only content audit and queue-state
comparison establish which volumes preserve the original data. Do not expand
the deletion allowlist from a matching size or Machine attachment alone.

## Known Fly resources

| State | App | Machine | Attached volume | Replacement |
| --- | --- | --- | --- | --- |
| Deprecated, stopped, preserved | `ytsearch-typesense-staging` | `8147deb94937e8` | `vol_v8e6yy17jo63nplv` (10 GB) | `ytsearch-raw-production` |
| Deprecated, stopped, preserved | `ytsearch-typesense-staging` | `d8de466f330468` | `vol_vdezd781q880qm34` (10 GB) | Historical benchmark only |
| Deprecated, stopped, preserved | `ytsearch-api-typesense-fly-staging` | `1855eddf654298` | `vol_vwnydd578odzzo8v` (1 GB) | `ytsearch-api-raw-production` |
| Deprecated, stopped, preserved | `ytsearch-indexer-staging` | `68352d2be17728` | `vol_r68z3gl5wq7jkqd4` (10 GB) | `ytsearch-indexer-raw-production` |

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

The Raw cache now holds 8,829 transcript files. The preservation verifier checks
the complete seven-volume inventory and exact stopped-Machine metadata; an
unexpected Machine or volume makes it fail closed without changing anything.

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
7. Keep the four known Machines stopped and deprecated and retain all seven
   verified legacy volumes as the rollback/oracle snapshot until every final
   gate passes. Re-run `fly apps list`, `fly machines list`, and `fly volumes
   list` during cost reviews to prove that nothing has restarted.
8. After the final gates pass, verify the inventory again and run only a
   separately reviewed cleanup that names this exact set. Confirm that every
   Machine, volume, and app is absent before declaring retirement complete.

The historical cleanup implementation remains on the Typesense archive branch.
On master, `scripts/destroy-deprecated-typesense-fly.sh` is verification-only:
it resolves every exact Machine and volume and detects inventory drift but has
no destructive mode. The final migration still requires a separately reviewed
implementation that deletes only the verified inventory.

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
that no live request depends on Typesense. The retained four attached and three
historical volumes total 61 GB and cost $9.15/month at current Fly list pricing,
excluding snapshots and stopped Machine root filesystems.

The shared T3 browser now passes the production visual gate: the page reports
24/7 availability, renders twelve result cards without an alert, supports every
active sort, and returns 21 creator-scoped `blackhole` videos for Veritasium.
The exact retained set also passes its live verification-only gate. It remains
deprecated and stopped until all final gates pass. A passing gate authorizes no
resource outside the separately reviewed exact retirement set.
