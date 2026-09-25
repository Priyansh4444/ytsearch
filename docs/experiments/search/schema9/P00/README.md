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
- A 2026-09-22 read-only audit corrected the original manifest attribution:
  generation 2950's active schema-6 manifest contains 8,422,419 documents,
  35,215 videos, and 61 channels. The previously recorded 8,142,650 documents
  and 32,822 videos belong to the immutable `/data/index` base manifest, not
  the active mutable generation. The active manifest was written 2026-08-31,
  before the original 2026-09-19 capture, so this was a measurement error rather
  than observed growth. The active reader is a 2 GiB, two-shared-vCPU Fly Machine.
- Reader-process RSS was 788,799,488 bytes with zero process swap. The cgroup
  used 1,906,786,304 bytes, mostly file cache, at the observation. These are
  different metrics and neither is relabeled as the other.
- The Raw queue worker was running. Its queue loop was enabled, while its daily
  update loop was disabled. P00 only observed it and did not start a job.
- On 2026-09-24 the exact worker was stopped and staged with the current queue
  entrypoint, 62-handle list, production-Machine-specific reader URL, and daily
  loop enabled. The [staging receipt](queue-stage-2026-09-24.json) verifies the
  live Machine configuration and embedded file hashes. The reader remains
  IPv4-only, so the full health gate fails and daily indexing has not resumed.
- A later read-only exact handle comparison found 62 configured handles but 61
  in active generation 2950. `@BrodieRobertson` is the sole missing configured
  handle; no unexpected handles were present. The queue worker has no
  `.reconciliation` discovery-manifest directory yet. The
  [`creator-gap-2026-09-22.json`](creator-gap-2026-09-22.json) receipt binds this
  comparison to the list digest, active-generation pointer, and channel-file
  digest. This is not a per-video completeness check, nor proof that Brodie's
  discovery or queue request succeeded.
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
Its original source SHA and timestamps identify the first capture; the
`observation_correction` field identifies the later direct audit and retains
the misattributed base manifest separately. The public health endpoint also
reported 8,422,419 documents on 2026-09-22. This count agreement does not
substitute for an authenticated readiness receipt or full live performance lane.

The read-only P00 collector previously exited before writing `receipt.json`
when a Fly probe failed under `set -e`. A committed red fixture (`014cd4d`)
demonstrated the missing receipt. The repair records its five baseline Fly/HTTP
probes independently, reports malformed responses and transport failures as
separate cases, leaves unavailable values null, and counts simultaneous probe
failures exactly. The local success, failed-reader, malformed-readiness, and
combined-failure fixtures pass.

The base-versus-active misattribution prompted a second red fixture (`119ab88`).
The collector now reads the active-generation pointer from the single reader,
binds it to authenticated readiness, then reads only that generation's manifest
through two read-only SSH probes. It records schema and document/video/channel
counts plus a SHA-256 manifest digest. A stale pointer fails closed; skipped
probes are counted when prerequisite evidence is unavailable. These local
fixtures repair evidence collection; the separately retained
[`capture-2026-09-22/`](capture-2026-09-22/README.md) provides the later
credentialed, read-only inventory receipt at source `e528093`.

The first live attempt at `4b459bb` retained a failed receipt because the
collector's fixture used obsolete `mode` while the live availability response
uses `schedule_policy`. After a red fixture (`50f77c4`) and the contract fix,
the seven probes passed. Another red fixture (`30ddae9`) prevented the collector
from hardcoding target grammar/rank version 2 as if they were observed production
versions; the final receipt reports observed versions as unknown. This inventory
capture does not close P00's failed public performance or runtime-differential
gates.

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

### QV-01 count-only reference slice

At source `b6d2b5f88f800497e0cb39cd82722ac372ddf6a0`, the independent
reference evaluates the frozen matrix case `empty-010-invalid-page-size-zero`
with `per_page=0`. A fixture with two transcript moments in one video, one
title-only video, and one other creator preserves three videos and four exact
moments across full and count-only windows. Zero-sized windows return no hits
and `has_next_page=false` for first, deep, zero, and maximum-u32 pages. Creator
scope retains its own exact totals and facets. The differential checker also
rejects an incorrect next-page flag even when hits and totals agree. The focused tests passed 3/3;
the full Raw crate passed 244 tests with one ignored; all-target Clippy and
formatting passed. The query matrix still records QV-01 as `decision_pending`:
this fixture does not test the production runtime, `limit=0`, or cursors.

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

