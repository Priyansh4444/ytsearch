# Recent Raw search latency (Axiom, read-only)

Observed on 2026-09-22 with `WINDOW=7d sh scripts/axiom-search-report.sh`
from the repository root, using the existing operator query credential. The
window contained 273 `search_query` events for `raw`: average 294.4 ms, median
8.9 ms, p95 695.7 ms. A query for `duration_ms > 250` returned 30 events.
The last 24 hours contained no search events in this dataset. This is Axiom
API telemetry, not a Fly Grafana machine-resource trace. The same event schema
does not prove CPU, disk, memory, cache state, or concurrent request load.

All 30 slow events reported one backend call, and `backend_search_ms` accounted
for all but at most 1 ms of `duration_ms`; `result_processing_ms` was 0 ms.
This localizes the observed delay to the Raw backend call, rather than API
result processing. It does not establish which part of the Raw engine was slow.

| UTC time | Query | Total / backend (ms) | Note |
| --- | --- | ---: | --- |
| Sep 19 23:16:38 | `the world is ending` | 42,283 / 42,283 | Largest outlier, 16 seconds after post-OOM health passed |
| Sep 20 13:03:02 | `clearly the world is ending` | 5,141 / 5,141 | Same query took 2,524 ms on Sep 19 |
| Sep 19 23:12:42 | `fable on high` | 1,924 / 1,924 | Previously only creator-scoped in benchmark matrix |
| Sep 19 23:12:29 | `and the` | 3,213 / 3,212 | High-frequency words |
| Sep 19 23:12:22 | `in the` | 1,791 / 1,790 | High-frequency words |
| Sep 19 23:12:09 | `the` | 1,223 / 1,222 | High-frequency single word |
| Sep 21 11:57:17 | `this` | 719 / 718 | Later single-word request |
| Sep 17 10:23:49 | `How I code with AI right now` | 421 / 421 | Long title-like query |

The remaining slow events include common-term and compact/spaced queries
already in `scripts/benchmark-search-cases.tsv`, plus `this is not real`,
`showered in fire`, `the beginning of hte end`, `this is the end`, and
`how I code with AI`. The Sep 19 23:11–23:16 cluster substantially overlaps
the existing benchmark matrix and may be a test run; telemetry alone does not
identify its initiator. Query strings here are generic and contain no client
metadata. The complete operator-only query output was not committed.

The Fly reader Machine event history reports an OOM kill at
2026-09-19T23:14:41Z, restart at 23:14:43Z, and a passing health check at
23:16:22Z. The 42.3-second request began at 23:16:38Z, soon after that
restart. Cold cache after restart is a plausible contributor. The record does
not prove that this query caused the OOM, whether it waited on disk, or why
the restart was slow. A cold first-query and concurrent memory trace remains
required. The Machine still showed the legacy IPv4 bind and schema-6 image
when inspected on 2026-09-22.

Ten previously missing global query shapes were added to the benchmark matrix.
The four most useful new cases are also mandatory in the performance contract.
The existing cases for `the`, `in the`, `and the`, `there are`, `you know`,
`this that`, and other slow common terms remain. Future release measurements
must run these cases against the exact candidate generation, separate cold and
warm runs, record resource counters, and apply the stricter 200 ms schema-9
contract. This observation does not pass that contract or explain the 42-second
outlier's internal cause.
