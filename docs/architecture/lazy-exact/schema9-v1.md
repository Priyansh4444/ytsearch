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

A schema-9 generation is a schema-7 generation plus three build-time sidecars.
Posting, position, document, and metadata files are unchanged, so every
existing decoder and its corruption checks still apply.

| File | Contents | Replaces |
| --- | --- | --- |
| `accelerators.bin` | For each text term with at least 4,096 postings, per 128-posting block: the block's last document, its maximum document-dependent score factor (BM25 frequency/length part times the first-position multiplier), and the cursor state after the block (the runtime `SkipEntry`) | Lazy checkpoints built on first use; the exhaustive scan's need to score every match |
| `bitmaps.bin` | Dense transcript bitmaps for the highest-DF text terms within a 64 MiB budget, and title bitmaps over videos for terms at ≥0.1% of videos | Decoding the broadest posting lists to count matches |
| `plan-bounds.bin` | For each lexicon term whose last-token plan is broad (≥100,000 estimated documents) and has several alternatives: the alternatives it covers and the plan's true maximum score per 512-document window, computed with the scan and rounded up to one byte | Summed per-alternative maxima, which overstate a prefix plan's score several times and disable pruning |

Bounds are per posting block rather than per document window so that bounds
and seek checkpoints share one boundary and the file stays small.

All three are listed in the manifest with lengths and BLAKE3 digests like
every other file. The reader opens schemas 6, 7, and 9; schema 9 without its fast
paths enabled behaves exactly as schema 7.

## Query fast paths (steps 2 and 3, landed)

The fast path serves the broad grouped paths (group by video, no filter, no
exclusions, every term plan estimated at 100,000 or more documents), which is
where the scan cost lies. Multi-term searches take it only when the
conjunction is `Complete` (more than 50,000 matched documents and enough
groups); every other disposition, the engagement sort, and relevance queries
without tight bounds keep the schema-7 scan. Code: `engine/broad_fast.rs`.

1. **Exact totals.** Each plan's text set (bitmaps for the broadest terms,
   decoded postings otherwise) and title set (the documents of matching
   videos) are built in parallel. `found`, `total_moments`, and the matched
   document count come from walking their conjunction across the scan
   partitions with the engine's `MomentCounter`.
2. **Relevance pages by window priority.** Each 512-document window has a
   rank bound: the exact mask its exact alternatives allow, and the sum over
   plans of the plan's window maximum (a single-alternative plan from its
   block maxima and title scores, a prefix plan from `plan-bounds.bin`). The
   64 highest-bound windows are scored first; then each partition scores its
   remaining windows in document order, seeking through the stored
   checkpoints and skipping any window whose bound cannot reach the
   `required`-th best group so far. Partitions hold disjoint videos, so each
   partition's cut-off may prune the others.
3. **Field-sort pages.** Newest, oldest, views, likes, and comments pages come
   from the matched-video set: the leading videos by the field and every
   video tied with the last one.
4. **Completion.** The page's groups are scored from all of their documents
   with the scan's own streams, so scores, hits, and moment counts are the
   scan's, and the page is formatted by the scan's formatter.

The exactness argument is in the module documentation: a group's partial
rank never exceeds its true rank, so a skipped window can neither change a
leading group nor lift another group into the page.

## Fast-path evidence (2026-09-30, 8,142,650-document candidate)

| Check | Result |
| --- | --- |
| Sidecar build | 13 s including plan bounds; `plan-bounds.bin` 12.8 MB |
| Windows scored for relevance | 64 of 15,904 (`the`, `and the`, `you know`, `i think`), 337 (`in the`), 1,776 (`this is the`) |
| Differential, production rows | 1,453 requests identical |
| Differential, broad set | 432 requests (20 common words and pairs × 7 sorts × pages 1, 2, 5, plus creator filters) identical |
| Fixture differential | Every sort, window, and group size for 12 queries against the formatted complete scan, single-range and with four forced partitions; an unsound bound margin makes it fail |

Reader latency on 4 pinned CPUs, three fresh-reader passes (median ms):

| Query | Sort | Schema 7 | Schema 9 |
| --- | --- | ---: | ---: |
| `and the` | relevance | 204 | 93 |
| `the` | relevance | 178 | 131 |
| `the` | newest | 202 | 92 |
| `and the` | views | 198 | 57 |
| `of the` | relevance | 185 | 99 |
| `this is the` | relevance, page 2 | 144 | 176 |
| All 68 broad cases | maximum | 239 | 180 |
| All 68 broad cases | sum of medians | 8,035 | 4,890 |

Without `plan-bounds.bin`, pruning scored every window and relevance took
2–2.5× the scan; that is why prefix plans without a covering bound keep the
scan. `this is the` page 2 is the one case still slower than schema 7 (11% of
windows scored).

## Evidence already in hand

On the real 8.1M-document base, set totals equalled the engine on 11 queries
and block-priority returned the exact first 21 groups after scoring 0.17–16%
of matches (see
[P01 evidence](../../experiments/search/schema9/P01/exact-totals-and-block-priority-2026-09-27.md)).

## Rollout guards and switch order

Publication builds schema 9 and advertises `raw-index-schema-9-v1`. The reader
opens schemas 6, 7, and 9, so an active schema-7 generation keeps serving until
the first schema-9 publication replaces it.

1. **Reader first.** The new reader advertises schema 9. The old worker, which
   builds schema 7, fails its preflight against it before building, so no
   hour-long build is wasted. At startup the reader retires any persisted
   `raw-index-schema-7-v2` job (a derived artifact; the mutation store stays
   authoritative).
2. **Worker second**, through `scripts/reconcile-raw-queue-worker.sh`. Its
   next publication builds, uploads, warms, and activates a schema-9
   generation.
3. **Rollback.** A pre-switch reader cannot open schema 9. Rolling the reader
   back after a schema-9 activation needs the active pointer moved to a
   retained schema-7 generation first; rolling forward (fixing the new
   reader) is the default.

Every publication-format consumer is tied together by tests (see
[raw-operational-invariants.md](../raw-operational-invariants.md)), and
`scripts/raw-schema-differential.sh` must report zero differences between the
schema-7 and schema-9 builds of the same corpus before a switch.

## Deferred to later versions

Bit-packed posting blocks (about a third smaller), lazily verified integrity
units for a fast cold open, compact fast-field columns, sessions and cursors,
and Tier-1/2 summaries.
