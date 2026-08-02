# Raw search validation

Measured 2026-08-01 and updated 2026-08-02 through the schema-v5 audit. This
report extends the common backend experiment with real creator corpora and
adversarial checks around compact and multi-token queries, creator scope,
ranking, dates, concurrency, and malformed index data.

## Contract

Workload: grouped transcript search through the Raw HTTP service and unchanged
`youtube_api`, centered on `blackhole` globally and under `@veritasium` and
`@vsauce` filters.

Invariant: creator filters, relevance order, explicit date order, pagination,
grouping, highlights, authentication, route aliases, and public API response
shape must remain correct. The original eight-query workload must not incur a
material CPU regression.

Rollback: Typesense remains production. Raw is an isolated read-only service.

## Real corpus

The data came from the stopped snapshot clone, accessed only through its
loopback interface. The clone had no process group or services, was stopped
again after export, and never joined the serving pool.

| Measurement        |   Result |
| ------------------ | -------: |
| Transcript chunks  |   47,860 |
| Videos             |      787 |
| Channels           |        2 |
| Source JSONL       | 34.19 MB |
| Raw fuzzy-v2 index | 10.54 MB |
| Terms              |   29,416 |
| Build time         |   304 ms |

## Deployed four-channel contract

The public experiment is available at
`https://ytsearch-raw-experiment.fly.dev`. Its health route reports the active
backend without authentication; search routes require the experiment key from
the ignored local `.search-experiment/search-key` file.

The deployed fuzzy-v2 corpus expands this benchmark to 420,720 chunks, 2,743
videos, four channels, and 59,596 terms. Its index is 72,441,536 bytes and was
built in 4,090 ms. The external contract passes global, Veritasium, Vsauce,
and channel searches; `blackhole`, `blackhloe`, `blakhole`, and `blackhle` typo
cases; relevance/date ordering; creator scope; grouping; pagination; both
route aliases; and missing-key authentication.

The builder also recovered missing numeric publication timestamps from the
existing RFC 3339 `published_at` field through `youtube_time`.

## Engagement v3 deployment

The active four-channel index is schema v3: 420,720 chunks, 2,743 videos,
59,596 terms, and 73,018,300 bytes. All videos have current public views,
likes, and comments. The existing contract plus combined engagement, pure
views, pure likes, and pure comments order passes on the deployed service.

The matched 270-request deployed run measured 96.55 ms external p50,
120.60 ms p95, 2.082 ms internal mean, 70 CPU ticks, and 33.14 MB ending RSS.
The pre-engagement baseline measured 108.00 ms, 134.92 ms, 2.010 ms, 67 ticks,
and 33.09 MB respectively. Network timing improved, while the engine-level
cost stayed effectively flat. See
[engagement-ranking.md](engagement-ranking.md) for the ranking formula, index
delta, isolation boundary, and full results.

## `blackhole` correctness

Raw treats a compact token as eligible for bounded lexicon splitting. For this
case it evaluates `blackhole`, `black + hole`, and the narrow plural
`black + holes`. The split path intersects both components in one transcript
chunk; it does not enable broad edit-distance matching such as `hole` →
`whole`.

| Scope                      | Raw found | Typesense found | Same top one | overlap@10 |
| -------------------------- | --------: | --------------: | ------------ | ---------: |
| Matched two-channel corpus |        21 |              17 | Yes          |          7 |
| Veritasium                 |         6 |               7 | Yes          |          6 |
| Vsauce                     |        15 |              13 | Yes          |          8 |

Manual literal-phrase review found 21 videos containing `black hole` or
`black holes`; Raw returned all 21. Typesense's seventh Veritasium result was
“The Equation That Beat Wall Street,” matched through `Black`, `whole`, and
typo expansion rather than black-hole content.

Raw's leading results were:

1. “Travel INSIDE a Black Hole” — Vsauce
2. “The real science of black holes” — Veritasium
3. “The Most Misunderstood Concept in Physics” — Veritasium
4. “Is Time Travel Possible?: Mind Blow” — Vsauce
5. “Something Strange Happens When You Follow Einstein's Math” — Veritasium

The compact query returns five plural-only videos beyond the strict spaced
query. This is an intentional guarded recovery path, not global stemming.

## Typo and prefix compatibility

The [Typesense compatibility audit](typesense-compatibility.md) traces the
active 30.2 implementation and query defaults. Raw fuzzy v2 now uses a compact
mmap trie with progressive prefix, one-edit, and two-edit expansion;
adjacent transpositions use optimal-string-alignment Damerau-Levenshtein
distance. Candidate terms are field-frequency ranked and corrected terms drive
highlights.

