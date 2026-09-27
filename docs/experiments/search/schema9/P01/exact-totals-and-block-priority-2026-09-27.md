# Exact totals from sets and block-priority grouped ranking (2026-09-27)

Two P01 forks decide whether schema 9 scales: whether exact grouped totals can
come from term sets instead of scoring every match, and whether grouped
ranking can prune scoring with block bounds. Both were measured on the real
8,142,650-document base generation (schema 6) with the engine's own term plans
(`conjunction_plans`, prefix expansion of the last token, text and title
fields) and checked against the current engine's responses.

Both diagnostics are ignored tests in
`crates/youtube_raw_search/src/engine/tests/exact_totals_sets.rs`; run them
with `RAW_TOTALS_INDEX=<generation>` as described in the file. Timings are
local, warm, single-threaded, and taken on a busy host; they are relative
evidence, not a Fly-reader gate.

## Exact totals

Each term plan becomes two document bitsets: transcript-text matches (every
text alternative's postings) and title matches (every document of a video
whose title matches). A query's match set is the AND over plans of
text-or-title; transcript provenance is the OR of the text sets. `found` is
the number of videos with a matched document, and `total_moments` feeds only
text-matched documents, in document order, to the engine's own
`MomentCounter` (20-second anchored reducer, minimum one per video).

All 11 queries returned **exactly** the engine's `found` and `total_moments`.

| Query | Matched documents | Videos | Moments | Set AND/OR | Totals walk |
| --- | ---: | ---: | ---: | ---: | ---: |
| `and the` | 3,096,743 | 32,454 | 1,490,698 | 3.0 ms | 36.2 ms |
| `the` | 5,581,443 | 32,685 | 1,970,430 | 0.8 ms | 53.7 ms |
| `in the` | 1,640,227 | 32,216 | 974,214 | 1.2 ms | 30.0 ms |
| `this` | 1,771,864 | 32,145 | 1,045,740 | 0.3 ms | 33.6 ms |
| `for the` | 1,050,217 | 31,684 | 673,482 | 1.4 ms | 21.9 ms |
| `you know` | 623,446 | 26,604 | 439,492 | 1.1 ms | 14.8 ms |
| `this is the` | 572,538 | 30,356 | 441,512 | 1.9 ms | 14.8 ms |
| `react` | 282,905 | 5,071 | 36,619 | 0.8 ms | 3.0 ms |
| `fire` | 65,924 | 6,715 | 17,571 | 0.4 ms | 1.8 ms |
| `the problem is that` | 22,647 | 9,017 | 20,809 | 1.5 ms | 1.3 ms |
| `this is the end` | 12,593 | 6,370 | 11,337 | 2.3 ms | 1.9 ms |

Set algebra over 8.1M documents costs at most 3 ms. The totals walk is linear in
**matched documents** (not postings) and reads 24-byte schema-6 metadata
records; the schema-9 fast fields (video and start time as compact columns)
and the reader's per-CPU partitions should cut it further. Even as measured,
the broadest query stays inside the plan's provisional 70 ms P05 budget for
exact totals. Building the sets from postings took 0.3–37 ms here; in schema 9
high-DF terms carry precomputed bitmaps, so that cost moves to build time.

## Block-priority grouped ranking

Every matched document's rank key (exact-token count, exact mask,
ordered-phrase flag, score) came from the engine's own conjunction scan, and
per-plan scores from single-plan scans. Documents are grouped in blocks of
128 IDs. A block's bound is its maximum tier and the **sum over plans of each
plan's block-maximum score**, which is what per-term block maxima stored in
`postings.blockmax` provide. Blocks are scored in descending bound order,
keeping each video's best document; scoring stops once no remaining block can
beat the 21st group (checked every 16 blocks). The chosen groups are then
finalized from all of their documents.

The pruned top 21 equalled the full ranking, including order and the
external-ID tie-break, for every query.

| Query | Matched documents | Blocks | Blocks scored | Documents scored | Fraction |
| --- | ---: | ---: | ---: | ---: | ---: |
| `and the` | 3,096,743 | 63,585 | 64 | 5,269 | 0.17% |
| `you know` | 623,446 | 59,067 | 90 | 3,255 | 0.52% |
| `in the` | 1,640,227 | 63,569 | 214 | 15,163 | 0.92% |
| `for the` | 1,050,217 | 63,435 | 450 | 29,713 | 2.8% |
| `this is the` | 572,538 | 62,291 | 1,459 | 42,340 | 7.4% |
| `this is the end` | 12,593 | 8,128 | 223 | 1,983 | 15.8% |

Common-word queries prune best because a few short transcript chunks dominate
BM25. Selective and sentence queries prune less, but their match sets are
small already.

## Corpus-wide posting size in the proposed layout

`measure_proposed_schema9_posting_bytes` transcodes every posting list of the
same generation: lists of at most 128 postings stay delta varints; longer lists
use 128-document blocks of bit-packed deltas and (frequency − 1) with an
8-byte block header, an 8-byte skip entry, and a 4-byte block-max bound.

| Field | Schema-6 postings | Schema-9 blocks | Skips | Block-max | Schema-9 total | Bitmaps (64 MiB text budget) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Transcript | 311,695,403 | 194,332,211 | 8,812,232 | 4,406,116 | 207,550,559 | 65 terms (DF ≥ 394,515), 66,159,080 |
| Title | 683,017 | 561,079 | 10,224 | 5,112 | 576,415 | 1,185 terms (DF ≥ 0.1%), 4,862,055 |

Transcript postings shrink by a third **while gaining** the skip and bound
data that ranking pruning needs. Sizes cover document IDs and frequencies
only; first and remaining positions, fast fields, and payloads are separate.

## Limits of this evidence

- The tier bound uses each block's true maximum tier. Per-term exact flags
  give the same or a looser bound; the score bound is already per-term.
- Ranking keys exclude proximity and ordered-phrase evidence, as the engine's
  broad grouped path does above 50,000 matches. The engine applies that
  evidence at or below 50,000 matches, so the `this is the end` row (12,593)
  shows pruning without it, not the engine's live order. Phrase-aware ranking
  multiplies scores after matching, so its bounds must include the maximum
  proximity and phrase multipliers.
- Relevance sort only. Engagement and date sorts need their own bounds
  (fast-field maxima per block).
- Pages beyond the first need the retained frontier (P06); this measures the
  first 21 groups.
- Local warm timing on schema-6 files. The 2 GiB/4-vCPU Fly reader, cold
  pages, and schema-9 file layouts remain P10 evidence.

## What this settles for the plan

- **Exact totals:** exact `found` and `total_moments` from sets plus per-document
  fields are correct, and cost is linear in matches with a small constant.
  Keep bitmaps for high-DF terms and fast fields for video and start time.
- **Ranking fork:** block-priority with per-term block-max score bounds prunes
  scoring to 0.2–16% of matches with exact results. Prefer it for grouped
  relevance ranking, and store per-block score maxima.
