# Raw production cost and persistence decision

Measured on 2026-08-11 in `sjc`. Prices are monthly estimates for a 30-day
month from [Fly.io resource pricing](https://fly.io/docs/about/pricing/); actual
billing is prorated and egress remains usage-dependent.

## Always-on allocation

| Resource | Allocation | Estimated monthly cost |
| --- | ---: | ---: |
| Raw reader/writer | `shared-cpu-2x`, 2 GB, always started | $11.83 |
| Queue/profile API | `shared-cpu-1x`, 512 MB, always started | $3.32 |
| Raw volume | 20 GB encrypted | $3.00 |
| Queue API volume | 1 GB encrypted | $0.15 |
| Indexer cache volume | 10 GB encrypted | $1.50 |
| Stopped indexer rootfs ceiling | 10 GB | $1.50 |
| **Fixed Raw-only total** | | **about $21.30/month** |

The fixed total is about **$0.71/day**. This is below both a $65/day ceiling and
the stricter $65/month target used for this migration.

The indexer normally starts only for queued work or scheduled repair. Its
current `shared-cpu-4x`, 4 GB allocation is $0.0329/hour while started. Even if
it ran continuously, the Raw-only fixed total would be about $43.46/month
($1.45/day) before variable egress. After the corpus migration, resize it to
`shared-cpu-2x`, 2 GB if the observed ingestion RSS remains below the rollback
threshold.

Destroying the complete verified legacy set removes 61 GB of old encrypted
volumes, or another $9.15/month, in addition to any retired Machine rootfs
charges. Those savings are not subtracted from the Raw-only total above because
they are legacy cleanup rather than Raw capacity.

The previous `performance-2x`, 4 GB Raw reader alone cost about $64.39/month.
Moving the measured workload to the shared 2 GB allocation removes about
$52.56/month from the reader and brings the full fixed stack below the $65/month
target with more than $40/month of headroom.

## Resource and latency gates

The full snapshot contains 8.19 million transcript documents. Before resizing,
the clean Raw process settled near 351 MiB RSS and live bounded publication
peaked near 736 MiB process RSS. The 2 GB allocation includes 512 MB swap as a
last-resort recovery margin; normal operation must not depend on swap.

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

Rollback to `performance-2x`, 4 GB if publication OOMs, swap remains active
after publication, the search error count increases, or a repeated matched
workload regresses p95 by more than 25%.

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
