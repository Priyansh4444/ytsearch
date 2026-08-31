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

```rust
let plan = self.single_token_plan(term, fields)?;
if let Some(answer) = self.inner.broad_single.lookup(plan.key())? {
    return self.format_broad_groups(params, &plan.highlights, answer, &mut cache);
}
self.search_single_token(params, term, fields, excluded_documents)
```

The builder owns selection, reduction, encoding, and sizing. Callers never
coordinate archive stages or inspect offsets.

```rust
let report = BroadSingleBuilder::measure(inputs, policy)?;
let artifact = BroadSingleBuilder::build(inputs, report.accepted_policy()?)?;
```

## Shape

Schema 8 adds one required immutable `broad-single.bin`. A key represents the
complete normalized query plan, not one independent term: literal exact match,
the accepted prefix alternatives, their score multipliers, field set, and
token-position policy. Pre-reducing the plan is necessary because independent
term summaries cannot reconstruct overlapping document scores or the union of
20-second moment clusters exactly.

```rust
enum BroadFieldSet { Text, Title, TextAndTitle }
enum PositionPriority { Disabled, Enabled }

struct BroadProfile {
    fields: BroadFieldSet,
    position: PositionPriority,
}

struct BroadPlanKey<'a> {
    normalized_token: &'a str,
    profile: BroadProfile,
}

struct BroadSingleIndex { /* validated mmap and private directory */ }
struct BroadAnswer<'a> { /* exact totals and a checked group cursor */ }

impl BroadSingleIndex {
    fn open(path: &Path, manifest: &Manifest) -> Result<Self>;
    fn lookup(&self, key: BroadPlanKey<'_>) -> Result<Option<BroadAnswer<'_>>>;
}
```

There are six profiles: `text`, `title`, and `text,title`, each with position
priority enabled or disabled. A materialized profile stores:

- exact `found` and `total_moments` summaries;
- one record per matching dense video ID;
- the exact group relevance tuple;
- exact per-video moment count;
- up to 250 document matches with original `f32` score bits and flags.

Groups are ordered by video ID. The query scans compact video headers, uses the
existing comparator to retain only the requested video window, then decodes and
hydrates documents only for that page. Runtime work is
`O(log K + V log R + R*G)`, where `K` is materialized keys, `V` matching videos,
`R` the requested video window, and `G <= 250`. It does not depend on matching
transcript postings.

The binary owns a checked header, a front-coded normalized-token dictionary,
six profile slices per key, exact totals, delta-coded video groups, and
variable document records. Scores are stored losslessly with `f32::to_bits`.
Every offset, range, varint, ID, count, finite score, ordering invariant, and
trailing byte is validated before serving.

## Build and publication

`engine/broad_single/builder.rs` constructs one plan at a time with the same
grouping, ranking, and moment reducer used by live search. A bounded external
spool and deterministic video-ID partitions prevent the largest plan from
exceeding its build-memory budget. Identical query plans share a versioned plan
fingerprint and payload.

The file is built before `manifest.json`, included in the schema-8 generation
file set and `youtube_index_store::RAW_GENERATION_FILES`, hashed and length
checked like every other payload, uploaded resumably, and activated by the
existing atomic generation switch. Schema 5–7 remain readable and use the
postings path. Schema 8 without a valid file fails open; it never silently
falls back after corruption.

The format advances coherently to schema 8 and `raw-index-schema-8-v1`. A
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
It must measure thresholds `{25k, 50k, 100k, 250k}` and retained depths
`{5, 20, 250}`, including:

- selected keys, unique plan fingerprints, alternatives, matching videos, and
  retained documents;
- fixed-width, varint/zig-zag, and block-compressed byte estimates plus one
  throwaway encoded artifact;
- total posting visits, build duration, scratch bytes, and peak builder RSS;
- active + rollback + incoming generation disk requirement.

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

Extract one pure shared group reducer and implement the read-only production
corpus sizing command. The sizing report can reject or revise the physical
layout before schema 8 enters the publication contract.
