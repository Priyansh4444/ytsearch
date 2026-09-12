# Lazy exact Raw engine: arena synthesis

Status: design verdict. This does not authorize implementation, deployment, activation, or deletion of the retained schema-8 generation.

## Decision

Use the Lucene/PISA-style candidate as the base. It fits the current engine's immutable generations, mmap-backed delta postings, cold positional streams, document blocks, dense generation-local IDs, and atomic reload boundary. It replaces schema 8's query-plan answers with structural term, block, document, video, facet, and checksum metadata.

Graft two bounded ideas:

1. From the Vespa-style candidate, select field-specific high-document-frequency occurrence bitmaps deterministically under a manifest-recorded byte budget. Sparse score-bearing postings remain canonical and the engine must be exact with an empty bitmap pool.
2. From the Roaring-style candidate, use adaptive frozen Roaring for derived match/facet sets only when measured size wins, and distinguish `DocSet` from `VideoSet` in the type system.

## Rejections

- No query-plan-keyed files, durable result pages, or `broad-single.bin` correctness dependency.
- No hard-coded stop words or named-query paths.
- No candidate caps that drop valid prefix, typo, phrase, or token-fallback alternatives.
- No default approximations, fixed moment buckets, or partial facets presented as complete.
- No BM25-only block bounds. Bounds cover the entire stable video-ranking tuple or disable pruning.
- No stateless cursor that silently reruns WAND, changes generations, or claims no-rescan behavior.
- No all-term Roaring primary representation until corpus measurements prove disk, fault, RSS, and score-stream costs.
- No popularity-based document reorder; preserve video/timestamp locality unless a corpus fork proves a better exact ordering.

## Two semantic constraints

`total_moments` is not posting cardinality. Current behavior greedily coalesces query matches within a 20-second window anchored at the first match per video. Exact totals therefore require a query-specific ordered scan of matched `(video_id, t_start_ms)` fast fields. A static bitmap, fixed bucket, or transitive grouping changes the contract. This scan is a decisive prototype gate for broad queries.

Likewise, an exact relevance boundary is insufficient to resume arbitrary grouped ranking without work already performed. A live query session must retain its generation lease and branch-and-bound frontier. The opaque cursor authenticates the session, generation, query fingerprint, sequence, and expiry. If the session expires, the API returns an explicit refresh-required error; it never silently rescans or switches snapshots.

## Rubric verdict

| Criterion | Vespa hybrid | Lucene/PISA | Roaring primary |
| --- | ---: | ---: | ---: |
| Exact predicate, totals, scores, pages | 4.5 | **4.8** | 4.4 |
| No recall loss | 5.0 | **5.0** | 5.0 |
| Lazy paging without prior-work rescan | 4.3 | **4.8** | 4.2 |
| Exact Tier-0/Tier-1 feasibility | 4.1 | **4.3** | 4.2 |
| No precomputed answers | 5.0 | **5.0** | 5.0 |
| Footer-only cold open | 4.7 | **4.8** | 4.7 |
| 2 GB reader fit | 4.3 | **4.5** | 3.9 |
| Generation pinning | 4.7 | **4.9** | 4.6 |
| Migration fit | 4.0 | **4.8** | 3.7 |
| Website/API compatibility | 4.4 | **4.7** | 4.4 |

These are design plausibility scores, not benchmark evidence. Acceptance is conditional on the prototype gates.

## Required prototype forks

1. Phrase candidates: bitmap intersection versus rare-lead skip leapfrog, both followed by exact positions.
2. Counting: dense/frozen-Roaring algebra versus skip merge across measured DF bands.
3. Exact moments: dense set-bit iteration versus sparse iteration over bitpacked video/timestamp columns.
4. Tier-1 histograms: scalar set-bit gathers versus batched/SIMD fast-field reads.
5. Grouped ranking: BMW pivot traversal versus a block-priority resumable frontier with complete bounds.
6. Posting codec: current delta varints plus skips versus SIMD-BP128/Stream VByte at 64/128/256 block sizes.
7. Sessions: in-memory lease/frontier with honest expiry; durable checkpoints only if restart evidence justifies them.

The first feature branch is a throwaway corpus-shaped measurement harness. It must not freeze schema 9, activate a generation, or mutate production.

## Release gates

- A 300-query exhaustive differential oracle proves membership, score bits, every page, totals, 20-second moments, highlights, fallback modes, filters, sorts, and Tier-0/1 aggregates.
- A generation-advance test proves every page, summary, and video expansion in a session stays on one immutable generation.
- Instrumentation proves page N+1 does not reopen exhausted blocks or rescore earlier documents.
- Cold open physically reads under 200 MB and non-phrase queries read no remaining-position units.
- On the 2 GB profile, exact totals, Tier 0, and page one are all under 200 ms for the worst canonical query; RSS is below 1.5 GiB with zero swap/OOM.
- Artifact shape depends only on corpus structural facts, never benchmark or observed query-plan count.
- Every current website behavior remains covered, including initial JavaScript below 30 KB gzip.
