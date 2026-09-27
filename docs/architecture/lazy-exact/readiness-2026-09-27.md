# Schema 9 production readiness (2026-09-27)

Status: assessment. Schema 9 is **not** production-ready. This maps the
[execution plan](../../../schema9.md) to the code and evidence on `master` and
names the shortest path to a first production switch.

## Where production stands without schema 9

The schema-6/7 reader now meets the latency target on production traffic. On
2026-09-27, after partitioned scans, lazy posting checkpoints, warm-before-swap
publication, and a four-vCPU/2.5 GiB reader, the canonical matrix and sentence
probes measured a 181 ms maximum with no request above 200 ms. Its cost still
grows linearly with the postings of a query's words, every publication is a
full rebuild, and exact totals come from scoring every match. Schema 9 is the
answer to corpus growth and to the new exact summary features, not to today's
latency.

## Phase map

| Phase | Plan outcome | On `master` | Remaining |
| --- | --- | --- | --- |
| P00 | Frozen contract, 300-query matrix, exhaustive oracle | Matrix, oracle foundation, count-only and ranking references, production evidence | 14 `decision_pending` matrix cases, fuzzy/split evidence factors, creator reconciliation, independent verdict |
| P01 | Structural and ranking forks decided with evidence | Synthetic set timing, active DF/overlap surveys, posting-ID codec comparison, **exact totals from sets and block-priority ranking on the real corpus** | Codec and block-size choice, bitmap DF threshold and byte budget, phrase bounds, Tier-1 histograms, session model, Fly-reader repeats |
| P02 | Exact sets and seekable postings | Typed `DocSet`/`VideoSet` with dense and sparse paths; delta-varint posting cursor prototype | Final codec behind the chosen P01 fork |
| P03 | Build, close, and reopen a structural generation | Directory/footer with lazy BLAKE3 units, finalizer, source spool and audit, video and document fast-field writers | Writers for terms, postings, skips, block-max bounds, bitmaps, positions, facets, payloads, and channels; the builder pipeline; reopen; ten live lanes |
| P04 | Exact predicate execution with provenance | — | Typed query AST, set/skip execution, phrase verification, match spans |
| P05 | Exact totals and summaries | Totals approach proven (see P01) | Tier 0/1/2 summaries, fast-field walk, budgets |
| P06 | Exact pages from a retained frontier | First-page block-priority pruning proven (see P01) | Frontier across pages, phrase-aware bounds, non-relevance sorts, differential against the reference |
| P07 | Pinned sessions through compatible APIs | Existing reader retains pages across activation | Session store, cursors, expiry, generation pinning |
| P08 | Browser and lazy overview | — | Frontend for exact overview and cursors |
| P09 | Publication and recovery on schema 9 | Schema-7 publication, receipts, resume, warm-before-swap | Schema-9 build in the worker or builder, transfer, activation |
| P10 | Full-generation qualification on the reader | Canary tooling and production replay | Full canary at one release identity |
| P11 | Verified live switch | Rollout procedure | The switch and its receipts |

## What the new evidence settles

[`P01/exact-totals-and-block-priority-2026-09-27.md`](../../experiments/search/schema9/P01/exact-totals-and-block-priority-2026-09-27.md)
shows, on the real 8.1M-document base and against the engine's own responses:

- Exact `found` and `total_moments` from term bitsets plus per-document video
  and start time equal the engine on all 11 canonical queries. Set algebra is
  at most 3 ms. The walk is linear in matches (36 ms for `and the`).
- Block-priority first-page ranking with per-term block-max score bounds
  returns the exact top 21 groups after scoring 0.17–16% of matches.

So schema 9's two central bets hold: exact totals do not require scoring, and
grouped relevance ranking can stop early. `broad-single.bin` materialization
is unnecessary, as the plan already assumes.

## Shortest path to a first production switch

A first switch can serve today's API contract (grouped relevance and the
existing sorts, exact `found`/`total_moments`, page-number paging) before the
new summary and session features:

1. **Finish P01's format decisions** that P03 needs: posting codec and block
   size (the codec survey found bit-packed blocks about 40% smaller than delta
   varints for high-DF terms at similar seek cost, but did not choose), block-max
   bound layout, bitmap DF threshold and byte budget.
2. **P03 writers and reopen** for the remaining nine files, reusing the source
   spool, document and video writers, and the directory finalizer; run the ten
   live lanes.
3. **P04 predicate execution** over schema-9 files, differentially tested
   against the current engine and the P00 oracle.
4. **P05 totals** (not Tier 1/2) and **P06 first-page plus page-N ranking**,
   recomputing pages without a retained session as today's reader does.
5. **P09 publication** of schema-9 generations from the worker, then **P10**
   qualification and **P11** switch behind the existing API.

Tier-1/2 summaries, pinned sessions and cursors (P07), and the lazy overview
(P08) can follow the switch without changing the index format, provided P03
already writes the facets and fast fields they read.

## Proposed P01 format decisions

These follow from the evidence above and the P01 surveys; P01 must still
confirm them on the reader profile before P03 freezes the format.

- **Bitmaps.** Dense transcript bitmaps for the highest-DF terms inside a
  64 MiB payload budget: about 63 terms at this corpus size, roughly DF ≥ 5%.
  The 28 terms above 10% DF need 29.5 MB and are where skip intersection costs
  most (about 2.2 ms per million documents at 10% DF, synthetic). Terms between
  1% and 5% use skip leapfrog. Title bitmaps are 4.4 KB each, so every title
  term at ≥0.1% DF (1,149 terms, about 5 MB) fits. Exactness never depends on
  a bitmap existing.
- **Blocks.** 128-document posting blocks, so block-max bounds, skips, and
  codec blocks share one boundary. Bit-packed deltas for lists longer than one
  block (about 40% fewer bytes than delta varints for the six broadest terms at
  similar seek cost); delta varints for shorter lists, where they were smallest.
  Corpus-wide, this layout stores transcript postings in 207.6 MB including
  skips and block-max bounds, against 311.7 MB of schema-6 postings without
  them; the 64 MiB budget holds 65 bitmaps (DF ≥ 4.8%).
- **Block-max bounds.** Per block, the maximum of the document-dependent score
  factor (BM25 term-frequency/length part times the first-position multiplier),
  4 bytes each (about 3% of a 128-posting block). Query time multiplies by the
  term's IDF, field weight, and alternative multiplier, and sums across terms,
  as the ranking simulation did.

## Risks

- The P00 contract still has open decisions; P04 and P06 differential tests
  need them closed before exactness can be claimed.
- Publication rebuilds the full generation; the schema-9 build must fit the
  worker or the performance builder profile and its deadline.
- Two generations must fit the reader's page cache during publication;
  schema 9's smaller hot set helps, but it must be measured (P10).
