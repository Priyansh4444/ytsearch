# Common search experiment results

Measured 2026-08-01. This is the canonical comparison of Raw, Tantivy, and
Typesense. See [methodology.md](methodology.md) before interpreting a row.

## Decision summary

The raw inverted index is the strongest experimental backend on this corpus.
Against the matched Typesense control it reduced pooled p95 by 71%, CPU by 73%,
warm RSS by 93%, and disk by 94%. Against Tantivy it reduced pooled p95 by 49%,
CPU by 38%, warm RSS by 38%, and disk by 52%.

It is not yet approved as the sole production replacement. The original
four-query oracle had only 50% top-one agreement, and later manual review found
the Typesense oracle itself wrong on `battery gate incident`. Schema v5 now
passes the full available 420,720-chunk snapshot, but the larger human-judged
suite and shadow-traffic tail gate remain.

Tantivy was retired after this comparison. Its source is preserved on
`archive/tantivy-experiment` at `3aa3f55`; its only Fly machine was destroyed.
Raw is the sole active replacement experiment.

## Latest Raw validation

The follow-up [Raw validation](raw-validation.md) uses 47,860 chunks from 787
Veritasium/Vsauce videos. For `blackhole`, Raw returned all 21 manually
identified literal `black hole`/`black holes` videos: 6 Veritasium and 15
Vsauce. It agreed with Typesense's top result in all three scopes while avoiding
a Black-Scholes false positive.

The final nine-query Raw run measured 1.42 ms p50, 3.26 ms p95, 0.64
CPU-seconds over 450 searches, and 23.37 MB warm RSS. At 16-way concurrency,
1,600 requests completed with zero errors at 1,602 requests/second; a separate
8,000-request run plateaued below 67 MB RSS.

The later [Typesense compatibility audit](typesense-compatibility.md) adds a
compact mmap trie with prefix, transposition, one-edit, and two-edit support.
The updated nine-query run measured 1.18 ms p50 and 2.09 ms p95. On fourteen
typo/prefix queries Raw measured 1.83/3.14 ms p50/p95 versus Typesense 30.2 at
3.80/9.12 ms, while recovering `blackhloe` and `blackhle` as black-hole intent
where the matched Typesense control returned zero.

Schema v5 adds compact first-token positions, packed exact-token masks, and
filter-aware conjunctive token dropping. On the engagement-enriched
four-channel snapshot it is 83.64 MB and built in 4.42 seconds. The active HTTP
contract passes with nonzero views/likes/comments. On the two-channel
differential, broad multi-word sets shrink substantially (`battery gate
incident` 41→1, `why is the sky blue` 61→1); the four-channel battery case
instead moves 3→10 because the added T3 videos contain literal battery-gate
mentions, so it remains in the judged-query gate. Against archived v4, core p95 moved only
1.8% (7.51→7.64 ms), while mean/CPU increased about 6–8%; the short typo run's
tail remains a shadow-traffic gate. Full measurements are in
[typesense-compatibility.md](typesense-compatibility.md).

Schema v6 adds complete positions with 2.94% artifact growth on the two-channel
snapshot. Exact quoted phrases and bounded slop are now available, and ordinary
exact multi-token results receive a guarded same-field proximity score. The
retained candidate-targeted implementation preserved all ordered top results
in the frozen nine-query suite; versus the immediate schema-v6 baseline its
900-search A/B moved p95 from 7.740 to 7.230 ms while mean and CPU rose 1.7%
and 1.8%. See [schema-v6-positions.md](schema-v6-positions.md),
[quoted-phrase-search.md](quoted-phrase-search.md), and
[proximity-ranking.md](proximity-ranking.md).

Exact token-start negative terms now reuse the existing sorted postings and
filter candidates before typo/drop thresholds. The unchanged nine-query A/B
measured 4.188/7.929/4.483 ms median/p95/mean for the immediate parent and
4.333/7.388/4.501 ms for the candidate across 900 searches each; CPU was
353 versus 355 ticks. The separate 300-search matched-exclusion workload had a
5.075 ms p95 and no errors without adding index bytes. See
[negative-term-search.md](negative-term-search.md) for semantics and evidence.

## Full 1,956-video corpus

| Measurement | Raw | Tantivy | Typesense |
| --- | ---: | ---: | ---: |
| Pooled p50 | 1.95 ms | 2.42 ms | 4.95 ms |
| Pooled p95 | 5.15 ms | 10.05 ms | 17.93 ms |
| Mean | 2.29 ms | 3.54 ms | 6.90 ms |
| CPU, 400 searches | 0.83 s | 1.34 s | 3.08 s |
| Warm RSS | 33.0 MB | 53.5 MB | 470.2 MB |
| Index/data directory | 61.54 MB | 129.03 MB | 1,064.03 MB |
| Build/import | 1.87 s | 1.16 s | 53.21 s |

Raw's measured loss is build speed: its safe two-field postings builder is 61%
slower than Tantivy on the full sample. Search startup, steady-state resources,
and query performance still favor Raw.

## Corrected scaling matrix

The rows below use the deterministic nested slices, not the rejected biased
slices.

### Index size and build time

| Videos | Backend | Chunks | Index/data | Build/import |
| ---: | --- | ---: | ---: | ---: |
| 500 | Raw | 88,408 | 15.36 MB | 0.52 s |
| 500 | Tantivy | 88,408 | 31.88 MB | 0.70 s |
| 500 | Typesense | 88,408 | 218.68 MB | 10.64 s |
| 1,000 | Raw | 184,375 | 31.17 MB | 1.05 s |
| 1,000 | Tantivy | 184,375 | 64.41 MB | 0.94 s |
| 1,000 | Typesense | 184,375 | 458.89 MB | 31.83 s |
| 1,956 | Raw | 372,860 | 61.54 MB | 1.87 s |
| 1,956 | Tantivy | 372,860 | 129.03 MB | 1.16 s |
| 1,956 | Typesense | 372,860 | 1,064.03 MB | 53.21 s |

