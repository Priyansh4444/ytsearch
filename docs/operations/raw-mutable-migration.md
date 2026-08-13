# Mutable Raw migration

## Contract

Workload: retain the immutable 8,142,650-document / 32,822-video schema-v6
generation while the existing indexer replaces complete videos, deletes stale
videos, updates creator profiles, and repairs channel gaps.

Invariants:

- Acknowledged durable writes survive process and Machine restarts.
- A complete video replacement is atomic; readers see either the old video or
  the complete replacement, never a partial transcript.
- Search continues on the last verified generation while a candidate builds.
- A candidate is activated only after checksums, counts, and `RawSearchEngine::open`
  succeed.
- Idempotency keys make indexer retries safe.
- Typesense stays stopped and deprecated after queue state, active writes,
  search reads, scheduler ownership, and recovery move to Raw. Its source,
  documentation, Machines, and volumes are preserved unless a later explicit
  decision authorizes exact destruction targets.
- The initial browser route remains below 30 KB gzip.

Primary gate: an indexed test video and channel update become searchable after
one publish, remain searchable after a Raw Machine restart, and disappear after
an atomic delete and second restart.

Secondary gates: publication peak RSS below the 4 GiB Machine allocation,
steady-state RSS below 512 MiB, zero search errors during publication, bounded
disk growth, and no regression beyond the established production latency gate.

Rollback: keep the current verified generation open until the candidate passes;
on any build, integrity, memory, or freshness failure, retain that generation
and leave the durable mutation log pending for retry. Keep Typesense resources
stopped and recoverable after the Raw restart gate as a rollback and relevance
oracle; migration completion does not require an irreversible stage.

## Baseline on 2026-08-09

| Measurement | Value |
| --- | ---: |
| Raw process RSS | 373,555,200 bytes (356.25 MiB) |
| Raw index artifacts | 1,444,907,196 bytes |
| Raw engine allocation | 4 GiB, 2 performance vCPUs, `sjc` |
| Raw API allocation | 512 MiB, 1 shared vCPU, `sjc` |
| Raw engine searches/errors | 86 / 0 |
| Internal engine search time | 2.044 seconds |

