# Schema 9 v1: additive accelerators with schema-7 parity

Status: design, 2026-09-28. Supersedes the first-switch scope in
[readiness-2026-09-27.md](readiness-2026-09-27.md); the full lazy-exact plan
in [`schema9.md`](../../../schema9.md) remains the long-term direction.

## Contract

Schema 9 v1 returns **byte-identical responses** to the schema-7 reader for
every request on the same corpus. Feature parity is not a separate goal to
test later: the schema-7 engine's response is the specification. Every fast
path runs inside the one existing engine and has a differential test against
the scan it replaces.

## Format

A schema-9 generation is a schema-7 generation plus two build-time sidecars.
Posting, position, document, and metadata files are unchanged, so every
existing decoder and its corruption checks still apply.

| File | Contents | Replaces |
| --- | --- | --- |
| `accelerators.bin` | For each text term with at least 4,096 postings: seek checkpoints every 64 postings (the runtime `SkipEntry`), and the maximum document-dependent score factor in every 128-document ID window that holds a posting | Lazy checkpoints built on first use; the exhaustive scan's need to score every match |
| `bitmaps.bin` | Dense transcript bitmaps for the highest-DF text terms within a 64 MiB budget, and title bitmaps over videos for terms at ≥0.1% of videos | Decoding the broadest posting lists to count matches |

Both are listed in the manifest with lengths and BLAKE3 digests like every
other file. The reader opens schemas 6, 7, and 9; schema 9 without its fast
paths enabled behaves exactly as schema 7.

## Query fast paths

Both apply only to the broad grouped relevance path (group by video, no
filter, no exclusions, more than 50,000 matches), which is where the scan cost
lies. Everything else keeps the schema-7 path.

1. **Exact totals.** `found` and `total_moments` come from the conjunction of
   per-term text-or-title sets and the transcript-provenance set, walked in
   document order through the existing `MomentCounter`. Bitmaps supply the
   broadest terms; other alternatives decode their postings into a set.
2. **Block-priority first pages.** Document windows are scored in descending
   bound order, where a window's bound is its maximum tier and the sum over
   term plans of each plan's window maximum times the term constants.
   Windows are scored with the existing conjunction visitor, seeking through
   checkpoints. Scoring stops when no unscored window can outrank the last
   group the page needs. The chosen groups are then completed from all of
   their documents, so their best document and top hits match the scan.

## Evidence already in hand

On the real 8.1M-document base, set totals equalled the engine on 11 queries
and block-priority returned the exact first 21 groups after scoring 0.17–16%
of matches (see
[P01 evidence](../../experiments/search/schema9/P01/exact-totals-and-block-priority-2026-09-27.md)).

## Rollout guards

- The reader advertises `raw-index-schema-9-v1`; worker preflight rejects any
  other build format (see
  [raw-operational-invariants.md](../raw-operational-invariants.md)).
- Generation activation stays warm-before-swap.
- A differential runner compares schema-9 and schema-7 responses for every
  production replay query before any canary or production switch.

## Deferred to later versions

Bit-packed posting blocks (about a third smaller), lazily verified integrity
units for a fast cold open, compact fast-field columns, sessions and cursors,
and Tier-1/2 summaries.
