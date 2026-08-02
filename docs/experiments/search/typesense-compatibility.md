# Typesense compatibility audit

Measured 2026-08-01 against Typesense 30.2 commit
[`d45d46b`](https://github.com/typesense/typesense/tree/d45d46baf3996d1de8bf96a87f375cfb43691560).
This audit covers the exact `text,title` configuration used by `youtube_typesense`,
not every Typesense feature.

## Active search contract

The application sends:

- `query_by=text,title` with weights `4,6`
- `text_match_type=sum_score`
- `prioritize_exact_match=true`
- `prioritize_token_position=true`
- `typo_tokens_threshold=5`
- `split_join_tokens=always`
- grouping by `video_id`, creator/video filters, and optional publication-date sorts

Parameters left at Typesense defaults include prefix search, two allowed typos,
the four-candidate limit, one-typo minimum length 4, two-typo minimum length 7,
and `drop_tokens_threshold=1`. The collection has no synonym sets, stemmer,
locale, infix fields, token separators, symbols-to-index, or default sorting
field. Those unconfigured capabilities are not Raw promotion requirements.

## What Typesense implements

Typesense stores each searchable field in an adaptive radix tree. Fuzzy lookup
walks that tree while incrementally updating optimal-string-alignment
Damerau-Levenshtein rows, including adjacent transpositions, and prunes paths
whose cost can no longer fit the requested distance. It ranks the surviving
leaves by posting frequency when no default sorting field is configured and
retains at most `max_candidates` terms.

The query evaluator tries edit-cost combinations progressively. It begins at
zero edits, expands toward one and two edits only while the result count is
below `typo_tokens_threshold`, and caches term/cost expansions within the
request. Token length and numerical/alphanumerical guards bound the maximum
cost. Split/join resolution separately tries whole-query joins, adjacent joins,
and two-part splits that have common postings.

Primary source locations:

- [Damerau-Levenshtein row update and pruned ART traversal](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/src/art.cpp#L1412)
- [Progressive fuzzy candidate combinations and per-request term cache](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/src/index.cpp#L4831)
- [Length-based typo-cost guard](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/src/index.cpp#L6897)
- [Space-as-typo split/join resolver](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/src/index.cpp#L7955)
- [Typesense 30 search parameter contract](https://typesense.org/docs/30.0/api/search.html)

## What Raw adopted

Raw keeps its immutable mmap representation and adds `fuzzy.bin`, a compact
20-byte-node breadth-first trie. It is not loaded into a process-owned map.
The exact path remains a binary lookup in `lexicon.bin`; the trie is consulted
only for guarded prefix/typo fallback.

| Behavior | Raw implementation |
| --- | --- |
| Adjacent transpositions | Optimal-string-alignment Damerau-Levenshtein rows |
| One/two typo gates | 4-byte and 7-byte minimums, matching active Typesense defaults |
| Candidate cap | Four terms per edit distance, ranked by field-weighted posting frequency |
| Progressive expansion | Exact/split, then prefix, then distance one, then distance two; stop at five grouped results |
| Prefix search | Guarded fallback when exact/split coverage is below five |
| Exact priority | Lower-distance stages complete first; prefix/fuzzy scores are demoted |
| Compact split | Same-chunk intersection with narrow plural recovery |
| Typo plus split | Additional bounded fallback requiring one exact component and one one-edit correction |
| Highlighting | Uses the actual corrected lexicon terms, not the misspelled input |

The typo-plus-split fallback intentionally improves on the control for this
domain. Typesense returns zero for `blackhloe` and `blackhle` because
`blackhole` is not an indexed token in the real corpus. Raw composes correction
with `black + hole`, requires both terms in one chunk, refuses shorter fuzzy
components such as `black + he`, and returns the same top result as
`blackhole`.

## What Raw has not copied

| Typesense behavior | Status | Reason |
| --- | --- | --- |
| Full positional scoring | Gap | Raw does not yet store token positions; adding them needs a disk/latency relevance experiment |
| Exhaustive cost combinations | Deliberately bounded | The public query uses default non-exhaustive search |
| General token dropping | Approximate | Raw's multi-token fallback is simpler and needs differential fixtures before promotion |
| Arbitrary filter language/faceting | Out of scope | The application uses creator/video equality filters only |
| Dynamic writes and replication | Out of scope | Raw is a read-only serving index built offline |
| Locale tokenizers/stemming/synonyms/infix | Not configured | The active collection does not use them |

Positional relevance and multi-token dropping are the important remaining
search-contract gaps. They should be solved with a judged query set and a
compact positions representation, not by importing general database machinery.

## Same-corpus results

Corpus: 47,860 transcript chunks, 787 Veritasium/Vsauce videos, 29,416 terms.
Both engines used the same JSONL and Typesense 30.2 query parameters.

| Workload | Backend | p50 | p95 | Mean |
| --- | --- | ---: | ---: | ---: |
| Nine core queries, 450 searches | Raw fuzzy v2 | 1.18 ms | 2.09 ms | 1.27 ms |
| Nine core queries, 450 searches | Typesense 30.2 | 4.54 ms | 7.26 ms | 4.90 ms |
| Fourteen typo/prefix queries, 700 searches | Raw fuzzy v2 | 1.83 ms | 3.14 ms | 1.94 ms |
| Fourteen typo/prefix queries, 700 searches | Typesense 30.2 | 3.80 ms | 9.12 ms | 3.89 ms |

| Resource | Raw fuzzy v2 | Typesense 30.2 |
| --- | ---: | ---: |
| Warm RSS observed | about 25 MB | 474–496 MB |
| Index/data directory | 10.54 MB | 107.31 MB |
| Build/import | 304 ms | 2.69 s |
| Typo workload CPU per search | 0.164 ticks | 0.389 ticks |

The Raw v2 trie adds 1.57 MB, or 17.4%, to the prior Raw index. In the adjacent
old/new core comparison, p50 moved from 1.04 to 1.18 ms, mean from 1.19 to
1.27 ms, p95 improved from 2.26 to 2.09 ms, and CPU moved from 46 to 50 ticks
over 450 searches. On the typo matrix the old/new p50 and mean were effectively
flat, p95 moved from 2.99 to 3.14 ms, and CPU improved from 120 to 115 ticks.
The retained tail cost buys transpositions, two edits, prefix search, corrected
highlights, and the black-hole composed fallback.

## Why Raw is faster and smaller

These are architectural causes supported by the index layouts and measured
work, not a claim that Rust is inherently faster than C++.

1. Raw is a read-only domain index. It has no Raft state, write path, RocksDB
   document database, schema alteration, faceting, joins, vectors, analytics,
   or general filter evaluator in its query process.
2. Raw indexes only the fields this application searches. Typesense retains
   the general 20-field collection and its database metadata.
3. Titles are indexed once per video in Raw and expanded through compact video
   membership. A flat per-chunk index repeats title postings for every chunk.
4. Delta-varint postings, fixed-width metadata, and the fuzzy trie are mmapped.
   Unused pages remain under kernel control instead of being reconstructed as
   a large process-owned object graph at startup.
5. Raw decodes only selected posting lists and materializes JSON only for the
   returned groups. Zstd document blocks are request-local and lazy.
6. Exact/split, prefix, one-edit, and two-edit work are separate stages with an
   explicit grouped-result threshold. Common exact searches avoid fuzzy work.
7. The supported filter and sort shapes are direct metadata checks rather than
   a dynamic query language.

The same specialization also explains the remaining gaps. Raw cannot claim a
complete replacement until positional relevance, token dropping, full-snapshot
capacity, and a larger human-judged query suite pass.

## Promotion definition

A complete win requires all of the following on a production-shaped snapshot:

- no important judged relevance regression, including typos, prefixes,
  compact/spaced forms, creator filters, and multi-token queries;
- core and typo p95 at least 25% below Typesense;
- CPU at least 25% below Typesense;
- RSS at least 40% below Typesense with a bounded concurrency plateau;
- smaller on-disk index and faster offline build;
- debug/release, corruption, concurrency, API-contract, and rollback checks;
- shadow traffic before any production switch.

Current decision: retain fuzzy v2 and continue toward positional and
multi-token parity. Do not replace production Typesense yet.
