# Raw production cost and persistence decision

Re-audited on 2026-08-19 in `sjc`. Prices are monthly estimates for a 30-day
month from [Fly.io resource pricing](https://fly.io/docs/about/pricing/); actual
billing is prorated and egress remains usage-dependent.

## Current temporary allocation

| Resource | Allocation | Estimated monthly cost |
| --- | ---: | ---: |
| Production Raw reader/writer | `shared-cpu-2x`, 2 GB, always started | $13.58 |
| Temporary Raw publication canary | stopped after the failed same-reader gate; non-serving | $0 compute; up to $1.20 stopped rootfs |
| Queue/profile API | `shared-cpu-1x`, 512 MB, always started | $3.81 |
| All Raw, temporary, and retained Typesense volumes | 122 GB encrypted | $18.30 |
| CLI-listed snapshot recovery sizes | about 168.48 GiB; first 10 GB free | about $12.68 before incremental-billing adjustment |
| Stopped Machine rootfs ceiling | at most 62 GB | at most $9.30 |
| **Temporary fixed allocation** | | **about $57.67/month before egress/indexer/Cloudflare** |

The temporary canary is not a serving replica and is now stopped. Its
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
| CLI-listed snapshot-size estimate | about $12.39 |
| Indexer, capped at 60 hours | at most $1.13 |
| North American egress reserve | $8.00 |
| Snapshot-growth reserve | $2.00 |
| **Conservative Fly projection** | **about $61.31/month** |

The snapshot number is a conservative logical-size estimate; Fly Cost Explorer
must confirm incremental billed bytes before the budget is declared final. If a
$5 Cloudflare Paid minimum belongs inside the same ceiling, this projection is
about $66.31. Load-gating the API from 512 MB to 256 MB saves about $1.49, and a
single $36/year shared-compute reservation is expected to save about $2/month
net; using both projects roughly $62.82/month amortized. Do not buy a reservation
or resize the API until its own load gate passes.

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

At current SJC pricing, the previous `performance-2x`, 4 GB Raw reader alone
would cost about $73.92/month and violate the complete-system ceiling.
Moving the workload to the shared 2 GB allocation keeps the complete retained
topology below the $65/month target without destroying the Typesense trial.

## Resource and latency gates

The measured canary generation contains 8.32 million transcript documents. On 2026-08-13
the clean Raw process used about 275 MiB RSS, had a 365 MiB peak, and used no
swap; live bounded publication has previously peaked near 736 MiB process RSS.
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

## Why SQLite remains in the control plane

SQLite is not the search engine. Public queries read the immutable,
memory-mapped inverted-index generation and acquire no SQLite connection or
lock. SQLite stores only acknowledged mutations, idempotency keys, latest
video/channel replacements, and publication state.

The live files measured:

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
