# Common search experiment results

Measured 2026-08-01. This is the canonical comparison of Raw, Tantivy, and
Typesense. See [methodology.md](methodology.md) before interpreting a row.

## Decision summary

The raw inverted index is the strongest experimental backend on this corpus.
Against the matched Typesense control it reduced pooled p95 by 71%, CPU by 73%,
warm RSS by 93%, and disk by 94%. Against Tantivy it reduced pooled p95 by 49%,
CPU by 38%, warm RSS by 38%, and disk by 52%.

Raw became the production search reader on 2026-08-09. It is not yet approved
as the sole production writer or control plane. The original
four-query oracle had only 50% top-one agreement, and later manual review found
the Typesense oracle itself wrong on `battery gate incident`. Schema v5 now
passes the full available 420,720-chunk snapshot, but the larger human-judged
suite and shadow-traffic tail gate remain.

Tantivy was retired after this comparison. Its source is preserved on
`archive/tantivy-experiment` at `3aa3f55`; its only Fly machine was destroyed.
Raw is the sole active replacement experiment.

## Full-corpus Raw rollout (2026-08-09)

The schema-v6 generation was first validated in isolation, then promoted
checksum-identically to the production Raw reader after exact corpus parity,
contract, browser, and concurrent-load gates passed.

| Measurement | Full Raw result |
| --- | ---: |
| Transcript documents | 8,142,650 |
| Distinct videos | 32,822 |
| Creator channels | 61 |
| Lexicon terms | 293,152 |
| Index size | 1,444,907,196 bytes |
| Release build time | 88.99 s |
| Builder peak RSS | 2,473,776 KiB (about 2.36 GiB) |
| Warm search RSS | 363,622,400 bytes (about 347 MiB) |
| Experimental Fly allocation | 512 MiB, one shared vCPU, `sjc` |
| Production Fly allocation | 4 GiB, two performance vCPUs, `sjc` |

The engagement enrichment streamed all 8,142,650 input documents and resolved
metadata for 32,821 of 32,822 requested videos. One unavailable video remains
explicitly recorded rather than silently receiving fabricated metrics. The
850,118,690-byte gzip artifact passed gzip integrity validation and has SHA-256
`ff45cca577d217e0971cbaeab4de70e10ecd6c0d39073a51c74c4a58cb8cd4e0`.

The full contract passes authentication, aliases, creator scoping, channel
search, typo recovery, pagination, date ordering, and views/likes/comments/
combined-engagement ordering. The fixed `blackhole` checks found 317 videos
globally, 7 for Veritasium, and 15 for Vsauce. `blackhloe`, `blakhole`, and
`blackhle` recovered 272, 273, and 272 videos respectively.

### Full-snapshot latency and relevance sample

The direct-engine latency run used nine queries with 15 warm sequential
requests per query. Raw's median of per-query medians was 13.192 ms versus
215.859 ms for the restored Typesense snapshot, a 16.36x ratio of those two
summary values. The geometric mean per-query speedup was 15.46x, with every
query between 9.70x and 25.73x. Raw consumed 230 Linux CPU ticks and
2.231 seconds of measured internal search work for the 135 requests.

These latency results are directional rather than a perfect matched-corpus
comparison: the Raw generation contains 8.14 million documents while the
available Typesense restore contains 7.16 million. They demonstrate a large
margin on this host, but the production decision still requires matched
snapshots, concurrent load, and tail-latency gates.
The temporary Typesense comparison machine and its dedicated 10 GB clone
volume were destroyed after the measurements were captured; the production
Typesense machine was not changed.

The first full-snapshot differential contains seven real cases. Mean ordered
overlap@10 is 7.71, and top-one agreement is 4/7. A sparse fuzzy fallback bug
was discovered on `battery gate incident`: Raw initially returned two weak
matches while Typesense returned fourteen. Requiring every tracked query token
to match exactly before accepting the full-query fuzzy set caused Raw to use
ordered token dropping instead; the corrected query returns fifteen videos
with 9/10 overlap. This is useful evidence, but seven cases do not satisfy the
planned 300-query human-judged oracle.

A later live differential exposed a separate prefix-recall error. Raw stopped
expanding the final token when five exact candidates existed, so `linux exploit`
returned only 96 videos. Typesense returned 189. After moving ordinary prefix
expansion ahead of the typo threshold, Raw also returns 189 videos and 486 true
transcript moments. Ordered top-10 overlap is 4/10, however, so equal recall is
not evidence of equal ranking quality; this case belongs in the judged oracle.

Production rollout checks passed through all layers:

- Raw engine: <https://ytsearch-raw-production.fly.dev/health>
- Raw API: <https://ytsearch-api-raw-production.fly.dev/api/youtubers>
- Production frontend: <https://search.pronsh.dev>

The production engine contains 8,142,650 documents and 32,822 distinct video
IDs. An authoritative Typesense `facet_by=video_id` export and the Raw
`videos.json` list had zero sorted-ID differences. Both API facades exposed the
same 61 creators and channel counts at cutover. The Raw machine is exactly half
the provisioned Typesense size: 4 GiB / 2 performance vCPUs versus 8 GiB / 4.

The post-cutover public load gate sent 75 requests across five queries and
three sort modes at five-way concurrency: zero errors, 225 ms mean, 366 ms p95,
and 373 ms maximum including Cloudflare, Fly API, and network time. Engine
metrics immediately afterward reported 373,096,448 RSS bytes, 73 engine
searches, zero errors, and 2.031 seconds of internal search work. A clean shared
browser rendered `curspr/@cursor_ai` as 12 of 45 videos and 42 of 195 matching
moments, with `Our designer built an operating system with Cursor` ranked
first and all seven Raw sort choices available.

The experimental Worker, API, and engine were retired only after those checks.
Typesense remains available as a rollback artifact and the deployed legacy API
still owns the indexing queue until the Raw volumes/indexer are cut over. The
repository no longer contains a Typesense runtime or dual-write path.

### Mutable full-corpus publication

The durable Raw writer stores idempotent complete-video/channel operations in
SQLite WAL, builds and verifies a candidate, activates it through an atomic
manifest, retains one rollback generation, and swaps the serving reader.

On the 8,142,650-document corpus, two real delete/publication cycles completed
in about 70 and 80 seconds. Search visibility changed correctly after each
publication, publication errors remained zero, peak RSS was 2,576,420 KiB
(2.46 GiB), and peak mutable disk with one prior generation was about 8.7 GB.
After explicitly releasing allocator pages, steady RSS settled at 141,772 KiB
(138.45 MiB). See
[the full-corpus mutable report](raw-mutable-full-corpus-2026-08-09.md).

The first cold boot of the 1.45 GB container generation spent roughly three
minutes hashing and validating every artifact before binding the port. Warm
suspend/resume is faster, but index generations need volume or object-storage
distribution before code-only deploys and cold starts are operationally sound.

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
Exact negative phrases now use candidate-targeted schema-v6 position streams.
Their unchanged-query gate measured 7.654 ms parent p95 versus 7.593 ms for the
candidate across 900 searches each; mean/CPU rose 1.1%/1.4%. A separate
300-search phrase workload measured 7.210 ms p95, 97 CPU ticks, and no errors,
again without changing the index artifact.

Fail-closed request validation now covers transcript and channel routes before
cache lookup. The release contract script passed on the 47,860-chunk snapshot;
explicit `facet_by`, invalid/unbound cache TTL, list filter, and channel-sort
probes each returned HTTP 400. See
[request-validation.md](request-validation.md) for the accepted parameter
surface and malformed-value matrix.

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