Raw additionally composes a one-edit correction with a compact split when one
side is already exact in the same chunk. This recovers black-hole intent that
Typesense misses on this corpus:

| Query       | Raw found | Typesense found | Raw top result               |
| ----------- | --------: | --------------: | ---------------------------- |
| `blackhloe` |        21 |               0 | “Travel INSIDE a Black Hole” |
| `blackhle`  |        21 |               0 | “Travel INSIDE a Black Hole” |
| `blakhole`  |        22 |               1 | “Travel INSIDE a Black Hole” |

The extra `blakhole` result is the control's literal fuzzy match; the 21
black-hole results rank above it. The composed path requires one exact and one
one-edit component, requires same-chunk intersection, and rejects shorter
corrections such as `black + he`.

## Sorting and public API

The contract harness validates monotonic RFC 3339 timestamps, not merely the
backend's internal metadata. The relevance, newest, and oldest orders are
different and deterministic.

| Check                                | Result                                      |
| ------------------------------------ | ------------------------------------------- |
| Relevance top result                 | `3pAnRKD4raY`, “Travel INSIDE a Black Hole” |
| Newest Raw top result                | `piHGnG4LsmQ`, 2025-12-24                   |
| Oldest Raw top result                | `KB24FakYVmc`, 2011-04-15                   |
| Global/Veritasium/Vsauce totals      | 21 / 6 / 15 videos                          |
| Page-one/page-two overlap            | 0                                           |
| Raw alias equals compatibility route | Yes                                         |
| Missing authentication               | HTTP 401                                    |

The unchanged `youtube_api` returned the same totals and date endpoints after
its normal nearby-hit merge and reranking. Twenty requests per API case:

| API case         | Errors |  p50 |  p95 |
| ---------------- | -----: | ---: | ---: |
| Global relevance |      0 | 2 ms | 3 ms |
| Veritasium       |      0 | 2 ms | 2 ms |
| Vsauce           |      0 | 2 ms | 3 ms |
| Newest           |      0 | 2 ms | 3 ms |
| Oldest           |      0 | 2 ms | 2 ms |
| Page two         |      0 | 2 ms | 3 ms |

## Final fuzzy-v2 engine measurements

The sequential matrix has nine queries, 10 warmups each, and 50 measured
requests each. It includes both `blackhole` and `black hole`.

| Measurement   |      Result |
| ------------- | ----------: |
| Searches      |         450 |
| p50           |     1.18 ms |
| p95           |     2.09 ms |
| Mean          |     1.27 ms |
| CPU           |      0.50 s |
| Internal mean |     0.86 ms |
| Warm RSS      | about 23 MB |

On the fourteen-query typo/prefix matrix, 700 searches measured 1.83 ms p50,
3.14 ms p95, 1.94 ms mean, and 115 CPU ticks. The matched Typesense 30.2
control measured 3.80 ms p50, 9.12 ms p95, and 3.89 ms mean. Raw used about
25 MB warm RSS versus 474–496 MB for Typesense; its index was 10.54 MB versus
107.31 MB for the Typesense data directory.

Across three fresh processes with 20 warmups and 200 `blackhole` searches each,
the median run measured 1.89 ms p50, 4.33 ms p95, 1.44 ms internal mean, and
0.37 CPU-seconds.

At 16-way HTTP concurrency fuzzy v2 completed 1,600 `blackhloe` requests with
zero errors at 1,535 requests/second. Internal mean was 2.49 ms, CPU was 3.89
seconds, and RSS moved from 23.69 MB to 54.06 MB. In a five-round 8,000-request
plateau run every request succeeded; RSS ended at 66.02 MB with a 66.02 MB
observed peak. This deliberately measures the new composed typo path rather
than the cheaper exact `blackhole` path.

## Schema-v5 position and multi-token gate

The four-channel schema-v5 candidate contains 420,720 chunks, 2,743 videos,
59,596 terms, and 83,637,146 bytes. It built from the 324 MB
engagement-enriched source JSONL in 4.419 seconds. Positions plus populated
counter metadata add 12.6% over the archived 74,256,152-byte zero-counter v4
index. The query hot candidate remains eight bytes: four
packed bits count exact query tokens, one bit records transcript provenance,
and 27 bits retain capacity for more than 100 million documents.