### Admission evidence independent of score-hypothesis selection

A new red reference regression at `09b4209` reproduced a lost `blackhole`
transcript match: two documents matched, but a title-only split won scoring and
the oracle reported one moment and no transcript provenance. The observed
failure was `(1, title=true, transcript=false, phrase=false)` versus the
required `(2, title=true, transcript=true, phrase=false)`. The selected split
score bits were `0x41277cd6` and are preserved by the repair.

The corrected oracle computes score-free membership evidence from every
complete admitted hypothesis, while selecting exactly one score hypothesis as
before. A partial split cannot contribute provenance: its test lexicon really
contains both split terms, but the tested document contains only one. Separate
fixtures check creator/field scope, negative predicates, phrase membership and
ordered evidence, and title-only versus exact-only paths. The slop membership
check has a boolean path and is differentially checked against reference phrase
evaluation on bounded fixtures. See
[`fallback-evidence-design.md`](../../../../architecture/lazy-exact/fallback-evidence-design.md)
for the arena synthesis and the proposed controller boundary. Controller
semantics marked proposed there are not a completed v2 contract.

Focused schema-9 tests: 61 passed; full Raw crate: 241 passed, one ignored;
formatting and all-target/all-feature Clippy passed. An independent reviewer
reran the 61 focused tests and found no blocker. This evidence covers the
reference oracle only; it does not prove production score parity, the 300-case
runtime differential, the 2 GiB latency gate, or readiness to activate schema 9.

### Reader private-network repair checkpoint (2026-09-22)

The started reader `d89303da029328` still advertises
`RAW_SEARCH_BIND=0.0.0.0:8108`; the stopped, DNS-isolated canary
`847314c22d52e8` is configured for `[::]:8108`. The queue worker
`8e426df7e305e8` remains started with `DAILY_UPDATE_ENABLED=0`, and the live
queue invariant fails. These are current-state observations, not repaired
production behavior.

Commit `6e7544d` changes both Raw serve modes to default to `[::]:8108`.
Commit `1f426dd` makes the on-demand worker own the 24-hour catalog pass on
hourly Cloudflare wakes and checks the reader's IPv6 configuration. Local Raw
crate tests (242 passed, one ignored), Clippy, formatting, the queue fixture,
the cost guard, and Worker Vitest (36 tests) passed. The exact committed Raw
overlay was built and pushed from production base image
`sha256:09b2fc639c63cbca74f05da137c455cf2eb5d32f0a256e12d1e5b711dca66dcd`
as `registry.fly.io/ytsearch-raw-production:raw-overlay-1f426ddb6cc4-09b2fc639c63`
with manifest digest
`sha256:25fd489e367ee3c333c58139f3018f399ce6674655548ae97e09c17be2ec6c79`.
The build used `--build-only --push` and did not deploy it.

Canary staging dry run selected only machine `847314c22d52e8` and volume
`vol_4oj1x0oxkq82eo2r`. The first apply attempt stopped at the credential
preflight because `FLY_API_TOKEN` was absent. An authenticated Fly CLI session
subsequently issued a short-lived, scoped organization token. The next apply
attempt exposed a source-commit mismatch: this README had been committed after
the image build, so staging derived an image tag that did not exist. Running
the stage script from an isolated checkout of the approved `1f426dd` source
resolved that mismatch. The canary now holds the exact image digest above,
remains stopped, skips DNS registration, and has no services or checks.

A leased, bounded private-IPv6 smoke then started that canary. Fly took about
three minutes to prepare the VM. After the Machine started, `/health` did not
respond during 300 one-second attempts through the exact private IP, and an
in-VM kernel check showed no TCP6 listener on port 8108 during that interval.
The authenticated admin readiness check was therefore not reached. Cleanup
stopped the canary and released its lease; the production reader remains
started on the old IPv4 image. See
[`reader-canary-smoke-2026-09-22.json`](reader-canary-smoke-2026-09-22.json).
This is a failed startup gate. Private IPv6 readiness, pending publication
recovery, daily completion, and schema-9 search gates remain open.

