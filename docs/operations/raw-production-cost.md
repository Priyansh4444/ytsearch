# Raw production cost and persistence decision

Re-audited from live Fly state and Fly's published SJC rates on 2026-08-29.
Prices are monthly estimates for a 30-day month from [Fly.io resource
pricing](https://fly.io/docs/about/pricing/). Actual billing is prorated, and
egress remains usage-dependent.

## Current temporary allocation

| Resource | Allocation | Estimated monthly cost |
| --- | ---: | ---: |
| Production Raw reader/writer | `shared-cpu-2x`, 2 GB, always started (`shared-cpu-4x` from 2026-09-27; see below) | $13.58 |
| Temporary Raw publication canary | stopped after the failed same-reader gate; non-serving | $0 compute; up to $1.20 stopped rootfs |
| Queue/profile API | `shared-cpu-1x`, 512 MB, always started | $3.81 |
| All Raw, temporary, and retained Typesense volumes | 132 GB encrypted | $19.80 |
| CLI-listed snapshot recovery sizes | current logical-byte estimate after the first 10 GB free | about $9.58 before incremental-billing adjustment |
| Stopped Machine rootfs ceiling | at most 62 GB | at most $9.30 |
| **Current fixed allocation** | | **about $56.07/month before egress/indexer/Cloudflare** |

The temporary canary recorded in
[`config/fly-production-resources.json`](../../config/fly-production-resources.json)
is not a serving replica and was verified stopped on 2026-09-01. Its 20 GB
volume remains provisioned. The retained on-demand SJC indexer in
`ytsearch-indexer-raw-production` was also verified stopped and must not be
confused with the obsolete ORD publication builder, whose volume was extended
from 10 GB to 20 GB after the schema-8 build reached 77% disk utilization. The
extra 10 GB costs $1.50/month while provisioned. The canary's
publication proved that a same-reader full build causes requests to exceed the
five-second timeout, so adding permanent reader capacity is not the fix.

## Target allocation after the external-builder gate

Reuse the existing on-demand SJC indexer as a sequential fetcher and generation
builder. Add no Machine or volume. After the canary and the superseded ORD repair
Machine/volume are explicitly audited and removed, the conservative monthly
projection is:

| Resource | Estimated monthly cost |
| --- | ---: |
| Reader plus API compute | $17.39 |
| 92 GB encrypted volumes, including preserved Typesense evidence | $13.80 |
| Retained stopped rootfs ceiling (44 GB) | $6.60 |
| CLI-listed snapshot-size estimate after five-day expiry | about $8.60 |
| Indexer, capped at 60 hours | at most $1.13 |
| North American egress reserve | $8.00 |
| Snapshot-growth reserve | $2.00 |
| Cloudflare Paid minimum | $5.00 |
| **Conservative complete-system projection** | **about $62.52/month** |

The snapshot number is a conservative CLI logical-size estimate; Fly Cost
Explorer must confirm incremental billed bytes before the budget is declared
final. The approximately $62.52 target requires deleting only the obsolete Raw
canary Machine plus its 20 GB volume and the superseded ORD repair builder plus
its now-20 GB volume. Removing these two 20 GB volumes reduces the current
132 GB allocation to 92 GB. It preserves the active reader/API, the retained on-demand SJC
indexer, and every Typesense rollback resource. Immediately after those
deletions, retained snapshots make the conservative estimate about $63.50;
their approximately $0.98 reduction arrives after the five-day retention
window. Those deletions are irreversible and remain gated on the exact final
inventory and a separately reviewed cleanup. Merely stopping the Machines
leaves the projection at about $70.70 including
Cloudflare, the indexer allowance, egress, and snapshot-growth reserves.

Load-gating the API from 512 MB to 256 MB remains an optional further saving,
but do not resize it until its own load gate passes. A compute reservation is
also excluded from the proof until its scope and amortized billing are verified.

The stopped 4 GB temporary repair Machine must not run again unless unique cache
data is found during its deletion audit; continuous operation would add about
$28.48/month. The stopped Typesense
Machines and their 61 GB volume set are intentionally retained as deprecated
rollback and differential-test evidence. Their $9.15/month volume cost remains
part of the target above, not a pending deletion assumption.

The external-builder design caps the on-demand 2 GB indexer at 60 Machine-hours
per month. The intended automatic cadence is one gap pass on a four-hour wake
and no more than two coalesced generation builds per day; approved queue work
still wakes immediately. The queue remains durable and fails closed when the
runtime cap is reached.

The 2026-09-01 schema-8 build did not exhaust RAM: observed use stayed roughly
724–993 MiB on the 2 GB builder and Fly reported no OOM kill. It timed out while
holding a 2.6 GB unigram spool, 3.0 GB bigram spool, and document artifacts at
once. The builder now completes and deletes the unigram phase before replaying
the compressed document store into the bigram phase, then deletes the bigram
spool before broad-query materialization. This removes the measured reason for
a memory or CPU resize. Re-run the exact build gate before deciding whether the
retained SJC indexer needs a 30 GB volume; resize only if a measured
latest-schema peak reaches 80%.

At current SJC pricing, the previous `performance-2x`, 4 GB Raw reader alone
would cost about $73.92/month and violate the complete-system ceiling.
Moving the workload to the shared 2 GB allocation keeps the complete retained
topology below the $65/month target while the Typesense trial remains available
through the final retirement gates.

## Final Raw-only allocation after approved retirement

The requested terminal state also removes the 61 GB Typesense trial volumes and
the now-40 GB temporary Raw canary/ORD repair volumes after their exact final gates.
That leaves 31 GB provisioned: 20 GB reader, 1 GB API, and 10 GB on-demand
indexer. Using the current official SJC prices, the deterministic provisioned
floor is $17.39 reader/API compute plus $4.65 volumes, or $22.04/month. The
stopped 10 GB indexer rootfs adds at most $1.50/month.

With the existing 60-hour indexer cap ($1.13), Cloudflare Paid minimum ($5),
$8 egress reserve, and $2 snapshot-growth reserve, the conservative complete
Raw-only bound is $39.67/month. Actual invoices can be lower or higher only in
usage-priced categories such as egress and snapshot changed bytes; those are
not truthfully expressible as an exact fixed number in advance. The exact fixed
provisioned amount is $23.54/month including the stopped-rootfs ceiling.

## Resource and latency gates

The measured canary generation contains 8.32 million transcript documents. On 2026-08-13
the clean Raw process used about 275 MiB RSS, had a 365 MiB peak, and used no
swap; live bounded publication has previously peaked near 736 MiB process RSS.
On 2026-08-30 the live 2 GB reader reported 851,877,888 bytes (812.4 MiB) of
cgroup-v1 current memory, zero current swap, zero allocation failures, zero OOM
kills, and 17% volume utilization. Fly exposed a zero cgroup-v1 kernel peak for
that Machine, so this is explicitly a current sample rather than a claimed
historical peak; the publication gate records actual sample timestamps and
fails if their gap exceeds six seconds.
The 2 GB allocation includes 512 MB swap as a last-resort recovery margin;
normal operation must not depend on swap. Keep 2 GB until publication and load
tests prove that 1 GB retains safe headroom. The 17 MiB API is a candidate for a
separate 512-to-256 MiB load-gated reduction.

A public 75-request, five-concurrent `blackhole` load check produced:

| Allocation | Errors | Median | p95 |
| --- | ---: | ---: | ---: |
| `performance-2x`, 4 GB | 0/75 | 38.5 ms | 344.7 ms |
| `shared-cpu-2x`, 2 GB | 0/75 | 40.8 ms | 254.3 ms |

The p95 improvement is not treated as a CPU-speed result because the public
Cloudflare path and cache state add variance. The stable comparison is the 5.8%
median increase with no errors. Cold generation open increased from roughly 21
seconds to 86 seconds; that is acceptable only while the service remains
always-on. A clean 57-request canary baseline stayed below 200 ms, but the same
reader produced multiple requests above the five-second timeout as soon as a
full build began. Full generation construction is therefore prohibited on the
serving reader. The final gate builds on the on-demand indexer and separately
measures snapshot export, generation upload, verification, atomic swap, and the
warmed new reader.

If publication OOMs, swap remains active after publication, the search error
count increases, or a repeated matched workload regresses p95 by more than 25%,
roll back the binary while retaining `shared-cpu-2x`, 2 GB. Fail closed on a
resize: any larger allocation requires a newly calculated complete-system cost
gate and explicit authorization. Never use `performance-2x`, 4 GB as an
automatic fallback because that Machine alone exceeds the $65 ceiling.

## Four shared vCPUs for the reader (2026-09-27)

The owner authorized moving the reader from `shared-cpu-2x` to
`shared-cpu-4x` at the same 2 GB. Broad scans partition across every
available CPU, and a real-traffic replay on the exact canary showed:

| Canary shape | Warm replay p99 | Warm replay max | Synthetic max | Warm requests over 200 ms |
| --- | ---: | ---: | ---: | ---: |
| `shared-cpu-2x`, 2 GB | 136 ms | 228 ms | 227 ms | `and the` |
| `shared-cpu-4x`, 2 GB | 110 ms | 154 ms | 143 ms | none |

Fly's pricing page on 2026-09-27 listed `shared-cpu-2x`/2 GB at $19.42 and
`shared-cpu-4x`/2 GB at $23.31 per 30 days before regional markup, so the
change adds about $3.89 per month times the SJC multiplier. Memory, volumes,
and every other allocation are unchanged. Cold starts also shortened (the
canary became healthy in 76 s instead of 150 s). Revert with
`flyctl machine update d89303da029328 --vm-cpus 2` and the matching
`fly.raw-search.toml` change if the budget requires it.

## Why SQLite remains in the control plane

SQLite is not the search engine. Public queries read the immutable,
memory-mapped inverted-index generation and acquire no SQLite connection or
lock. SQLite stores only acknowledged mutations, idempotency keys, latest
video/channel replacements, and publication state.

On 2026-08-29, the live files measured:

| File | Bytes |
| --- | ---: |
| `mutations.sqlite3` | 78,839,808 |
| `mutations.sqlite3-wal` | 20,632 |
| `mutations.sqlite3-shm` | 32,768 |

That is about 75.2 MiB total on a 20 GB volume. Removing it cannot move the
Machine to a lower memory or storage tier, and it cannot make searches faster
because the query path never opens it.

A custom append-only journal would still need to reproduce full-sync durable
acknowledgement, atomic operation-plus-payload writes, idempotency conflict
detection, latest-record indexing, publication sequence snapshots, state
updates, corruption detection, migration, and crash recovery. Replacing a
tested transactional component without a tier-changing measurement would add a
large correctness surface for no demonstrated cost win.

Keep SQLite WAL for production. A Rust-native candidate such as `redb` may be
tested behind the same mutation-store contract, but it ships only if it passes
the existing idempotency/publication/restart/corruption matrix and either lowers
end-to-end publish CPU by at least 10% or enables another paid resource-tier
reduction. Binary size or microbenchmark wins alone are insufficient.
