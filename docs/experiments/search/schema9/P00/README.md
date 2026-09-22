# Schema 9 P00 baseline

Status: in progress with failed live-performance gates. This directory is
evidence, not a release claim. Nothing in this capture deployed, activated,
indexed, or deleted data.

## Authoritative observations

- The implementation branch starts at `4d36ec8`, whose merge base with
  `origin/master` is `588dfa7`. The three old `/tmp/ytsearch-*` worktrees are
  absent. Their committed history survives; any uncommitted files do not.
- Authenticated readiness on 2026-09-19 returned generation and mutation
  sequence 2950 with one acknowledged reader. This supersedes the historical
  generation-2145 planning assumption.
- The active schema-6 manifest contains 8,142,650 documents, 32,822 videos,
  and 61 channels. The active reader is a 2 GiB, two-shared-vCPU Fly Machine.
- Reader-process RSS was 788,799,488 bytes with zero process swap. The cgroup
  used 1,906,786,304 bytes, mostly file cache, at the observation. These are
  different metrics and neither is relabeled as the other.
- The Raw queue worker was running. Its queue loop was enabled, while its daily
  update loop was disabled. P00 only observed it and did not start a job.
- The retained schema-8 artifact was absent from the active reader and its two
  retained schema-6 generations. A bounded inspection started the exact
  sleep-only builder `080d297f070d08`, verified the artifact at job sequence
  2152, hashed it, and stopped the same machine. `broad-single.bin` is
  2,738,580,172 bytes, BLAKE3
  `e05dc250790e50958f82aa9932e6eec453a7970613866bbee41a051853c13c8c`,
  and SHA-256
  `b65d859033d4d983662677084d36103dd11e432a652f7d2e0506e7bba40a7733`.
  It contains 8,322,726 documents, 34,611 videos, and 68 channels and was never
  activated. The older 8.208 GB figure described a different retained build and
  is not the current rollback artifact.

The machine-readable facts and failures are in
[`baseline-receipt.json`](baseline-receipt.json).

## Independent exhaustive oracle

The oracle now covers the exact-query foundation plus typed prefix and fuzzy
expansion without importing production parsing, scoring, posting, sorting, or
aggregation helpers. Its expansion tests freeze phrase-aware final-token
selection, first-occurrence term deduplication, per-field lexicons, uncapped
equal-distance admission, Unicode-scalar edit thresholds, decimal exactness,
canonical alternative order, additive score bits, and exact negative terms.
Literal exact-split hypotheses now have fixture coverage for uncapped candidate
enumeration and exclusive score/priority/byte-order selection. Unsupported
positive phrase/split combinations still fail closed. Repeated slots now share
one global score contribution while phrase occurrence checks remain intact.
Plural/fuzzy splits and the token-drop/threshold ladder remain incomplete. The evaluator
now uses corpus-wide IDF/BM25/title-log/first-position field scores with canonical
contribution ordering. Literal unquoted proximity and generation-wide engagement
are implemented, including ranking keys drawn from one actual best document.
Expanded-term positional evidence, complete phrase semantics, complete v2
ranking parity, and the 300-case runtime differential remain open P00 gates.

## Search baseline

[`public-api-samples.txt`](public-api-samples.txt) retains every request from the
first public baseline attempt. [`public-api-summary.tsv`](public-api-summary.tsv)
contains case, attempts, errors, p50, p95, p99, and maximum milliseconds.

The attempt intentionally remains failed. The public edge accepted the first
120 requests, then returned HTTP 429 for 360 requests. `react_global` also had a
490.436 ms maximum. A later run must pace or explicitly provision the test lane,
retain errors in the denominator, and measure internal, public, and browser
timers separately. It must not discard this run or call its valid subset a pass.

### Paced single-sample probe at `afc83ad`

The first read-only probe stayed below the 48-request/minute edge budget and
returned 36 successful responses plus 12 client timeouts. It is still a failed
gate because every request is part of the denominator. Complete response times
included `this is the` 82 ms, `the` 1,341 ms, `and the` 3,292 ms, `in the`
1,879 ms, `css which i think is dead now` 277 ms, `convex` 212 ms,
`createEffect` 161 ms, `blackhole` 92 ms, `coding is dead` 339 ms, and `react`
306 ms. Compact common queries measured 615–747 ms. The two-second cold
`this is the` observation from a separate two-request probe was followed by a
165 ms warm response; it is not a passing cold measurement.

The current schema-6 reader therefore does not satisfy the schema-9 200 ms
gate for broad/common queries. The failure is useful evidence: the lazy exact
engine must obtain exact totals and page-one ranking without scanning or
rescoring the broad match set.

The narrower production contract probe passed on the same date for `blackhole`:
338 global videos and 1,163 moments, 21 Veritasium videos, 15 Vsauce videos,
all seven sorts, exact last-page behavior, and fail-closed invalid sort/video
parameters. That probe proves its named contract only; it does not repair the
failed repeated performance lane.

## Operations capability boundary

Current publication code provides publication locks, verified-build reuse,
completed-file retry, durable receipts, and polling of one detached remote job
while the same wrapper invocation retains its handle. It does not provide
byte-range upload continuation or cross-invocation adoption of a prior remote
job directory. Schema 9 must extend the shared path rather than assuming those
two missing capabilities already exist.

## Exhaustive reference foundation

