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
- Typesense is destroyed only after queue state, all active writes, search reads,
  scheduler ownership, and recovery have moved to Raw and passed live gates.
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
stopped but recoverable until the Raw restart gate passes; destroy them only in
the final irreversible stage.

## Baseline on 2026-08-09

| Measurement | Value |
| --- | ---: |
| Raw process RSS | 373,555,200 bytes (356.25 MiB) |
| Raw index artifacts | 1,444,907,196 bytes |
| Raw engine allocation | 4 GiB, 2 performance vCPUs, `sjc` |
| Raw API allocation | 512 MiB, 1 shared vCPU, `sjc` |
| Raw engine searches/errors | 86 / 0 |
| Internal engine search time | 2.044 seconds |

Current Fly list pricing for `sjc` is $0.1027/hour for the Raw engine and
$0.0053/hour for the API. At the enforced 12-hour daily search window, compute
is approximately $36.97 + $3.81 = $40.78 per 30 days. A 20 GB Fly Volume costs
$3/month at $0.15/GB-month, producing an expected mutable-stack baseline of
about $43.78/month before bandwidth, snapshot bytes above the included tier,
stopped-rootfs bytes, and Cloudflare usage. Always-on compute would instead be
approximately $80.73/month including that volume.

## Design

The mmap schema-v6 generation remains immutable. Mutability is implemented as
a durable control plane and atomic generation publication, not by placing locks
inside the query hot path.

1. Store idempotent operations, complete replacement chunks, channel updates,
   and tombstones in SQLite WAL on the Raw volume.
2. Keep the validated base corpus archives on the same volume. A publisher
   streams the base corpus, omits replaced/deleted video IDs, appends complete
   replacements, merges channel updates, and builds a versioned candidate.
3. Open and validate the candidate before atomically changing the active
   generation manifest and swapping the in-process reader.
4. Video writes become durable without rebuilding immediately. The channel
   upsert at the end of an indexing batch publishes all pending mutations once,
   preventing an 8.14-million-document rebuild per video.
5. On restart, open the manifest-selected generation, replay SQLite WAL state,
   and resume any durable unpublished operation.
6. Expose readiness, operation state, counts, video-ID pagination, replacement,
   deletion, and channel administration over an authenticated Raw admin API.
   The CLI consumes the backend-neutral `SearchIndexAdmin` contract.

This design removes work from the common write batch and keeps query reads on
an immutable `Arc<RawSearchEngine>`. The generic fallback is a full verified
generation rebuild; no unsafe mutable posting manipulation is introduced.

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

The queue currently contains one pending `@t3dotgg` request. It must be copied
to the persistent Raw API queue and processed through Raw before destruction.
Historical unattached indexer volumes are audited separately and are not
implicitly deleted by this migration.
