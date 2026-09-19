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

The initial unit supports exact terms, conjunction, repeated-token phrases,
negation, creator scope, missing-versus-present numeric sorting, exhaustive
ordering, and pagination. Prefix, fuzzy, split, and token-drop behavior must
return an explicit unsupported result until their independent semantics are
implemented. Unsupported is a failed coverage gate, never a passing fallback.

Verification requires fixture boundary tests, NFC/NFD token equivalence,
phrase positions, negation-before-totals, missing-versus-zero ordering,
page-size invariant enumeration, and a deliberately wrong candidate that the
differential comparator rejects.