The first independent oracle unit now covers the exact-query subset without
importing production parser, scorer, posting, sorting, or aggregation helpers.
It validates fixture identity and video-level metadata, preserves absent values
separately from real zero, scans every document, paginates only after complete
ordering, and differentially rejects candidate errors and result/facet drift.
Subsequent units added independent prefix/fuzzy candidates, literal split
hypotheses, corpus-aware scoring, ranking, unquoted proximity, and zero-slop
phrase factors. Token dropping, automatic fallback selection, expanded positional
substitutions and the complete filter/summary surface remain
unfinished. This is foundation evidence, not completion of the 300-case gate.

The grammar boundary unit reads query-error cases directly from the frozen matrix
and checks their expected rejection categories. Separate boundary fixtures cover
512 UTF-8 bytes, 32 phrase occurrences, escaped quotes/backslashes, and negation
boundaries. This tests the independent oracle only: it does not prove production
request validation, the remaining matrix cases, or the schema-9 latency target.

Grammar-unit verification: 38 focused oracle tests; full Raw search crate 218
passed, one ignored; formatting and all-target/all-feature Clippy with warnings
denied passed. Eleven matrix query-error cases are exercised by one test; they
are not eleven separately reported Rust test functions. No live/performance lane
was run for this test-only parser unit.

### Phrase-slop policy clarification

Independent review identified that the prior contract specified zero-slop
scoring but required nonzero slop without defining its score reduction. The
contract now explicitly defines total intervening-token membership, a separate
budget per required phrase, exact negative phrases, and strongest-occurrence
factor selection. Gapped positive phrases retain ordered evidence. This is an
intentional v2 ranking policy, not a claim of legacy score equivalence.
Implementation and runtime differential evidence are separate gates; policy
review alone does not close either.

The independent reference now implements this policy with a validated optional
slop type and exhaustive ordered positions on bounded fixtures. Nine focused
tests passed, including independent binary32 factor/score goldens, repeated
tokens, total-gap boundaries, exact exclusions, strongest-occurrence selection,
stable ties, and absent-versus-explicit-zero validation. Five frozen matrix
queries additionally exercise local-fixture membership, ranking, pages, and
creator/content-kind aggregates; this subset does not check highlights or a
production candidate engine. All 300 matrix definitions retain their original
digest. Full crate verification: 227 passed, one ignored; formatting and
all-target/all-feature Clippy with warnings denied passed. Independent final
code review passed and separately ran all nine focused tests. No live/perf
measurement or runtime/reference parity is claimed by this unit.

### Reviewed matrix byte-boundary correction

The later acceptance audit found `threshold-017-max-query` incorrectly expected
a 4,096-byte query to succeed despite the frozen 512 UTF-8 byte request limit.
The structural/digest validator had accepted that contradiction. Before fixing
the fixture, the new semantic guard reproduced the failure:
`threshold-017-max-query: successful request query must not exceed 512 UTF-8 bytes`.

Only the query strings for immutable IDs `threshold-017-max-query` and
`threshold-018-over-max-query` changed: 4,096/4,097 bytes became 512/513 bytes.
All IDs, counts, and expected outcomes are preserved. This deliberately revises
the matrix's canonical digest from
`4ab1279df8cb0debb8f877a59917320fac86a2c4ef43ab26b08153fa1927e492` to
`3d36c847d3b9ef641c363ccffa2053975f13032ec849c02e49dae18ffd0949b9`.
Earlier receipts describe the original digest; they are not retroactively
evidence for this revision. No engine limit was loosened to accommodate a test.

The validator now rejects every oversized success query, checks both actual
boundary sizes, and the Rust grammar test parses the matrix's success fixture.
Matrix validation passed with 300 cases; focused grammar tests: four passed;
full Raw search crate: 227 passed, one ignored; formatting passed. These remain
reference/fixture checks, not production differential or performance evidence.

### QG-06 exact-only ordered selector

The reference now exposes an explicitly exact-only retained-query selector,
separate from the still-unsupported automatic `TokenDrop` policy. It selects
the full query first, then every prefix from `n-1` to `1`, then every suffix
starting at `1` through `n-1`. It returns the original retained range plus the
selected result, or an explicit exhausted outcome. Negatives remain exact and
opaque; positive phrases are rejected by this helper. Scores describe retained
slots using generation-wide statistics. Threshold one uses complete scoped
totals before pagination, not the number of hits on the requested page.

Six focused tests cover every prefix/suffix for 21, 22, and 32 distinct terms;
the actual 22-token matrix request `threshold-004-fifteen-terms` reaches the
twenty-first prefix and suffix. Other fixtures cover duplicate slots,
prefix-before-suffix preference, creator and term/phrase exclusions,
generation-wide scoring, deep/empty/count-only pages, and exhaustion.
All six passed independent review and a separate test run. Full crate: 233
passed, one ignored; formatting, all-target/all-feature Clippy with warnings
denied, and diff checks passed. No matrix case was relabeled complete, no
matrix digest changed, and no production/live/performance gate is claimed.

## Next autonomous repairs

1. Implement schema-9 structural metadata and lazy exact execution against the
   frozen contract; do not optimize by dropping broad-query membership.
2. Run the full 300-case exhaustive differential and exact Tier-0/Tier-1
   summary oracles against that implementation.
3. Add a single read-only P00 collector and fixture test so future captures do
   not depend on hand-assembled commands.
4. Run paced internal/public/browser baselines against identical corpus and
   source identities, then request an independent verdict at the exact head.
