# Fallback selection and independent admission evidence

Status: reviewed design direction, not a deployed controller. The admission fix
has a reproduced failing regression in `09b4209`; implementation evidence follows
separately. Automatic stage policy below is proposed, not a claim that the existing
isolated oracle modes already implement it. The normative query contract remains
the authority for v2 semantics.

## Caller and ownership

```rust
let selected = session.select_query(validated_query)?;
let overview = selected.exact_summary()?;
let page = selected.page(window, sort)?;
```

Selection returns a generation-bound predicate, never a precomputed answer.
The session owns scope, exclusions, enabled fields, grouping unit, and policy
identity. Page size, page depth, and sort do not influence which predicate wins.
Only the selected object can execute the predicate against its retained generation.

```rust
enum Stage { Base, ExactSplit, FuzzySplit, TypoOne, TypoTwo }
enum Retained { Full, Prefix { end: usize }, Suffix { start: usize } }
enum CountUnit { Videos, Documents }
enum ThresholdProof { AtLeastFive, ExhaustedBelowFive(BelowFive) }
enum SelectionReason { ReachedFive, SparseNonempty, Exhausted, NonExpandable }
struct SelectedQuery<'g> { /* lease, predicate, retained slots, policy, reason */ }
```

Keep transitions private behind one selection API. Generate stage descriptors
lazily; do not eagerly compile every retained query's full lexicon alternatives.
Each threshold proof is bound to generation, complete predicate, scope, and count
unit. Reaching five permits a bounded proof, not an approximate final total.
Proving fewer requires exhaustion or an exact structural bound. No answer changes
because a work limit was hit; resource failure is explicit.

## Proposed controller policy to implement after the admission fix

1. Base admits exact terms plus proper prefixes on the last retained free unquoted
   slot. Numeric slots stay exact-only. Prefix admission is not suppressed by five
   exact matches. Scope, exclusions, and required phrases apply before counting.
2. For a single free unquoted slot without positive phrases, add all exact split
   hypotheses, then fuzzy splits if still below five. Do not use exact-split
   existence as an additional veto: the scoped five-result threshold is sufficient.
   Each fuzzy split corrects exactly one component by scalar edit distance one;
   original and corrected components have at least three scalars, and pure-decimal
   components are not corrected. Mixed enabled fields are allowed within a document.
3. Add complete whole-token distance-one and then distance-two families for eligible
   slots, retaining earlier families. AND across slots allows multiple corrected
   slots. Do not stop halfway through admitting a semantic family.
4. Stop at the first stage with five scoped videos/documents as appropriate. At
   the terminal eligible stage, retain every sparse nonempty result without an
   all-exact witness requirement. Only an empty terminal set permits dropping.
5. For queries without a positive phrase, run the same ladder for full query,
   all shorter prefixes, then all suffixes. Each retained predicate has its own
   final prefix slot; singleton drops may split. Select one predicate, never
   union different retained ranges. Exhaustion returns the empty full predicate,
   not a wildcard.
6. Positive phrases remain exact-token/slop predicates. Free unquoted companions
   may expand, but dropping and split recovery stay disabled for any positive
   phrase query in the initial controller proposal. Tokens shared with a phrase
   remain locked exact and score once. The current QG-06 wording does not carve
   out quoted queries. This proposed exception requires an explicit normative
   QG-06 revision and discriminating phrase fixtures before implementation; until
   then it is not an approved controller rule. Companion dropping remains an
   alternative to adjudicate in that review.
7. Distinguish first-admitting stage from selected terminal stage. A sparse
   nine-scalar compact token can first match at FuzzySplit yet finish at TypoTwo.
   Stages are eligible by syntax/length, not by whether an iterator happened to
   produce a new alternative. Pagination cannot alter this receipt.

