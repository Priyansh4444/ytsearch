# Raw exact negative-term search

Measured 2026-08-08 on the 47,860-chunk, 787-video schema-v6 snapshot. This
slice adds the exact token exclusion needed by the active search grammar while
keeping ordinary queries on their existing path.

## Contract

- `-term` excludes exact matches for `term` in the active `query_by` fields.
- The marker is an operator only at the start of the query or immediately after
  whitespace. `x-ray` therefore remains the two positive tokens `x` and `ray`.
- Multiple negative terms are ORed: a candidate matching any one is removed.
- Text exclusions remove matching chunks. A title exclusion expands through the
  existing title posting path and removes every chunk belonging to that video.
- Negative terms do not receive prefix expansion, typo correction, splitting,
  or token dropping.
- Negative-only and `* -term` queries use wildcard search and exclude matching
  chunks before grouping. A separated or repeated marker and a negative quoted
  phrase fail closed with HTTP 400. Negative phrases remain a separate TODO.

Typesense first parses excluded tokens/phrases separately, unions their exact
posting matches, then passes the excluded IDs into phrase and ordinary search.
See its [query token parser](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/src/collection.cpp#L4735-L4836),
[exact exclusion posting intersection](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/src/index.cpp#L6243-L6290),
and [pre-search excluded-ID preparation](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/src/index.cpp#L3580-L3620).
Raw adopts that exact-posting principle but deliberately rejects negative
phrases until their complete grouped-search semantics exist.

## Implementation

The parser keeps positive and excluded tokens in separate, request-bounded
vectors. Search decodes each excluded term through the existing title/text
posting readers, merges the already sorted document IDs, then filters candidate
vectors with one forward cursor. No corpus bitset, hash set, or persistent index
structure is added.

Exclusion happens before every result-count decision: strict intersection,
prefix and typo expansion, split fallback, and token dropping. This prevents an
excluded hit from satisfying `typo_tokens_threshold` or
`drop_tokens_threshold`, and prevents a later fallback from reintroducing it.
When the query has no negative terms, construction returns an empty vector and
the filtering helper exits before scanning candidates.

Correctness fixtures cover quoted phrases, title expansion, prefix correction,
one-edit typo correction, token dropping, exact-only negative terms,
negative-only wildcard search, malformed markers, and hyphenated positive
words.

## Performance gate

The release binary used one loopback client, 10 warmups per query, and 50 timed
runs per query. Two rounds alternated the immediate parent and candidate over
the unchanged nine-query workload, for 900 searches per variant:

| Variant | Median | p95 | Mean | Internal mean | CPU ticks |
| --- | ---: | ---: | ---: | ---: | ---: |
| Immediate parent | 4.188 ms | 7.929 ms | 4.483 ms | 3.337 ms | 353 |
| Exact-negative candidate | 4.333 ms | 7.388 ms | 4.501 ms | 3.344 ms | 355 |

The ordinary-query mean changed by +0.4%, internal mean by +0.2%, and CPU by
+0.6%; p95 decreased by 6.8%. These small mixed movements are treated as noise,
not a speedup. The no-negative fast-path passes because no meaningful ordinary
query regression was measured.

A separate six-query matched-exclusion workload ran 300 searches. Its three
positive controls measured 3.631/5.029/3.850 ms median/p95/mean; the paired
negative variants measured 3.518/5.075/3.690 ms. The entire run used 96 CPU
ticks, averaged 2.560 ms inside the engine, reported no errors, and kept the
same 12,261,425-byte index: this feature adds no on-disk or steady-state index
memory.

Reproduce the matched path with `scripts/benchmark-negative-queries.txt` and
`scripts/benchmark-search-engine.sh`. Promotion still depends on the larger
human-judged and Typesense differential oracle.