Commit `eeb679b` adds startup stage logs. Its tested, immutable overlay was
pushed and staged on the same isolated canary. A second bounded run confirmed
that SQLite opened in about half a second, then active-generation opening did
not finish within 300 seconds. The canary volume points to generation 2152,
whose directory is 4.2 GB and includes the retained 2.6 GB schema-8
`broad-single.bin`. During the open, the reader was observed in disk sleep;
its read counter progressed beyond 5 GB, RSS rose from about 9 MB to 753 MB,
and swap remained zero. The evidence localizes the delay to generation open
but does not identify the specific file operation. The canary stopped and its
lease was released. See
[`reader-canary-diagnostic-2026-09-22.json`](reader-canary-diagnostic-2026-09-22.json).
The 300-second startup gate still fails; this run does not validate private
IPv6 readiness or authorize production promotion.

The source path explains a mandatory part of this work: `RawSearchEngine::open`
selects `ManifestValidation::Full`, and `validate_manifest_files` reads and
BLAKE3-hashes every schema-8 generation file before the listener binds. That
includes `broad-single.bin`. The observed disk reads are consistent with this
full validation; the probes did not measure each file's share of the 300 seconds.
Schema 9's lazy per-unit checksums and cold-open contract remain necessary.

The exact `7cf6fc3` overlay was then built from the same production base and
staged only on the stopped, DNS-isolated canary. A third 300-second private
IPv6 probe failed, and cleanup stopped the canary, released its lease, and
revoked the temporary scoped token. Per-file startup logs show that all 12
schema-8 files were checksummed: 4,462,828,726 logical input bytes took a sum
of 266,991 ms. `broad-single.bin` alone contributed 2,738,580,172 bytes and
163,901 ms. The last checksum completed at 09:31:28 UTC, but no
`active raw generation opened` or listener-ready event appeared before the
probe ended. See
[`reader-canary-checksums-2026-09-23.json`](reader-canary-checksums-2026-09-23.json).
These byte counts are checksum input, not measured physical I/O. Source review
shows that `BroadSingleIndex::open` subsequently decodes and validates every
answer payload; its elapsed time has not been isolated. This remains a failed
cold-start gate, and the production reader was not changed.

Commit `b385432` added separate metadata and broad-index startup markers. Its
exact tested overlay was built and pushed from the same immutable base, then
staged only on the isolated canary. The fourth bounded private-IPv6 probe again
failed the 300-second health gate. The 12 checksums took 268,588 ms in sum;
metadata and other indexes then opened in 6,005 ms. Eager
`BroadSingleIndex::open` began at 10:04:57 UTC and had not completed when the
canary was stopped at 10:05:25 UTC. The new receipt is
[`reader-canary-open-stages-2026-09-23.json`](reader-canary-open-stages-2026-09-23.json).
The canary lease was released, its temporary token was revoked, and the
production reader remained on its previous image. The exact remaining
broad-index open duration is unknown; schema-9's lazy cold-open implementation
and a passing canary gate are still required before production promotion.

A read-only Axiom review on 2026-09-22 found 30 of 273 Raw search events over
250 ms in the preceding seven days, including a 42.3-second backend call.
Ten missing slow query shapes were added to the shared benchmark matrix. The
timing breakdown and unresolved cause are recorded in
[`slow-searches-2026-09-22.md`](slow-searches-2026-09-22.md). These observations
are failed performance evidence, not a schema-9 performance pass.

## Next autonomous repairs

1. Implement schema-9 structural metadata and lazy exact execution against the
   frozen contract; do not optimize by dropping broad-query membership.
2. Run the full 300-case exhaustive differential and exact Tier-0/Tier-1
   summary oracles against that implementation.
3. Add a single read-only P00 collector and fixture test so future captures do
   not depend on hand-assembled commands.
4. Run paced internal/public/browser baselines against identical corpus and
   source identities, then request an independent verdict at the exact head.