The proposed QS-01 revision groups one Whole hypothesis containing additive
admitted exact/prefix/typo contribution
identities. Each concrete split is a separate competitor; Whole and splits never
sum together. Whole's tie class is its strongest actually matched kind; preserve
the contract's existing kind priorities and canonical byte tie order. The
current QS-01 text describes per-kind competing whole parses. Its normative
wording and score fixtures must be revised explicitly before this grouped Whole
rule is implemented. Numeric score selection remains slot-local.

Expanded Whole positions are the union of admitted matching physical occurrences.
Distinct query roles require distinct positions for proximity, not for ordinary
AND membership. Ordered preference requires full adjacency across all retained
roles, not just one adjacent pair. Fuzzy-split local factors apply once and do not
trigger a second document multiplier. Automatic splits are single-slot/no-phrase,
so they need no invented outer multi-slot position. Keep score positions separate
from admission witnesses; neither may cross disabled fields or chunk boundaries.
These position rules need normative fixtures and a reviewed contract revision
before controller implementation; the current execution does not prove them.

## Immediate fix: score-free admission

```rust
enum Admission { Miss, Hit(MembershipEvidence) }
struct MembershipEvidence { /* retained literal identities, provenance */ }
struct WinningScore { /* finite score, chosen hypothesis, score positions */ }
fn admit(plan: &Predicate, doc: &ScopedDocument) -> Admission;
fn score(plan: &Predicate, doc: &ScopedDocument) -> Result<WinningScore, Error>;
```

These are private boundaries, not a new caller choreography. Admission receives
no scoring context. Whole unions matching admitted identities. A split contributes
only after both components match; union successful complete alternatives. An AND
publishes evidence only when all required predicates and exclusions succeed.
Negatives gate admission and never add positive evidence. Transcript provenance
means participation in a complete satisfied predicate, not that every term must
match transcript alone. Exact literal evidence survives a different score winner.

The score reducer retains numerical operation order, winner selection, and tie
keys. It cannot overwrite membership fields. Reject nonfinite values in every
actually scored hypothesis, including losers. A count-only admission pass must
not eagerly score all matches or promise to validate unexecuted arithmetic.
The exhaustive reference may still score every result for comparison; that is
not the lazy production execution strategy.

## Synthesis and verification

Initial candidates: C1 cumulative enum controller, C2 declarative predicate rows,
C3 retained-query lattice/certificates. Root read all artifacts completely; C2/C3
runners hit usage limits after saving complete designs. No runner completion or
test pass was inferred from their files. Fresh cross-judge scores: C1 17/20,
C2 15/20, C3 14/20. Choose C1's bound API, graft C2's single cumulative threshold
policy/lazy row presentation and C3's proof identity binding. Reject eager rows,
mandatory witness-object storage, split-existence vetoes, partial-adjacency rules,
and changed tie priorities. Phrase companion dropping remains deliberately outside
the initial controller policy; that is explicit, not a hidden planner choice.

All candidates missed the winner-only evidence defect. Re-grounding used an actual
two-chunk reproduction: split scoring preserved both IDs but reported one moment
and no transcript provenance. Exact-only returned two moments; title-only returned
one. Two new designs were independently compared: A combined evaluation product,
B pure admission interpreter. Fresh judge rated both 18/20 as written; selected
B, repaired by removing its eager all-members scoring requirement, scored 20/20.
Graft A's unchanged arithmetic, losing-hypothesis error handling, and private
adapters. Reject requiring a score winner merely to establish membership.

Immediate red/green gates: original 0/30000ms reproduction; title-only control;
literal coverage under a split winner; unchanged numerical bits; incomplete split
and failed-AND evidence isolation; exclusions, creator/field scope, retained slots,
count-only/deep pages, and a score-free admission API. These tests must run before
claiming the fix. Controller gates additionally need 4/5/6 thresholds, scoped
split eligibility, sparse stage receipts, multi-slot corrections, full adjacency,
generation mismatch, and all-page runtime/reference equivalence. No design score
or local oracle test proves production latency, full matrix coverage, or migration.
