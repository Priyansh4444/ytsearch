# Search backend experiments

The search experiments test whether a smaller read backend can satisfy the
existing Typesense-shaped contract without changing `youtube_api` or adding
browser code.

| Backend   | Implementation       | Status                           |
| --------- | -------------------- | -------------------------------- |
| Typesense | Production control   | Supported production backend     |
| Tantivy   | Archived branch      | Retired trial; machine destroyed |
| Raw       | `youtube_raw_search` | Active experiment                |

Raw uses `youtube_search_service` for HTTP routing, authentication, metrics,
and the bounded response cache. Index layout, query evaluation, scoring, and
hydration remain engine-owned. Tantivy's final source is preserved on
`archive/tantivy-experiment` at `3aa3f55`; it is not compiled by the active
workspace.

Read [common-results.md](common-results.md) first. It is the canonical result
matrix. [raw-validation.md](raw-validation.md) contains the current Veritasium,
Vsauce, sorting, concurrency, and failure-mode evidence.
[methodology.md](methodology.md) defines how to reproduce the measurements.
[typesense-compatibility.md](typesense-compatibility.md) traces the active
Typesense 30.2 typo, prefix, exact-priority, position, and token-drop behavior;
records what Raw adopted; and names the remaining parity gaps.
[schema-v6-positions.md](schema-v6-positions.md) records the complete-position
layout, integrity contract, schema-v5 rollback, and measured non-phrase gate.
[quoted-phrase-search.md](quoted-phrase-search.md) records the exact quoted
grammar, streaming position reader, correctness cases, and phrase benchmark.
[proximity-ranking.md](proximity-ranking.md) records the Typesense-compatible
window, rejected full-corpus pass, candidate-targeted fast path, and A/B gate.
[negative-term-search.md](negative-term-search.md) records exact token-start
exclusion semantics, malformed-query rejection, and its ordinary-query fast
path/performance gate.
[engagement-ranking.md](engagement-ranking.md)
records the views/likes/comments data path, bounded score, deployed benchmarks,
and isolated frontend topology.
