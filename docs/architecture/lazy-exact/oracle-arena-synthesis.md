# Schema-9 exhaustive-oracle arena synthesis

Status: implementation in progress  
Date: 2026-09-19

## Usage first

The correctness harness loads a validated adversarial fixture, converts a
matrix request into an oracle-owned query, exhaustively evaluates every
document, derives totals and summaries from the complete match set, orders the
complete result, and paginates last. A candidate adapter normalizes an engine
response into the same result type and reports the first exact mismatch.

The oracle must not import the production query parser, posting structures,
score helpers, ordering helpers, or aggregation code. Shared bugs are a failed
oracle, even when the differential is green.

## Arena verdict

The typed-AST/compositional evaluator is the base. Its public surface remains
one `evaluate_exhaustively` entry point plus fixture/request/result boundary
types. It was selected because phrase, negation, field provenance, filters,
and future split/fuzzy hypotheses compose as typed nodes instead of expanding
a request-shaped switch.

Two ideas are grafted from the compact scan candidate:

1. `FixtureCorpus`, `OracleRequest`, and `OracleResult` remain wholly
   independent of production types.
2. A narrow `CandidateImplementation` boundary performs differential
   comparison without letting production execution leak into the oracle.

The cross-judge agreed with this base and both grafts. Two additional arena
runners were interrupted after failing to produce their assigned artifacts;
their dropout is not counted as supporting evidence.

## Rejected shapes

- A table-driven request switch was rejected as the semantic core. It is
  attractive for the first flat term cases but turns interactions among
  phrases, negation, fields, splitting, and token dropping into scattered
  branches.
- Reusing `SearchParams`, the current parser, or current score helpers was
  rejected because a shared defect would make both sides agree incorrectly.
- Building schema-9 postings before this oracle was rejected because fast
  execution would have no independent authority for membership, score bits,
  totals, or page ordering.

## First verifiable unit

The implemented foundation supports exact terms, conjunction, repeated-token
phrases, negation, creator scope, missing-versus-present numeric sorting,
exhaustive ordering, and pagination. The next unit adds typed per-field prefix
and fuzzy plans: prefix applies only to the last deduplicated positive token,
fuzzy distance counts Unicode scalars, decimal-only tokens remain exact, every
in-threshold lexicon alternative is admitted, and negative terms remain exact.
Exact literal split hypotheses are implemented as a separate unit. Token-drop
and the full expansion ladder remain unsupported until their independent
semantics are implemented. Unsupported is a failed coverage gate, never a
passing fallback.

Verification requires fixture boundary tests, NFC/NFD token equivalence,
phrase positions, negation-before-totals, missing-versus-zero ordering,
page-size invariant enumeration, and a deliberately wrong candidate that the
differential comparator rejects.

Prefix/fuzzy verification additionally covers phrase-aware final-token
selection, first-occurrence deduplication, independent title/text lexicons,
more than four equal-distance alternatives, the four- and seven-scalar edit
thresholds, mixed alphanumeric terms, ASCII and non-ASCII decimal terms,
canonical alternative ordering, additive score bits, and exact negation while
positive terms expand.

The literal-split unit enumerates every qualifying Unicode boundary with at
least three scalars on each side and both components in enabled lexicons. Each
document selects one hypothesis by score, whole-token priority, then canonical
bytes; competing hypotheses are not added. Components may occupy different
enabled fields within one document. Decimal-only tokens remain exact.
Tests include nine competing splits, canonical ties, field scope, cross-document
rejection, exact negation, and the component/component/bonus reduction.
Plural variants, fuzzy splits, repeated slots, positive phrases with splitting,
and the threshold-driven expansion ladder remain pending. Unsupported phrase
and repeated-slot combinations return explicit errors in this fixture evaluator.

## Remaining authority gaps

This fixture evaluator is still a foundation, not the v2 ranking oracle. Its
field scores currently use token frequency times fixed field weights. They do
not yet implement corpus IDF, BM25 normalization, title logarithmic frequency,
position/proximity factors, or engagement. Existing score-bit assertions prove
only the implemented arithmetic, not equivalence with the frozen complete
score. The matrix and release gates remain open until those formulas and the
production differential adapter are implemented and exercised end to end.

Numeric prefix suppression now has a red-to-green regression covering ASCII
and Arabic-Indic decimals plus mixed alphanumeric controls in all three field
selections. Global query-slot deduplication across terms and phrases remains
unfixed. Deduplicating the plan's term list alone does not prevent the AST
evaluator from adding the same slot twice. Keep these as explicit defects;
do not treat the earlier unit review as full semantic approval.