Multi-token evaluation no longer falls back to an OR of every query word. It
expands prefix and typo alternatives within an intersection, then drops suffix
tokens one at a time before switching direction. The stop threshold is applied
after creator/video filtering. Four focused fixtures verify longest-subset
preference, direction switching, filter-aware stopping, and a typo inside a
retained conjunction. A fifth verifies exact-token priority over a
higher-frequency correction.

On the production-shaped corpus, representative result counts changed as
intended:

| Query | Archived broad fallback | Schema v5 |
| --- | ---: | ---: |
| `battery gate incident` | 41 | 1 |
| `why is the sky blue` | 61 | 1 |
| `relativity explained simply` | 139 | 2 |
| `black hole event horizon` | 16 | 3 |

The full contract passes with 30 global `blackhole` videos, including 6
Veritasium and 15 Vsauce results; the extra nine come from the two additional
channels. `blackhloe`, `blakhole`, and `blackhle` each recover all 30 with the
same top result. Relevance, newest/oldest, engagement, pure counter sorts,
pagination, channel lookup, authentication, and both search routes also pass.
The harness now requires a positive value in every counter-sorted result set;
this rejected an intermediate v5 build made from the stale zero-counter JSONL
before it could remain deployed. The corrected public maxima for the
`blackhole` result set are 34,876,084 views, 1,724,038 likes, and 56,264
comments.

The matched 225-search core A/B measured v4 at 4.39/7.51/4.58 ms
p50/p95/mean and v5 at 5.19/7.64/4.89 ms. CPU moved from 93 to 100 ticks. The
matched 350-search typo A/B measured v4 at 12.28/23.58/12.35 ms and v5 at
11.86/27.74/13.04 ms; CPU moved from 415 to 439 ticks. The retained contract
therefore costs 5.6–7.5% in mean/CPU in these runs; the short-run typo p95 is a
named shadow-traffic gate. Warm RSS after the typo matrix was 52.6 MB for v4
and 57.6 MB for v5.

## Optimization and rejected scope

The first split implementation hydrated every candidate to award a phrase
bonus. At 16-way concurrency that duplicated decompressed blocks across
requests: RSS reached 84.97 MB, CPU took 3.79 seconds, and internal mean was
2.50 ms. Replacing hydration with a score-only guarded split reduced the
matched run to 41.13 MB, 2.44 CPU-seconds, and 1.48 ms internal mean without
changing its top ten.

A later attempt applied singular/plural and join normalization to every normal
multi-term query. The archived-binary A/B rejected it: median p95 increased
from 4.28 to 7.13 ms and CPU from 80 to 101 ticks per 400 searches. The final
implementation confines plural expansion to a successfully split compact
token.

Against the archived pre-split binary on the original eight-query workload,
the final narrowed implementation pooled 1,200 searches per variant:

| Variant            |     p50 |     p95 |    Mean | Median CPU / 400 | Internal mean |
| ------------------ | ------: | ------: | ------: | ---------------: | ------------: |
| Archived pre-split | 2.08 ms | 6.19 ms | 2.56 ms |         94 ticks |       1.95 ms |
| Final narrow split | 2.17 ms | 6.61 ms | 2.67 ms |         95 ticks |       1.97 ms |

The retained cost is therefore 4.4% p50, 6.8% p95, 4.3% mean, and one CPU tick
per 400 searches on this noisy HTTP workload. The correctness gain is kept;
the rejected broad path is not.

## Correctness evidence and gaps

- 43 Raw tests and 3 shared-service tests pass in release; the full workspace gate is
  run before handoff.
- Tests cover grouping, highlights, creator/video filters, relevance/date
  order, compact split and plural recovery, prefix search, transpositions,
  two-edit correction, typo-plus-split recovery, pagination, field selection,
  wildcard/channel search, empty corpora, concurrent clones, stable slicing,
  overwrite refusal, corrupt magic, truncated postings/positions/documents,
  position-offset validation, manifest mismatches, fuzzy-trie truncation,
  reference-distance equivalence, integer boundaries, and malformed varints.
- Clippy passes with warnings denied.
- Miri and sanitizer tooling were unavailable on this host; mmap behavior is
  protected by the immutable-file safety contract and bounds-checked readers,
  but this is still a named coverage gap.
- The schema-v5 gate covers the full available 2,743-video snapshot, but it is
  still not a substitute for judged queries or production shadow traffic.
- The concurrency driver uses parallel `curl`; throughput includes shell/load
  generator overhead and is retained primarily for errors, CPU, and RSS.

Decision: keep Raw active as the replacement candidate, preserve Typesense as
production, and expand full-snapshot and human-judged relevance coverage next.
