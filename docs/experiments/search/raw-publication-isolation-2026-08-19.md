# Raw publication isolation, 2026-08-19

## Decision

Full generation construction is prohibited in the serving Raw reader. Search is
fast enough while idle, but the 8.3-million-document rebuild shares storage and
page cache with the active mmap and creates multi-second stalls. Production will
reuse the existing on-demand SJC indexer as a detached builder, stream nine
verified files to the reader, and perform only a bounded receive plus atomic
activation there.

This adds no permanent Machine or volume. Typesense remains stopped,
deprecated, and preserved as an oracle.

## Production evidence

The privacy-safe Axiom search report showed that the regression was backend
wait, not API formatting:

| Window | Requests | p50 | p95 | API result processing |
| --- | ---: | ---: | ---: | ---: |
| 24 hours | 114 | 121 ms | 1,178 ms | about 0.1 ms |
| 7 days | 413 | 68.6 ms | 8,118 ms | about 0 ms |

Successful `react` requests took 11-13 seconds entirely inside Raw. The deployed
engine was still evaluating the missing-separator fallback (`react` as `re` +
`act`, and `fire` as `fi` + `re`). Gating that fallback and requiring at least
three characters on both sides restored correct broad-query results.

The isolated 8.32-million-document reader then passed a clean idle gate:

| Samples | Errors | p50 | p95 | Max |
| ---: | ---: | ---: | ---: | ---: |
| 57 | 0 | 88.7 ms | 143.3 ms | 147.5 ms |

The same reader exceeded the five-second client timeout immediately after an
in-process publication began. That rejects same-reader construction regardless
of its average latency.

## Publication trace

The completed canary build took 1,333,102 ms (22.22 minutes), while its builder
thread consumed 216,897 ms of CPU. Instrumentation recorded:

| Work | Count/bytes | Time |
| --- | ---: | ---: |
| General durability/reclaim boundaries | 6,248 / 6.70 GB | 453,057 ms in `sync_data` |
| Posting durability/reclaim boundaries | 2,699 / 2.78 GB | 70,983 ms in `sync_data` |
| Total durability barriers | 8,947 | about 524 seconds |
| Reader cache advice | 12.95 GB | measured separately |
| Implicit document-position seeks | 130,045 | about 2.34 seconds |

Peak process RSS observed during this current bounded build was about 969 MB,
with no swap. The candidate workspace peaked near 3.98 GB. A retained older
2.47-GB RSS report describes the pre-bounded builder and is not accepted as
evidence for the current external builder; the current image must still pass a
2-GB cgroup gate before production.

The dominant failure is storage/writeback and page-cache interference in the
serving cgroup. Query scoring changes cannot repair that tail.

## Implemented isolation contract

- Reader startup no longer launches a full build.
- SQLite exports one immutable, checksummed snapshot through sequence N.
- The builder verifies an exact corpus identity and builds from the read-only
  snapshot using the shared `GenerationBuilder`.
- Nine fixed files stream sequentially with exact length and BLAKE3 checks.
- Reader upload uses a 256-KiB buffer and a bounded 64-MiB
  sync-and-`DONTNEED` window, preserving the active mmap.
- The reader reserves 1 GB after the declared upload and rejects insufficient
  disk before accepting its manifest.
- Activation performs structural validation and atomic rename/swap without a
  duplicate 1.45-GB checksum or tree-sync pass. Restart/recovery still performs
  full checksums.
- Queue requests write durable mutations with `--defer-publish`, share one
  publication per bounded batch, and are acknowledged only after the initially
  observed required sequence is active.
- The builder workspace is absolute, non-root, single-owner locked, restricted
  to exact job-name cleanup, and requires 8 GB free before construction.

## Remaining release gate

Run `scripts/benchmark-publication-isolation.sh`; it continuously drives the
canonical mixed-query matrix at concurrency four from before snapshot export
through activation and the warmed new generation. The gate records every
timeout and invalid body instead of aborting early.

Required before cutover:

- every measured request at or below 200 ms, with zero HTTP/JSON errors;
- correct `react`, `fire`, `one fire`, `coding is dead`, CSS phrase, GitHub,
  Prototype chain, `createEffect`, and scoped/global `blackhole` results;
- one atomic sequence change, with N visible and later mutations still durable;
- current builder peak RSS below 1.75 GiB and cgroup memory below 1.85 GiB;
- no OOM, swap dependence, or disk use above 80%;
- a 60-second post-swap warm phase with the same relevance/highlight contract.

The same-reader canary was stopped after the failed gate. It must not be left
running as a non-serving replica.