### Search and resources

| Videos | Backend | p50 | p95 | CPU / 400 | Warm RSS |
| ---: | --- | ---: | ---: | ---: | ---: |
| 500 | Raw | 1.75 ms | 4.66 ms | 0.73 s | 22.2 MB |
| 500 | Tantivy | 2.10 ms | 6.35 ms | 0.89 s | 34.3 MB |
| 500 | Typesense | 3.75 ms | 7.33 ms | 1.78 s | 476.4 MB |
| 1,000 | Raw | 2.10 ms | 5.45 ms | 0.85 s | 30.3 MB |
| 1,000 | Tantivy | 2.50 ms | 8.19 ms | 1.06 s | 36.9 MB |
| 1,000 | Typesense | 5.30 ms | 12.52 ms | 2.59 s | 462.2 MB |
| 1,956 | Raw | 1.95 ms | 5.15 ms | 0.83 s | 33.0 MB |
| 1,956 | Tantivy | 2.42 ms | 10.05 ms | 1.34 s | 53.5 MB |
| 1,956 | Typesense | 4.95 ms | 17.93 ms | 3.08 s | 470.2 MB |

Raw disk growth is close to linear with source chunks. Its p95 remains within
4.66–5.45 ms and its RSS grows sublinearly across these slices. Query latency is
not strictly monotonic because term selectivity and scheduler noise change by
slice; there is no longer an inverted smaller-is-slower trend.

## Relevance

| Case | Raw overlap@10 | Tantivy overlap@10 | Top one agrees with Typesense |
| --- | ---: | ---: | --- |
| `fable on high` | 6/7 | 6/7 | No |
| `battery gate incident` | 3/8 | 3/8 | No |
| `linux exploit` | 9/10 | 9/10 | Yes |
| low-signal newest | 10/10 | 10/10 | Yes |
| **Mean / total** | **7.0** | **7.0** | **2/4 (50%)** |

Manual review changes the interpretation of `battery gate incident`: Raw and
Tantivy return exactly the three actual Battery Gate videos, while Typesense's
first three results are unrelated battery mentions. The disagreement is useful,
but it does not replace a larger human-judged query set.

## Shared cache

The response cache lives in `youtube_search_service`, so a cache hit does not
exercise either experimental engine. Raw's current 4,000-hit run measured 0.369
ms p50, 1.026 ms p95, and 1.09 CPU-seconds. Earlier runs measured Tantivy at
0.425/0.751 ms and Typesense at 0.836/1.383 ms. These rows verify the cache
contract; they are not used to choose an engine.

## Existing API compatibility

The first compatibility run used `youtube_api` unchanged against Raw. Twenty
requests for each case produced zero errors:

| Case | API p50 | API p95 | Stable result |
| --- | ---: | ---: | --- |
| `battery gate incident` | 1 ms | 2 ms | 3 videos / 7 moments |
| `linux exploit` | 2 ms | 5 ms | 9 videos / 21 moments |
| `fable on high` | 2 ms | 5 ms | 6 videos / 12 moments |
| low signal | 3 ms | 4 ms | 1,060 videos / 20 returned moments |
| page two | 1 ms | 2 ms | Empty page, stable total |

This covers the real pagination, nearby-hit merge, grouping, date sorting,
gzip, and public response path rather than only the engine endpoint. A later
exact-token audit found the API's legacy Typesense substring reranker could
undo Raw's backend order. The experimental API now enables an explicit
Raw-only preserve-order policy; Typesense retains the old default. The deployed
facade and backend both rank “Which Way Is Down?” first for `relativity
explained simply`.

The latest public contract also passes typo recovery, creator scoping,
views/likes/comments ordering, ascending/descending date order, pagination,
channel discovery, and authentication. Initial browser JavaScript remains
24,002 bytes gzip against the 30,000-byte ceiling.

## Fly experiments

| Measurement | Raw machine | Retired Tantivy measurement |
| --- | ---: | ---: |
| Allocated memory | 256 MB | 512 MB |
| RSS after 400 searches | 26.36 MB | 45.6 MB |
| Internal mean search work | 1.93 ms | 4.98 ms |
| CPU, 400 searches | 0.97 s | 1.85 s |
| Public p95 from development host | 115.9 ms | 146 ms |

Public latency includes TLS and network transit and is not mixed into the local
engine percentages. Both apps use one shared CPU in `sjc`, require a search key,
keep zero machines warm, and suspend when idle.

- Raw health: <https://ytsearch-raw-experiment.fly.dev/health>
- Tantivy has no machine or live endpoint. Its historical rows remain for the
  trial record.

## Full-snapshot context

Restoring the 7,159,726-document Typesense snapshot clone took 9m21s and about
3.5 GB RSS before health checks passed. This is a startup baseline, not evidence
that the raw prototype can already replace the full index.

## Promotion gates

| Gate | Raw result |
| --- | --- |
| p95 at least 25% below Typesense | Pass: 71% lower |
| CPU at least 25% below Typesense | Pass: 73% lower |
| RSS at least 40% below Typesense | Pass: 93% lower |
| Scaling no worse than Typesense | Pass on retained slices |
| Contract behavior | Pass on unit and unchanged-API tests |
| Mean overlap@10 at least 7 | Pass: 7.0 |
| Top-one agreement at least 80% | **Fail: 50%** |

Decision: keep Raw as the active isolated experiment and expand the judged
query set and full-snapshot corpus before any shadow-production proposal.
