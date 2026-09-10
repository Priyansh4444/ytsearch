# Broad single-token query index

## Problem

The schema-7 broad path bounds retained memory but still visits every matching
transcript posting. That preserves exact grouped results, yet `react` exceeded
five seconds on the shared2/2 GB canary. The replacement must make query work
independent of matching document count while preserving exact video `found`,
20-second moment totals, exact-plus-prefix and title relevance, all supported
sorts and windows, highlights, and `group_limit <= 250`.

## Usage

The HTTP and backend interfaces do not change. After existing validation and
single-token planning, the engine asks one immutable index for a complete
grouped answer. An absent key means the existing postings path remains the
exact fallback.

`BroadSingleIndex::lookup(token, fields)` selects the materialized answer.
The caller checks `supports_depth` before using it. Missing plans, unsupported
depths, and ineligible queries retain the canonical postings fallback.

## Shape

Schema 8 adds one required immutable `broad-single.bin`. A key represents the
complete normalized query plan, not one independent term: literal exact match,
the accepted prefix alternatives, their score multipliers, field set, and
token-position policy. Pre-reducing the plan is necessary because independent
term summaries cannot reconstruct overlapping document scores or the union of
20-second moment clusters exactly.

There are six profiles: `text`, `title`, and `text,title`, each with position
priority enabled or disabled. A materialized profile stores:

- exact `found` and `total_moments` summaries;
- one record per matching dense video ID;
- the exact group relevance tuple;
- exact per-video moment count;
- up to five document matches per video with original `f32` score bits and flags.
  Legacy payload version 1 retains up to 250. Larger requests fall back to the
  canonical reducer when the materialized depth is insufficient.

Groups are ordered by video ID. Lookup currently decodes all matching groups
and retained hits before pagination. It avoids transcript-posting traversal,
but allocation and parsing still scale with matching videos. Page-only group
decoding is not implemented.

The binary has a checked header, a sorted variable-length token directory, and
contiguous answer payloads. Records use fixed-width integers, not delta coding.
Version 3 compresses each answer independently with Zstd level 1 and prefixes
its exact decoded length. Versions 1 and 2 remain readable without decompression.
Scores are stored losslessly with `f32::to_bits`. Counts are checked against
available bytes before allocation; decoded lengths are bounded by corpus size.

## Build and publication

`engine/broad_single.rs` constructs one plan at a time with the same
grouping, ranking, and moment reducer used by live search. A bounded external
spool avoids retaining every plan in RAM. Identical payloads are not shared.

The file is built before `manifest.json`, included in the schema-8 generation
file set and `youtube_index_store::RAW_GENERATION_FILES`, hashed and length
checked like every other payload, uploaded resumably, and activated by the
existing atomic generation switch. Schema 5–7 remain readable and use the
postings path. Schema 8 without a valid file fails to open; it never silently
falls back after corruption.

Compressed payload version 3 uses `raw-index-schema-8-v2` compatibility. Older
`raw-index-schema-8-v1` pending jobs are validated and retired at startup while
durable mutations remain authoritative. A
separate sidecar manifest or activation pointer is forbidden because it could
mix generations and duplicate recovery invariants.

## Synthesis decision

The query-plan materialization design is the base because it preserves prefix
overlap and cross-term moment clustering while eliminating document-posting
work. The design also uses a measured threshold matrix, deterministic
video-range partitioning, encoding comparisons, and stricter resource gates
from the alternative materialized-prefix design.

Rejected shapes:

- Per-term video summaries still need per-document evidence to merge prefix
  overlap and moments, returning query work toward posting cardinality.
- Block-max/WAND can skip work for top hits but exact global totals still force
  a complete traversal.
- A runtime response cache leaves the first query unbounded and adds mutable
  generation-invalidated RAM.

## Measurement gate

Do not publish schema 8 until a sizing command runs on the production snapshot.
The first schema-7 probe is available as:

```console
youtube-raw-search measure-broad-single /data/mutable/generations/<id> react fire
```

It executes the canonical reducer sequentially for all six profiles and reports
posting visits, exact result totals, retained depths, estimated encoding bytes,
elapsed time, and before/after RSS. It intentionally accepts explicit tokens;
the threshold sweep and throwaway artifact below remain the next gate.

It must measure thresholds `{25k, 50k, 100k, 250k}` and retained depths
`{5, 20, 250}`, including:

- selected keys, unique plan fingerprints, alternatives, matching videos, and
  retained documents;
- fixed-width, varint/zig-zag, and block-compressed byte estimates plus one
  throwaway encoded artifact;
- total posting visits, build duration, scratch bytes, and peak builder RSS;
- active + rollback + incoming generation disk requirement.

### Production-snapshot evidence

On 2026-08-31, generation 2145 (8,322,725 documents and 34,610 videos) was
measured twice on the exact shared2/2 GB canary. Each fresh diagnostic process
reproduced the same cold-volume pattern:

| Plan | Emitted documents | Matching videos | Packed upper bound | First pass | Identical warm pass |
| --- | ---: | ---: | ---: | ---: | ---: |
| `fire`, text | 26,139 | 7,499 | 354,092 B | 11,190/11,178 ms | 8 ms |
| `react`, text | 54,356 | 6,059 | 582,332 B | 451/445 ms | 14 ms |
| `react`, text + title | 283,326 | 5,187 | 1,854,327 B | 39/40 ms | 49/43 ms |

The process grew from about 84 MB to 265 MB RSS. The eleven-second first pass
followed by an eight-millisecond identical pass proves that the worst latency is
page-fault and volume-read amplification across postings, not the relevance
comparator. A 2 GB machine cannot depend on keeping the 1.9 GB corpus image hot.
The compact query-plan payloads are small enough to make schema-8
materialization the accepted direction; a runtime cache or posting scan is not.

The release gate requires semantic differential equality, zero document
posting decodes on an indexed hit, shared2/2 GB reader peak RSS below 1.5 GiB,
zero swap/OOM, and concurrent broad-query p95 and worst `react`/`fire` below
200 ms. Filters and exclusions remain on the exact fallback until production
traffic proves that additional materialization is necessary.

## Verification

- Differential JSON for real and adversarial tokens across all six profiles,
  sorts, pagination boundaries, filters, and group limits 1/5/249/250.
- Property tests for documents shared by exact and prefix alternatives and
  timestamps at 19,999/20,000/20,001 ms boundaries.
- Corruption tests for every structural field and schema-7 compatibility.
- Deterministic build hashes, bounded-partition equivalence, interrupted upload,
  activation, restart, and rollback tests.
- Production-snapshot cold/warm search plus concurrent publication tests on the
  exact 2 GB shape.

## Next implementation step

The checked binary format, schema-aware publication contract, six-profile
reader, and prefix-plan builder path are implemented. A post-order trie pass
retains only the four frequency winners required by the query contract for each
subtree, so non-terminal prefixes such as `con` can be selected without a
global prefix set. A focused differential test proves the materialized response
equals the canonical posting reducer. Schema 8 must not be deployed until the
production build/RSS/disk gate and broader differential matrix pass.

The current writer encodes and drops one plan at a time into a cold payload
spool, retaining only its small ordered directory in memory. Final composition
streams the spool into `broad-single.bin`; it never keeps all groups or encoded
payloads resident together.
