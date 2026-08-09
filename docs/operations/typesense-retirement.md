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

## Current execution blocker

The repository migration and local full-corpus proof are complete, but this
shell has no Fly access token (`flyctl auth whoami` returns “no access token
available”). No Fly metadata, deployment, queue copy, stop, or destructive
operation can be performed until `flyctl auth login` succeeds in this shell.

The deprecation script defaults to a no-op and requires an exact acknowledgement
because its metadata update may restart a Machine. Destruction remains a manual,
separately reviewed operation after all gates above are recorded.
