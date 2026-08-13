# Search experiment methodology

## Contract

Workload: grouped transcript search over eight representative queries, with 10
warmups and 50 measured requests per query. Requests are sequential so a single
search is the unit of work.

Invariant: backend choice may not change creator filtering, date sorting,
grouping, pagination, typo fallback, highlights, channel listing, empty-result
behavior, authentication, or the JSON consumed by `youtube_api`.

Primary metric: pooled warm p95 HTTP latency on localhost. Secondary metrics
are p50 and mean latency, process CPU ticks, warm RSS, index bytes, build/import
time, relevance overlap, startup behavior, and cached response cost.

Historical rollback during this experiment kept production on Typesense. Raw
was later promoted; Typesense now remains stopped and preserved as the rollback
and differential oracle. The experiment apps exposed no write API and were
isolated from production data.

## Corpus

The full sample contains 372,860 transcript chunks from 1,956 videos across
`@t3dotgg` and `@computerphile`. It was exported from a stopped snapshot clone,
not the production Typesense machine.

The original “first N videos encountered” slices were rejected because they
overrepresented long, topic-dense videos. The retained 500- and 1,000-video
slices select the lowest stable FNV-1a hashes of all video IDs. They are nested,
deterministic, input-order independent, and closer to the full chunk-per-video
distribution.

| Videos | Chunks | Source bytes |
| ---: | ---: | ---: |
| 500 | 88,408 | 66,745,336 |
| 1,000 | 184,375 | 139,595,245 |
| 1,956 | 372,860 | 280,583,117 |

The follow-up correctness corpus contains 47,860 chunks from 787 Veritasium
and Vsauce videos. It is separate from the scaling corpus so ranking fixes do
not rewrite the retained backend comparison. See
[raw-validation.md](raw-validation.md).

Generate the slices with:

```sh
cargo run --release -p youtube_raw_search -- slice \
  .search-experiment/corpus/chunks.jsonl \
  .search-experiment/scaling/stratified-500.jsonl 500
```

## Local benchmark controls

- Linux host, loopback HTTP, optimized Rust binaries.
- Only the measured engine and load generator run during a retained run.
- Typesense 30.2 runs with one CPU and a 512 MB container limit.
- Raw and the retired Tantivy control received sequential requests, so each
  used one active core.
- Ten warmup requests per query occur before resource counters are sampled.
- Each retained row contains 400 uncached searches: eight queries × 50 runs.
- CPU is `/proc/<pid>/stat` user plus system ticks; 100 ticks equal one second
  on this host.
- RSS is sampled after warmup and the repeated workload.
- Times include request parsing, search, hydration, and JSON serialization.

The active harness is `scripts/benchmark-search-engine.sh`. The historical
Tantivy-named harness is retained only on `archive/tantivy-experiment`.
The query lists are checked in separately so the original eight-query workload
can be compared against the expanded matrix without filtering after timing.
The fuzzy-v2 follow-up adds `scripts/benchmark-search-queries-typos.txt` and
`scripts/benchmark-search-typo-cases.tsv`. It uses the same 47,860-document
Veritasium/Vsauce JSONL in a dedicated local Typesense 30.2 container and Raw
index; neither backend receives a response cache. The container was not CPU or
memory limited, so latency comparisons favor Typesense's ability to use the
host while resource rows report observed process use.

`scripts/check-raw-search-engine.sh` verifies creator scope, global search,
date order, pagination, authentication, channel lookup, compatibility/raw route
equality, and `blackhloe`/`blakhole`/`blackhle` top-result recovery against a
running Raw service.

## Relevance

Typesense is the automated oracle for top-one agreement and overlap at 10. This
is only a regression signal: disagreements are manually judged because an
existing backend can rank irrelevant partial-token matches above exact intent.

Promotion requires at least 80% top-one agreement, mean overlap at 10 of at
least 7, and no important manually judged regression. Four cases are enough to
find failures, not enough to prove relevance parity.

## Known gaps

- The sample is about 2,000 videos, not the 7.16-million-chunk snapshot.
- Local runs are single-host measurements, not confidence intervals across
  several hardware classes.
- The raw mmap safety contract is unit-tested for bounds and corrupt headers,
  but Miri does not execute OS memory mappings.
- Public Fly latency is network dominated and is reported separately.
- Parallel `curl` concurrency runs are suitable for error/resource plateaus;
  their throughput includes load-generator process overhead.