Current [Fly list pricing](https://fly.io/docs/about/pricing/) is $0.0894/hour
for the 4 GiB `performance-2x` Raw engine, $3.32/month for the 512 MiB
`shared-cpu-1x` API, $0.0329/hour while the 4 GiB `shared-cpu-4x` indexer runs,
and $0.15/GB-month for volumes. At the enforced 12-hour daily search window,
engine compute is about $32.20 per 30 days. Engine/API/indexer volumes total 31
GB ($4.65/month), so the scheduled engine, always-on API, and volumes are about
$40.17/month before variable indexer runtime, stopped rootfs, snapshots,
bandwidth, and Cloudflare usage. An always-on engine plus API and the same
volumes is about $72.36/month before those variable charges.

## Design

The mmap schema-v6 generation remains immutable. Mutability is implemented as
a durable control plane and atomic generation publication, not by placing locks
inside the query hot path. SQLite is not the search engine: public searches do
no SQL query and acquire no SQLite lock. SQLite WAL is the small crash-safe
operation journal used for idempotency keys, latest complete-video/channel
intent, tombstones, and publication sequence tracking. Replacing it would mean
building and validating a custom transactional log without making reads faster.

1. Store idempotent operations, complete replacement chunks, channel updates,
   and tombstones in SQLite WAL on the Raw volume.
2. Keep the validated base corpus archives on the same volume. A publisher
   streams the base corpus, omits replaced/deleted video IDs, appends complete
   replacements, merges channel updates, and builds a versioned candidate.
3. Open and validate the candidate before atomically changing the active
   generation manifest and swapping the in-process reader.
4. Video writes become durable without rebuilding immediately. An explicit
   publish captures a target operation sequence; mutations that arrive while it
   builds remain durable for the next explicit publish instead of making the
   worker chase a moving sequence. The indexing batch publishes once at its end.
5. On restart, open the manifest-selected generation, replay SQLite WAL state,
   and resume any durable unpublished operation.
6. Expose readiness, operation state, counts, video-ID pagination, replacement,
   deletion, and channel administration over an authenticated Raw admin API.
   The CLI consumes the backend-neutral `SearchIndexAdmin` contract.

This design removes work from the common write batch and keeps query reads on
an immutable `Arc<RawSearchEngine>`. The generic fallback is a full verified
generation rebuild; no unsafe mutable posting manipulation is introduced.

## Live migration checkpoint on 2026-08-10

Generation 461 activated while the nine-channel drift repair continued writing.
It contains 8,169,501 documents. The corrected publisher stopped after that
captured target rather than immediately starting another generation. During the
build, process HWM was 2,796,580 KiB and total cgroup usage briefly reached about
3.90 GiB; after activation and scratch deletion cgroup usage fell to about 1.97
GB. The temporary merged `chunks.jsonl` was 6.6 GiB and the candidate was 1.4
GiB. This passes the 4 GiB allocation gate narrowly but does not satisfy the 512
MiB compaction goal, so streaming external runs/delta segments remain required.

The deployment interrupted one write for video `g20JZ2HNZaw`. That video must
be replayed after the channel batch, followed by the final explicit publication,
restart, and public contract/load gates before Typesense is considered fully
deprecated. Its preserved resources remain stopped afterward.

### Streaming publication gate

Commit `204f754` removes the materialized merged corpus. The publisher now
filters the gzip base and chains serialized replacement chunks directly into the
builder. A release-build full-corpus publication on the development machine
completed at 03:59:07 UTC with the same 8,142,650 documents, 32,822 videos, 61
channels, and 293,152 terms. The completed mutable directory was 1,445,988,194
bytes, `work/` stayed empty, process HWM was 2,575,380 KiB, and RSS returned to
about 145 MiB after `malloc_trim`.

Commit `2ddbb5b` then replaces the retained global posting map with
lexicographically ordered external posting runs. Runs over 32 MiB are refined by
the next term byte, and the builder groups only one final run at a time. Fixed
document metadata is also streamed instead of retaining all 8.14 million
records. The final full-corpus release build peaked at 457,928 KiB (447.2 MiB),
completed in roughly 80 seconds, and used about 4.26 GB at peak before returning
to a 1,445,988,194-byte completed directory.

Every search-artifact BLAKE3 checksum matched the prior generation. Parsed and
ID-sorted channel/video metadata also matched exactly; their raw JSON checksums
differ only because the publisher canonicalizes array order. Smoke queries kept
`linux exploit` at 189 videos with `gIJOtP1AC3A` first and returned 317 videos
for `blackhole`. All 160 workspace tests pass (5 real-network/manual benchmarks
remain intentionally ignored), and Clippy is clean. The deployable image is
`registry.fly.io/ytsearch-raw-production:raw-bounded-2ddbb5b`; deploy it after
the active indexing batch completes, then repeat the memory/artifact gate on the
4 GiB Machine before retirement.

### Final live publication gate

The nine-channel repair completed and atomically activated generation 1166 with
8,197,954 documents. The bounded image was then deployed without rebuilding the
active snapshot. The previously interrupted Veritasium video `g20JZ2HNZaw` was
replayed as 66 transcript chunks; the bounded publisher activated generation
1167 with 8,198,020 documents, 33,823 videos, 61 channels, and 294,414 terms.
The completed search artifacts total 1,456,328,161 bytes, `work/` is empty, and
no posting or document-metadata spool remains.

The standalone builder's full-corpus HWM remains 457,928 KiB. The live serving
process reached 753,764 KiB while it kept the old mmap generation queryable and
built the replacement concurrently. This is a different, stricter measurement:
it includes the serving reader's resident file pages as well as publisher work.
After a clean restart, process RSS was 359,004 KiB (350.6 MiB), process HWM was
431,648 KiB, and cgroup usage was 1,705,377,792 bytes (1.59 GiB), including
Linux's reclaimable file cache. Both live measurements remain well inside the
4 GiB allocation, but only the standalone builder is below 512 MiB.

The restart reopened sequence 1167 and the replayed video was publicly
searchable. All 160 Rust workspace tests, 80 frontend tests, and 32 Worker tests
passed; five real-network/manual Rust benchmarks remain intentionally ignored.
Clippy, frontend typecheck, formatting, lint, and production build passed. A
75-request, five-concurrent production run had zero errors and a 308 ms p95.
The initial browser route is 24,343 bytes gzip.

## Fly retirement inventory

The following resources are legacy dependencies and must be treated as
deprecated during the migration:

- `ytsearch-typesense-staging`: active engine `8147deb94937e8`, volume
  `vol_v8e6yy17jo63nplv`, plus stopped benchmark Machine `d8de466f330468` and
  volume `vol_vdezd781q880qm34`.
- `ytsearch-api-typesense-fly-staging`: API `1855eddf654298` and queue volume
  `vol_vwnydd578odzzo8v`.
- `ytsearch-indexer-staging`: stopped Typesense indexer `68352d2be17728` and
  attached volume `vol_r68z3gl5wq7jkqd4`.

All five legacy queue rows were copied with their request IDs. The `@t3dotgg`
request completed through Raw with 95 videos and 30,684 chunks; the other four
retain their historical failed/rejected states. Historical unattached indexer
volumes are audited separately and are not implicitly deleted by this migration.
