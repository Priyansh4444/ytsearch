# Raw production cost and persistence decision

Measured on 2026-08-13 in `sjc`. Prices are monthly estimates for a 30-day
month from [Fly.io resource pricing](https://fly.io/docs/about/pricing/); actual
billing is prorated and egress remains usage-dependent.

## Always-on allocation

| Resource | Allocation | Estimated monthly cost |
| --- | ---: | ---: |
| Raw reader/writer | `shared-cpu-2x`, 2 GB, always started | $13.58 |
| Queue/profile API | `shared-cpu-1x`, 512 MB, always started | $3.81 |
| All Raw and retained Typesense volumes | 102 GB encrypted | $15.30 |
| Current volume snapshots | 108.61 GiB; first 10 GB free | about $7.89 |
| Stopped Machine rootfs ceiling | at most 54 GB | at most $8.10 |
| **Conservative fixed ceiling** | | **at most $48.68/month** |

The on-demand Raw indexer is `shared-cpu-2x`, 2 GB at $0.0189/hour and normally
stops after queued work. Budgeting 100 hours adds $1.89. Reserving $8 for 400 GB
of North American egress and $2 for snapshot growth produces a conservative
**$60.57/month** total, leaving $4.43 below the hard $65/month ceiling. The
Cloudflare Worker and Queue are outside this Fly subtotal, so that headroom
must remain reserved if the ceiling covers both providers.

The stopped 4 GB temporary repair Machine must run only for bounded migration
work; continuous operation would add about $28.48/month. The stopped Typesense
Machines and their 61 GB volume set are intentionally retained as deprecated
rollback and differential-test evidence. Their $9.15/month volume cost is part
of the ceiling above, not a pending deletion assumption.

The hourly durable repair wake may start the on-demand 2 GB indexer, which exits
after five idle minutes. An entirely idle month is at most about 60 runtime
hours; the envelope above reserves 100 hours so retry and bounded repair work
still fit. The hourly cadence keeps a six-hour transcript cooldown from slipping
to nearly twelve hours when a wake lands just before its retry deadline.

At current SJC pricing, the previous `performance-2x`, 4 GB Raw reader alone
would cost about $73.92/month and violate the complete-system ceiling.
Moving the workload to the shared 2 GB allocation keeps the complete retained
topology below the $65/month target without destroying the Typesense trial.

## Resource and latency gates

The active generation contains 8.32 million transcript documents. On 2026-08-13
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
always-on and the five-minute reconciler restarts failures. A successful live
full-corpus publication on 2 GB remains the final sizing gate.

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
