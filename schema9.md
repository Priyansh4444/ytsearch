# Schema 9 implementation and publication plan

Build an exact, lazy Raw search engine that preserves the complete query result set.
Deliver exact totals, the whole-result overview, and the first page within the measured 200 ms budget on the 2 GiB reader.
Preserve the website contract and complete indexing, publication, activation, recovery, and generation-pinned pagination.
Execute P00 through P11 in dependency order. This document is the planning deliverable, not evidence that schema 9 is deployed.
No plan can guarantee the latency target before measurement. Appendix C identifies the gates that can fail and the permitted repairs.

## How to read this

One box is one unit of work. Every box names the evidence that checks it.
Check a box only when its evidence exists. Evidence means a reproducible artifact tied to a full source SHA and corpus identity.
The body is a how-to. The appendices define the normative contracts, risks, and examples.
The execution playbook is `/home/pronsh/.agents/skills/workflow/playbooks/autopilot-stack.md` because the work is coupled and the user requested external review of this plan first.
The root coordinates branch topology and verification. The operator reviews this plan before execution resumes and reviews the rollout evidence before the explicit production switch.
Existing authorization remains valid within its stated scope. Do not repeatedly ask for authorization already given.
No new goal replaces the existing migration goal. The schema program is its current critical path.

Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

Use Appendix E as the behavioral and algorithm contract, Appendix F as the website checklist, and Appendix G as the evidence format.
A proposed path or command is explicitly marked PROPOSED until P00 or its owner creates and verifies it.
Never execute a guessed Fly command, substitute a different generation, or count an unrun test as passed.
If a gate fails, follow Appendix C and keep the gate unchecked. Repair causes rather than weakening the requirement.
Structural helper tests do not prove that production uses those helpers.

## Program checklist

### Arm the program

- [x] Deliver this plan for the requested external review. Evidence is the resolved exactness and publication review findings recorded in the P00 receipt. Implementation resumed after the operator's explicit approval on 2026-09-19.
- [x] Retain the existing `/goal`. The explicit execution approval binds its next phase to `schema9.md`, P00 through P11, the full exactness contract, and the unit/live/perf verification rule.
- [x] Read `git status --short`, `git log -5 --oneline`, `git worktree list`, and the named planning source files. Save the current identities in the P00 receipt.
- [x] Resolve the actual trunk. This repository uses `origin/master`. The workflow template token `git show origin/main:` does not mean that an `origin/main` branch or repository-local workflow directory exists.
- [x] Read the installed execution and opening-a-PR playbooks from the paths in Appendix D. Use `git show origin/master:<existing-repo-path>` only for files actually tracked on trunk.
- [ ] Schedule a 30-minute audit through an available product task/wait mechanism. Track the real handle. If no recurring mechanism exists, perform an explicit audit between bounded units and report that limitation.
- [ ] Use this audit instruction. Read the plan and active goal, inspect current handles and artifacts, compare each claim with its evidence, repair drift, and send a status message listing PR, owner, head, gate failures, and next action. An elapsed observation timeout alone is not a reason to launch a duplicate build.
- [ ] On a user hold, stop new writes and tell all owners to preserve their artifacts. Record which processes remain live and their exact handles.

### Spawn owners

- [ ] Use P00 as the shared baseline. P01 follows P00. P02 follows P01. P03 follows P02. P04 follows P03. P05 follows P04. P06 follows P05. P07 follows P06. P08 follows P07. P09 follows P03 and P07. P10 follows P08 and P09. P11 follows P10.
- [ ] Keep one owner for the coupled engine integration. Delegate disjoint codecs, tests, and read-only reviews only after the owner publishes shared types. The root owns shared schema constants, protocol registration, and branch topology.
- [ ] Run at most the available number of agents. Ten verification lanes mean ten scenarios, scheduled in batches if necessary. They do not require ten simultaneous agents or ten paid cloud machines.
- [ ] Use isolated worktrees and explicit file ownership. Do not edit another owner's shared file while its tests or provenance measurements run.
- [ ] Retain the throughput checkpoint in every owner receipt. Name blocking first steps, independent workstreams, shared mutable state, and the smallest safe decomposition.
- [ ] Treat phase file lists as starting ownership boundaries. The owner must explicitly add required module registration, shared descriptor, caller, and test paths before editing them. Coordinate shared files with the root; do not leave an integration undone because its path was omitted from a list.
- [ ] P08 changes interaction and requires operator review with screenshots and a short recording. P11 requires the concrete rollout approval specified by the release instructions. Other PRs can reach review-ready with technical evidence.

### PR mechanics, for every PR

- [ ] Start from the current verified parent. Commit selectively with explicit file paths. Preserve unrelated changes and never use `git add -A`.
- [ ] Use one behavior per PR. For each PR run the exact named unit, live, and performance lanes and retain failing cases.
- [ ] Use normal git branches and `gh` if no stack tool is installed. Never treat the workflow placeholder `the stack tool` as a real command.
- [ ] Run relevant format, lint, type, and contract checks before the PR push. Run full CI at the integrated head. Record ignored tests separately.
- [ ] Write the PR around its final behavior, touched files, evidence, known limitations, and rollback. Use a temporary body file with `gh pr create --body-file`.
- [ ] Open ready for review when verified. Never merge an unverified dependency through a verified child.
- [ ] Reconcile review findings against actual code and add root-cause regression fixtures. Preserve review evidence across restacks only when patch identity and dependencies still match.
- [ ] Keep the final source commit clean during artifact builds and measurements. Source edits during a run invalidate its provenance and require a fresh run.
- [ ] Respect an approval-system rejection. Do not bypass a rejected remote upload by another command or tool.

### Verdict and merge, for every PR

- [ ] Have a reviewer who did not write the implementation inspect its diff and receipts. A self-report or green CI badge alone is not the review.
- [ ] Run all ten named live scenarios for the PR against the compiled artifact at its exact head. For library-only phases the live surface is the compiled fixture runner. Save JSON receipts rather than fabricated UI screenshots.
- [ ] For each performance comparison, run trunk first against the same immutable corpus and configuration, then counterbalance independent parent/head processes. Keep cold and warm measurements separate.
- [ ] Re-run changed or invalidated gates after a fix. Do not repeatedly rerun unchanged passing suites without a relevant reason.
- [ ] Root marks a PR review-ready only with a complete receipt. Stack dependent changes on the verified parent and hold actual landing to the operator's requested review process.
- [ ] After a merge, check the actual merged SHA and CI. Never infer deployment from a merge.

### Boot recipe, for every live lane

- [ ] Restore a checkout at the receipt SHA in a new temporary worktree. Confirm the lockfile and no unexpected tracked or untracked build inputs.
- [ ] Build the actual release binary. For P02 use the compiled structural runner. For P03 onward build and reopen a real temporary generation. For P07 onward start the actual Raw and API processes on local ephemeral ports.
- [ ] Use local controlled archives until P10. Reuse the canonical retained corpus read-only for full-corpus work. Never start a production rebuild to create a test fixture.
- [ ] For P08 use Playwright and the installed browser control skill against the real API. For backend lanes use HTTP/CLI requests plus structured process observations.
- [ ] Save evidence under `docs/experiments/search/schema9/<PR>/<source-SHA>/<lane>/` or a retained artifact store linked by digest. Screenshots belong only to visual assertions.
- [ ] For P10 Fly lanes use the verified inventory and existing canary scripts. Confirm the target is the isolated canary before any mutation.
- [ ] For P11 use only the exact approved production targets and artifact identities. Preserve the retained rollback data throughout the switch and observation window.

## Freeze the starting state and acceptance contract (P00)

**Depends on.** None.

**Files.**

- [ ] Inspect and edit `schema9.md` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `TODO.md` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `scripts/benchmark-search-contract.json` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED docs/architecture/lazy-exact/query-contract-v2.md` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED tests/schema9/query-matrix.json` only for this phase. Record the final touched paths.

**Build.**

- [ ] Resolve the saved integration branch and its ancestry. Recover committed work into a persistent worktree. Do not infer that deleted temporary worktrees still contain uncommitted foundation modules.
- [ ] Record the reader generation from authenticated readiness, all image digests, exact machine profiles, corpus hashes, mutation sequence, and retained schema-8 artifact identity. Public health document count alone is insufficient.
- [ ] Freeze the v2 ranking and match-provenance contract in Appendix E. Export current legacy outputs separately and classify every intentional semantic difference.
- [ ] Create the 300-case matrix with the category counts in Appendix G, the complete website traceability table in Appendix F, and explicit expected-error cases.
- [ ] Inventory existing operations commands and distinguish already implemented receipts, publication locks, upload resume, and detached job retry from missing schema-9 work.

**You see.**

- [ ] A reviewer can identify the real active generation, all starting commits, every required query, and every unproven gate from a single receipt. Save the P00 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Validate matrix IDs, exact category counts, schema fields, and absence of unlabelled approximations. Run the existing query-contract tests and a matrix structural check created in this PR. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Restore the recorded commit and compare its tree digest. Save `P00/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Read authenticated readiness and compare generation plus documents. Save `P00/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Validate all retained corpus and schema-8 identities read-only. Save `P00/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Capture canonical query outputs from the current reader. Save `P00/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Capture creator-scoped and global queries separately. Save `P00/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Capture the seven sort contracts and feature-flag states. Save `P00/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Capture current frontend URL and expansion behavior. Save `P00/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Capture queue worker and publisher identities without starting jobs. Save `P00/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Resolve missing temporary artifacts as missing rather than completed. Save `P00/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Produce the machine-readable acceptance checklist. Save `P00/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure the current engine, API, and browser separately for the canonical regression queries; save maximum, p50, p95, p99, sample count, and errors. No schema-9 latency claim is allowed.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P00 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail if any baseline case, corpus identity, source identity, or required runtime profile is missing. Do not compare different corpora as a speedup.

**Review gate.** None. P00 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Settle the structural and ranking forks (P01)

**Depends on.** P00.

**Files.**

- [ ] Inspect and edit `crates/youtube_raw_search/examples/lazy_exact_measure.rs on the saved integration branch` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/examples/schema9_forks.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `docs/architecture/lazy-exact/arena-synthesis.md` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED docs/experiments/search/schema9/P01/` only for this phase. Record the final touched paths.

**Build.**

- [ ] Reuse the corrected existing measurement harness and its provenance/failure accounting. Do not restart its historical defective measurements or accept those values as current evidence.
- [ ] Compare phrase intersection then positions against rare-lead skip verification on the same corpus, including mixed fields, repeated terms, and absent candidates.
- [ ] Compare sparse, dense, and frozen Roaring set cost across actual DF and overlap distributions. Select high-DF field bitmaps by an explicit manifest byte budget; prove an empty bitmap pool remains exact.
- [ ] Measure the anchored moment scan with transcript provenance, five numeric histograms, and title-only cases over the full snapshot.
- [ ] Compare canonical varints plus skips with BP128/Stream VByte blocks of 64/128/256. Choose one baseline codec and keep alternate codecs optional until a repeatable win is proven.
- [ ] Prototype the retained grouped rank frontier against exhaustive ranking. Measure complete-rank bounds, near-tie adversarial data, cross-block videos, per-session state, and deep enumeration.
- [ ] Measure two pinned generations, session admission/expiry, and bounded page replay. Record rejected forks and unresolved requirements.

**You see.**

- [ ] Each representation and frontier choice has exactness, bytes, latency, and failure evidence. No production choice depends on an unmeasured synthetic crossover. Save the P01 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Run independent oracles for each fork, including deliberate wrong-answer and measurement failures. Preserve failed cases and null timings in output. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Measure rare disjoint term sets. Save `P01/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Measure rare overlapping term sets. Save `P01/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Measure dense common terms. Save `P01/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Measure phrase candidates with multiple positions. Save `P01/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Measure repeated-term and cross-field phrase controls. Save `P01/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Measure title-only and mixed-provenance moments. Save `P01/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Measure full-snapshot numeric histograms. Save `P01/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Measure grouped rank ties and partial videos. Save `P01/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Measure deep-page retained state and expiry. Save `P01/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Measure two-generation residency under concurrent queries. Save `P01/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure wall time, process CPU, RSS/HWM, physical read bytes, scratch bytes, and artifact bytes. Use interleaved processes and at least ten paired samples after explicit warmup.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P01 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail a fork on any oracle mismatch, omitted case, unaccounted retained memory, query-plan-dependent artifact growth, or inability to support the required exact semantics. A slow exact baseline may continue as a baseline but cannot pass P10.

**Review gate.** None. P01 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Build exact sets and seekable posting primitives (P02)

**Depends on.** P01.

**Files.**

- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/sets.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/postings.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/tests/sets.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/tests/postings.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_raw_search/src/format.rs` only for this phase. Record the final touched paths.

**Build.**

- [ ] Recover or reimplement the uncommitted typed-set and skip-cursor sketches only after inspecting their actual availability. Move accepted code into the schema9 module tree and avoid duplicate public facades.
- [ ] Use generation and segment identity plus typed DocId/VideoId universes. Validate set inputs once and reject universe mismatch, duplicates, unsorted IDs, and out-of-range IDs.
- [ ] Implement exact AND, OR, and positive-universe subtraction using sparse/dense operations. Keep scalar iteration linear, avoid full-ID expansion of dense inputs, and use u64 byte-cost comparisons.
- [ ] Write canonical doc-delta/frequency blocks with separate positions and 8/64/512/4096 structural checkpoints. Seek from a validated checkpoint with no rewind.
- [ ] Specify one cursor contract. Posting advance returns the current posting if already at or above target; next consumes it. An ordered set iterator may consume on advance only through an explicitly different adapter.
- [ ] Track visited blocks and decoded/scored document identities in test instrumentation. Store conservative structural block statistics for future bounds, never label max frequency alone a complete score bound.

**You see.**

- [ ] Exact Boolean set results match exhaustive membership. Large posting seeks decode at most one base block after directory lookup. Save the P02 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Run exhaustive subset algebra for small universes, randomized larger universes, typed compile-fail tests, u32 boundary cases, malformed input, all skip boundaries, and monotonic cursor tests. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Construct and enumerate empty sets. Save `P02/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Exercise document/video type separation. Save `P02/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Reject equal-size but different-generation universes. Save `P02/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Intersect sparse and dense sets in both directions. Save `P02/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Exercise union and subtraction with tail bits. Save `P02/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Enumerate a huge sparse universe without dense scratch. Save `P02/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Seek at every 8/64/512/4096 boundary. Save `P02/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Seek to the maximum valid ID. Save `P02/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Mix next and decreasing advance targets through exhaustion. Save `P02/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Jump near the end of a corpus-sized posting list. Save `P02/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Compare decoded postings, complete enumeration CPU, seek latency, and owned payload/scratch bytes against a sequential canonical decoder. Use the corpus-sized fixture from P01.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P02 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail any recall mismatch, rewind, duplicated result, sequential O(n log n) iterator, or seek that decodes more than the selected base block without an explained boundary case. Memory receipts must include capacity and scratch, not just logical length.

**Review gate.** None. P02 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Write and reopen the structural schema (P03)

**Depends on.** P02.

**Files.**

- [ ] Inspect and edit `crates/youtube_raw_search/src/engine/builder.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_raw_search/src/engine/storage.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/builder.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/footer.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/verified_units.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/fast_fields.rs` only for this phase. Record the final touched paths.

**Build.**

- [ ] Implement the Appendix E file directory and versioned codecs. Reuse existing archive ingestion, external posting runs, document payload compression, and channel canonicalization.
- [ ] Stream canonical corpus and mutations into an unpublished destination with one captured sequence. Assign deterministic dense IDs in video/time order and persist external-ID mappings.
- [ ] Generate structural postings, skips, positions, fast fields, facets, and bounded term occurrence bitmaps. Remove broad-single construction from the schema-9 build path and exclude all query-plan inputs.
- [ ] Store per-unit BLAKE3 digests and a validated footer/directory. The builder performs a complete structural and checksum validation before marking the candidate publishable.
- [ ] At open read only bounded directories and hot metadata. Verify a unit on its first read, once across concurrent callers; cache corruption failures and reject invalid ranges before borrowing bytes.
- [ ] Reuse mmap only under an explicit immutable-file lifetime guarantee. Publication/cleanup must uphold that guarantee. Support empty files deliberately.
- [ ] Make file enumeration and codec versions one shared schema descriptor used by writer, reader, transfer, and CLI. Keep schema-8 compatibility explicit.

**You see.**

- [ ] A schema-9 generation builds from canonical archives, closes, reopens, and yields the same decoded structural records without opening broad-single.bin. Save the P03 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Run encode/decode and corruption tests for every record. Verify length overflow, endian handling, tail blocks, missing fields, checksum failures, file substitution, and all unit boundaries. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Build and reopen a tiny complete corpus. Save `P03/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Build from a base plus replacements and tombstones. Save `P03/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Reopen empty optional files. Save `P03/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Corrupt an unread unit then touch an unrelated unit. Save `P03/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Touch the corrupt unit concurrently and verify one cached failure. Save `P03/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Reject footer and directory corruption at open. Save `P03/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Reject truncated or extended files. Save `P03/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Open the retained schema-8 reader using its old path. Save `P03/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Build identical input twice and compare canonical identities. Save `P03/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Run a full-corpus cold open in a fresh reader process. Save `P03/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure standalone build wall/CPU/peak bytes, reader cold physical reads, metadata heap, mmap residency, and first-touch hash cost. Reuse P01 codec evidence.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P03 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail if schema-9 artifacts depend on query-plan count, cold open reads 200,000,000 bytes or more, payload decoding is eager, or unit verification skips corrupt bytes. Builder peak must stay below its profile-derived resource gate.

**Review gate.** None. P03 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Execute exact query predicates and retain provenance (P04)

**Depends on.** P03.

**Files.**

- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/query.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/match_set.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/positions.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_raw_search/src/engine/query_support.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_raw_search/src/fuzzy.rs` only for this phase. Record the final touched paths.

**Build.**

- [ ] Reuse canonical tokenization and fuzzy lookup behind the frozen P00 grammar. Return a validated typed AST of Term/And/Or/Phrase/Not/Filter nodes.
- [ ] Preserve every admitted alternative and fallback branch. Enumerate exact prefix/typo alternatives without arbitrary truncation; budget failure must be explicit and never a successful partial answer.
- [ ] Use skip leapfrog for selective intersections and bitmap algebra for dense sets. Keep score evaluation separate from membership and exact count.
- [ ] Retain candidate documents, matching transcript documents, matching title videos, and per-document token/field evidence as defined in Appendix E. Do not derive transcript provenance from the union alone.
- [ ] Verify phrases only after candidate intersection. Handle repeated terms, multiple positions, slop, field boundaries, exclusions, and chunk boundaries according to the frozen grammar.
- [ ] Produce canonical match spans for returned hits. Render highlights from validated token offsets, preserving punctuation/Unicode boundaries and preventing substring-only false highlights.

**You see.**

- [ ] All 300 resolved query predicates return the complete reference membership and provenance, or the same explicit validation error. Save the P04 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Implement an independent exhaustive token/position evaluator and compare set membership, matched fields, fallback mode, normalized query identity, and spans. Include the historical regression queries and threshold boundary fixtures. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Query the and in the globally. Save `P04/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Query this is the under creator scope. Save `P04/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Run quoted phrases and repeated tokens. Save `P04/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Run negations with exact filters. Save `P04/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Run typo and prefix alternatives. Save `P04/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Run createEffect and create effect. Save `P04/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Run convex with con distractors. Save `P04/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Run title-only and mixed-field conjunctions. Save `P04/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Reject unsupported filters and sorts. Save `P04/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Run Unicode, punctuation, numbers, empty and wildcard cases. Save `P04/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure predicate construction, position reads, candidate counts, and scratch bytes separately from ranking and serialization. Compare common and rare query classes to P00.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P04 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail on any missing match/provenance, cross-field phrase, unlabelled partial result, silent parameter fallback, or full-position read for a query that does not require positions under its scoring contract.

**Review gate.** None. P04 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Compute exact totals and whole-result summaries (P05)

**Depends on.** P04.

**Files.**

- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/aggregate.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/histogram.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/fast_fields.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/tests/schema9_summary.rs` only for this phase. Record the final touched paths.

**Build.**

- [ ] Compute document membership cardinality, exact video projection, and transcript-provenance moment counts separately. Use the greedy anchored 20-second reducer and the title-only/wildcard rules in Appendix E.
- [ ] Persist no per-query totals in index artifacts. Return immutable MatchSets and their byte-accounting interface to the future session owner. Actual cache admission and lifecycle are implemented in P07.
- [ ] Implement Tier 0 creator, kind, year, and duration-bucket counts over the whole video set, with explicit video/moment measures and unknown buckets.
- [ ] Implement Tier 1 exact numeric histograms from bitpacked video columns. Specify inclusive/exclusive bucket endpoints, missing values, and values beyond browser safe integers.
- [ ] Implement Tier 2 as a compiled aggregate-job state machine with cancellation, progress, and Pending/Complete/Failed values. HTTP job lifecycle is implemented in P07. Tier 3 is opt-in and cannot contaminate exact fields.
- [ ] Give every facet a canonical explicit filter predicate and test it through the compiled evaluator. Browser clicks and HTTP requests are wired in P08/P07.

**You see.**

- [ ] Totals and every Tier-0/1 bucket equal a brute-force aggregation of the full pinned query result. Loaded page size never changes the overview. Save the P05 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Compare all aggregates with independent map/set scans. Test 0/19999/20000/20001/38000 ms, title-only and mixed matches, duplicate timestamps, unknown metadata, maximum numeric values, and bucket boundaries. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Count a title-only video with many distant chunks. Save `P05/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Count mixed title and transcript matches. Save `P05/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Count anchored 0 19000 38000 moments. Save `P05/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Exclude a chunk that previously anchored a moment. Save `P05/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Aggregate every creator across the full result. Save `P05/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Aggregate videos shorts and unknown kinds. Save `P05/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Evaluate year and duration facet predicates through the compiled runner. Save `P05/lane-07/receipt.json`. Pass when the resulting complete match set equals the independent exact predicate reference, with zero unexpected errors.
- [ ] Lane 8. Compare five numeric histograms to raw rows. Save `P05/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Cancel and resume an expensive Tier-2 request. Save `P05/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Request Tier-3 explicitly and verify approximation labels. Save `P05/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure exact moments, Tier-0, Tier-1 and exact totals plus Tier-0 aggregate cost on the full snapshot. Report all scans and metadata bytes. Integrated response timing is deferred to P10.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P05 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail on any aggregate mismatch, missing unknown bucket, loaded-page-only aggregation, or default approximation. Use a provisional 70 ms maximum for exact totals plus Tier 0 on the full corpus in the P05 compiled runner. P01 must record this sub-budget and P10 enforces the complete 200 ms response budget after ranking/API integration. A missed sub-budget stays a performance finding; do not claim a combined HTTP pass in P05.

**Review gate.** None. P05 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Rank exact pages with a retained frontier (P06)

**Depends on.** P05.

**Files.**

- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/rank_program.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/frontier.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/block_max.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/tests/schema9_ranking.rs` only for this phase. Record the final touched paths.

**Build.**

- [ ] Implement the explicit versioned comparator and deterministic score arithmetic from Appendix E. Keep a simple exhaustive scorer as the correctness oracle.
- [ ] Build conservative full-rank block bounds, including coverage, field evidence, phrase/proximity, text score, engagement or numeric order, and all tie breakers.
- [ ] Use rank-safe block-max traversal only where its bound proof holds. A loose upper bound expands more work; it never lowers the bound or removes matches.
- [ ] Keep partial-video state and canonical document contributions so a video spanning blocks cannot be emitted until its group rank and requested top hits are certified.
- [ ] Retain the unexpanded frontier and scored candidate state across pages. Page k+1 must continue without rescoring consumed documents or replaying exhausted blocks.
- [ ] Spill bounded retained state only if P01 proved the exact format and I/O budget. Reserve growth pessimistically or acquire accounting capacity before every retained-state allocation. If an existing session cannot grow safely, use the documented spill or explicit expiry path before allocation, with no partial committed page.

**You see.**

- [ ] Every page, score bit, video order, and within-video hit order equals the exhaustive v2 reference through exhaustion. Save the P06 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Differentially enumerate all 300 cases at page sizes 1/3/10/20 and test equal-score/numeric ties, multi-block videos, incomplete groups, unsafe synthetic bounds, and floating-point operation order. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Rank all common-word matches through the last page. Save `P06/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Rank exact-token matches above prefixes and typos. Save `P06/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Rank same-chunk create effect against title distractors. Save `P06/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Rank convex above con expansions. Save `P06/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Rank videos whose best coverage and raw score occur in different chunks. Save `P06/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Exercise all seven sorts with equal primary keys. Save `P06/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Place the winning hit in the last block of a video. Save `P06/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Exercise bounds exactly equal to the page threshold. Save `P06/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Replay a page then request the next page and inspect work counters. Save `P06/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Force frontier pressure while checking exact scores and admission. Save `P06/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure page-one latency, deep-page incremental CPU, retained bytes per session, decoded blocks, distinct scored documents, and duplicate scoring counters.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P06 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail any rank/score mismatch, bound that underestimates an actual key, rescoring of consumed documents between pages, or unbounded retained state. Exhaustive fallback is correctness-only and cannot pass the final performance gate by exemption.

**Review gate.** None. P06 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Expose pinned sessions through compatible APIs (P07)

**Depends on.** P06.

**Files.**

- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/session.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED crates/youtube_raw_search/src/schema9/cursor.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_raw_search/src/reloadable.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_search_service/src/lib.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_search_client/src/lib.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_api/src/main.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `src/types.ts` only for this phase. Record the final touched paths.

**Build.**

- [ ] Acquire reader and generation atomically using the existing snapshot lease. Reuse it for pages, summary, and full-video moments.
- [ ] Implement authenticated versioned cursor claims, query fingerprint binding, expiration, instance/session identity, and constant-time authentication through a vetted existing dependency.
- [ ] Implement the exact session and request-idempotency state machine in Appendix E, including duplicate and concurrent same-cursor requests.
- [ ] Bound session/frontier/match-set/replay bytes globally and individually. Preserve active request leases during eviction and return an explicit refresh-required response after expiry.
- [ ] Extend responses compatibly with generation, rank version, cursor, overview, and separate timing fields. Preserve all existing hit/video field shapes.
- [ ] Keep legacy page URLs. A direct deep link may compute preceding windows explicitly once; an active cursor chain must reuse prior work. Preserve browser back-navigation using retained or browser-cached pages.

**You see.**

- [ ] An active-generation change marks an old session stale while its pages and summaries remain internally consistent. Save the P07 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Test token tampering, expiry, wrong query/generation, process restart, duplicated/out-of-order requests, concurrent cancellation, retry-after-response-loss, and active-request leases. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Create a session and load its next page. Save `P07/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Request the same cursor twice. Save `P07/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Issue concurrent same-cursor requests. Save `P07/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Advance generation between two pages. Save `P07/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Advance generation between page and summary. Save `P07/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Expand all video moments from the old session. Save `P07/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Tamper with cursor bytes and query identity. Save `P07/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Expire or evict a session and request refresh. Save `P07/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Restart the reader and use an old cursor. Save `P07/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Load a direct page URL and then navigate back. Save `P07/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure session lock contention, cursor overhead, replay-cache bytes, and combined old/new generation RSS under admitted concurrency.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P07 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail any mixed-generation result, duplicate cursor advance, false count, leaked retired file mapping, silent recomputation after expiry, or memory beyond the admitted cap.

**Review gate.** None. P07 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Preserve the browser and add exact lazy overview (P08)

**Depends on.** P07.

**Files.**

- [ ] Inspect and edit `src/api.ts` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `src/types.ts` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `src/app.tsx` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `src/search-resource.ts` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `src/components/` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `tests/e2e/` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `worker/` only for this phase. Record the final touched paths.

**Build.**

- [ ] Implement the new fields and cursors through existing modular transport/state helpers. Keep request coalescing, abort cleanup, and per-caller request timing.
- [ ] Render exact totals and reserved Tier-0 layout with page one. Show complete-result overview separately from loaded videos/moments and label pending expensive work.
- [ ] Implement clickable canonical facet predicates, explicit stale-results refresh, and cursor continuation. Keep Previous/windowed page numbers/Next and the existing URL contract.
- [ ] Preserve every Appendix F behavior, accessibility role, analytics event and metadata field, creator picker, queue status, recent history, and availability countdown.
- [ ] Use validated highlight spans and preserve lazy video/moment loading, inline errors, permalinks, feedback, and Shorts separation.
- [ ] Retain lazy chunk boundaries and the 30,000-byte initial gzip budget. Do not hide backend latency by changing timing labels.

**You see.**

- [ ] The website behaves as before while exact whole-result summaries and consistent lazy pages are visible. Save the P08 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Run frontend/worker/type tests and add contract tests for decoding, abort/coalescing, pagination exhaustion, highlights, and stale-session refresh. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Exercise avatar swarm on fine touch and reduced-motion inputs. Save `P08/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Exercise creator combobox keyboard and request mode. Save `P08/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Exercise query submit loading error empty and welcome states. Save `P08/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Exercise URL restore deep page back and recent history. Save `P08/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Exercise availability countdown and queue polling. Save `P08/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Exercise exact overview facets and loaded-count separation. Save `P08/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Exercise all sorts and Videos/Shorts/unknown cards. Save `P08/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Exercise player timestamps moments expansion and feedback. Save `P08/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Exercise copied permalinks focus highlights and analytics. Save `P08/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Exercise stale session refresh and frontend gzip budget. Save `P08/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure request completion, first visible result, first exact overview, layout shift, response bytes, and initial gzip bytes on the controlled browser lane.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P08 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail any missing Appendix F behavior, layout shift introduced by the overview, false has_next_page/show-more state, initial JS at or above 30,000 bytes, or canonical visible-result/overview latency at or above 200 ms on the qualified browser lane.

**Review gate.** The operator reviews the interaction before merge.

- [ ] Save the relevant screenshot set and a 30 to 60 second video under P08/review; the operator checks the complete browser contract and records a verdict.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Make publication and recovery use schema 9 (P09)

**Depends on.** P03 and P07.

**Files.**

- [ ] Inspect and edit `crates/youtube_raw_search/src/publisher.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_raw_search/src/remote_publication.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_raw_search/src/admin.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_index_store/src/lib.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_index_store/src/http.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_cli/src/main.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `crates/youtube_cli/src/publication_receipt.rs` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `scripts/fly-publish-raw-generation.sh` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `scripts/lib/fly-publication-job.sh` only for this phase. Record the final touched paths.

**Build.**

- [ ] Register the exact schema-9 descriptor across writer, reader, client allowlist, protocol compatibility digest, persisted job validator, and CLI build receipt.
- [ ] Implement the proposed production-snapshot capture, candidate staging, and identity-checked adoption path in Appendix E. Existing publish does not provide arbitrary canary-to-production artifact adoption.
- [ ] Keep explicit schema-8 job compatibility while jobs or rollback artifacts still exist. Reject incompatible new uploads without making old job state unreadable.
- [ ] Reuse existing publication work locks, verified build receipts, exact incomplete-directory cleanup, and detached job reconnect. Resume the same build/upload; never rebuild solely because SSH observation failed.
- [ ] Fix activation retry after directory rename. Recover the installed generation by its validated destination identity before requiring the old upload directory.
- [ ] Implement durable installation and activation reconciliation from Appendix E. Test crashes between rename, active manifest update, reader swap, mutation mark_published, and job-state persistence.
- [ ] Pin old files while sessions or in-flight requests own leases. Cleanup requires inactive state, rollback retention policy, no leases, and grace completion.
- [ ] Validate queue/daily batching captures one mutation target and acknowledges visibility only after that generation is active. Keep later writes durable for the next batch.
- [ ] Add the required batch-target API and queue-worker integration explicitly. Resolve or reject an older pending job before capturing the new batch target; do not let publish_and_wait silently include later arrivals in a moving target.
- [ ] Apply descriptor-specific zero-length rules to transfer validation. Schema-9 empty optional files must pass upload/activate/restart tests while malformed empty required files still fail.

**You see.**

- [ ] A complete indexing batch publishes exactly one valid generation. Lost replies and process restarts converge on the same activated identity. Save the P09 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Add deterministic fault injection at every publication transition, including retry after a successful activate whose response was lost. Verify no duplicated build, wrong generation, lost durable mutation, or premature acknowledgement. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Publish one replacement and query it. Save `P09/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Publish a deletion and verify old sessions retain their snapshot. Save `P09/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Resume interrupted transfer from completed file receipts. Save `P09/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Reconnect to the same detached build job. Save `P09/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Crash after installed-directory rename and retry activation. Save `P09/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Crash after active manifest but before in-memory swap. Save `P09/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Crash after reader swap before mark_published. Save `P09/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Crash after mark_published before final job receipt. Save `P09/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Keep old sessions open during cleanup attempts. Save `P09/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Queue concurrent new mutations during a captured batch. Save `P09/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Measure upload/build/verification/activation phases separately. Run search load across the complete activation and post-activation window, using union-of-active-interval coverage.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P09 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail any search/admin error, orphaned success, duplicate generation build, acknowledged invisible mutation, unsafe cleanup, or load gate that stops before activation. Reader and builder limits must derive from their actual profiles.

**Review gate.** None. P09 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Qualify the full generation on the 2 GiB reader (P10)

**Depends on.** P08 and P09.

**Files.**

- [ ] Inspect and edit `scripts/benchmark-publication-isolation.sh` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `scripts/run-raw-publication-canary.sh` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `scripts/benchmark-search-contract.json` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED scripts/check-schema9-release.sh` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED docs/experiments/search/schema9/P10/` only for this phase. Record the final touched paths.

**Build.**

- [ ] Freeze the source and corpus identities. Reuse one full schema-9 candidate, and preserve the retained schema-8 generation.
- [ ] Separate synthetic update/delete canary generations from the immutable production-snapshot promotion candidate. Reopen and qualify the promotion candidate after lifecycle tests; their final synthetic generation is not the promotion artifact.
- [ ] Run all 300 differential cases to complete enumeration, with scores/totals/summaries and all supported sorts. Never compare only page one.
- [ ] Run the browser regression suite and concurrent exact API lanes against the same generation.
- [ ] Run a 2 GiB reader canary with the approved temporary builder profile. Include cold open, warmed queries, publication, activation, recovery, and two-generation sessions.
- [ ] Use observed query concurrency plus a stated stress level. Record errors, missing samples, wall/CPU, reader RSS, cgroup memory, swap, OOM events, network, and physical I/O.
- [ ] Build one concise release report that aggregates all gate receipts and fails closed when any receipt is missing or from a different identity.

**You see.**

- [ ] The real full-corpus canary passes exactness, frontend behavior, latency, memory, publication overlap, and recovery at one immutable release identity. Save the P10 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Run the complete workspace, schema-9 unit/differential suites, operational shell regressions, frontend/worker tests, and full CI at the release candidate. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Run the 300-case exhaustive reference comparison. Save `P10/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Run canonical warm queries at concurrency one. Save `P10/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Run the same matrix at measured production concurrency. Save `P10/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Run cold-open plus first queries in a fresh canary. Save `P10/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Run load during build and upload. Save `P10/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Run load through activation and post-activation. Save `P10/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Run generation advance with open sessions and summaries. Save `P10/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Run reader restart and cursor expiry. Save `P10/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Run corruption rejection and verified rollback on the canary. Save `P10/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Run every Appendix F browser contract and timing lane. Save `P10/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Use Appendix G thresholds and measurement boundaries. Record maximum plus percentiles and denominators. Reuse existing load harness only after its activation interval and failure reporting are verified.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P10 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail on any non-exact output, unexpected error, maximum canonical response at or above 200 ms under Appendix G's explicit metric matrix, RSS at or above 1.5 GiB, cgroup memory above 80% of the reader profile, any swap/OOM, or cold-open reads at or above 200,000,000 bytes. Assert exact expected status/code/body for declared negative tests and retain their counts. Change any existing inclusive RSS comparator to the required strict comparison and test the boundary. Do not silently resize the reader to pass.

**Review gate.** None. P10 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Publish the verified release and prove the live switch (P11)

**Depends on.** P10.

**Files.**

- [ ] Inspect and edit `config/fly-production-resources.json` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `docs/deploy.md` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `TODO.md` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `PROPOSED docs/experiments/search/schema9/P11/` only for this phase. Record the final touched paths.
- [ ] Inspect and edit `existing schema-aware deployment scripts` only for this phase. Record the final touched paths.

**Build.**

- [ ] Present the concrete source SHA, images, candidate generation/digests, target machine IDs, compatibility matrix, measured gates, rollback identity, and temporary resource costs.
- [ ] Confirm the deployment/production-switch authority required by the latest release instructions. A planning approval is not the rollout approval.
- [ ] Publish immutable images and deploy the compatible reader/API/worker in the tested dependency order. Do not change the serving generation until the candidate is available and validated.
- [ ] Activate the existing verified schema-9 generation once. Compare readiness, loaded generation, public response generation, documents/videos, and known regression results.
- [ ] Before activation prove the candidate belongs to the authoritative captured production snapshot/job. A changed snapshot, compatibility identity, manifest, or binary invalidates the promotion receipt and requires requalification and any affected renewed approval.
- [ ] Observe daily/queue publication through the new writer-reader path and confirm durable visibility receipts. Keep old rollback artifacts until their separate retention gates.
- [ ] Record final measurements and remaining limitations. Mark TODO complete only where live evidence proves the item. Legacy-resource destruction remains a later explicitly scoped operation, outside this schema plan.

**You see.**

- [ ] Public search and index publication use the verified schema-9 generation with exact sessions and measured performance. Save the P11 observable-result receipt.

**Verify, unit.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Re-run release receipt validation against the deployed image digests and protocol versions. No test result from a different SHA satisfies the gate. Save source SHA, full result, ignored cases, and exit code.

**Verify, live.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked. Ten lanes on available agents at the PR head, in capacity-limited batches.

- [ ] Lane 1. Verify actual deployed images and profiles. Save `P11/lane-01/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 2. Verify authenticated active generation and footer identity. Save `P11/lane-02/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 3. Verify public common-word queries and exact overview. Save `P11/lane-03/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 4. Verify createEffect convex blackhole and coding is dead. Save `P11/lane-04/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 5. Verify creator scope all sorts and highlighting. Save `P11/lane-05/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 6. Verify old open-session continuity across activation. Save `P11/lane-06/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 7. Verify new indexed video visibility after one batch. Save `P11/lane-07/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 8. Verify queued request acknowledgement and daily update state. Save `P11/lane-08/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 9. Verify resource and latency limits throughout the observation window. Save `P11/lane-09/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.
- [ ] Lane 10. Verify rollback readiness and complete release report. Save `P11/lane-10/receipt.json`. Pass when the named scenario matches Appendix E/F and this phase's observable result, with exact assertions and zero unexpected errors.

**Verify, perf.** Tests alone are not sufficient verification. A PR is verified only when its unit, live, and perf boxes are all checked.

- [ ] Metric. Repeat canonical public and internal timings and memory sampling after activation for the defined observation window, including at least one actual publication batch.
- [ ] Probe. Execute the phase's compiled runner or actual HTTP/browser path from the boot recipe. Retain raw samples and resource counters under P11 with the Appendix G schema.
- [ ] Baseline. Measure the verified parent first on the same corpus and profile. If the capability did not exist, compare its canonical exhaustive reference and state that there is no equivalent parent feature.
- [ ] Rule. Fail on any P10 regression. Execute the tested rollback to the retained compatible generation, preserve durable mutations and failure evidence, and repair before another switch.

**Review gate.** None. P11 is not review-gated. Explicit external-action approval requirements still apply where stated.

**Merge.**

- [ ] Record the independent verdict at the exact head SHA, all required unit/live/perf receipts, and resolved review comments.
- [ ] Reconcile parent/trunk changes and reverify affected claims. Root adds this verified change to the reviewed stack; follow the operator's approved landing process.

## Close the program

- [ ] Every implementation and release box has an immutable evidence reference and an independent verdict.
- [ ] Schema 9 is proven end to end from canonical archive and durable mutation through writer, transfer, activation, reader, API, and browser.
- [ ] The retained schema-8 generation and rollback compatibility remain accounted for. No destruction is implied by this plan.
- [ ] All 300 query cases, complete pages, exact summaries, generation consistency, website contracts, and defined performance/resource gates pass.
- [ ] Update TODO and the user-facing report with measured facts, remaining limitations, and exact release identities. Do not mark the larger migration goal complete while its other explicit requirements remain open.

## Appendix A. Prototype evidence

### Inspect saved work before reusing it

Planning checkpoint recorded on 2026-09-13.

| Artifact | Evidence and permitted conclusion |
| --- | --- |
| Main working tree | Last inspected HEAD `f7f2e14` with unrelated frontend/config edits and document deletions. Preserve them. |
| Saved integration branch | `integrate-lazy-exact-reviewed` resolves to `4d36ec8`. Its earlier code checkpoint is `383652f`. Inspect git objects before reuse. |
| Prior stack | Historical receipts say PRs 3, 4, 5 merged at `588dfa7` with combined CI passing. Revalidate remote state during P00. PR 2 was conflicting. |
| Bounded publication fix | Saved commit `c60b3ad` fixes activation-marker error masking and profile-derived builder limits. Verify ancestry in the chosen integration branch. |
| Arena choice | `docs/architecture/lazy-exact/arena-synthesis.md` selects Lucene/PISA structure with field occurrence bitmaps and adaptive derived sets. Its scores are design judgments. |
| Corrected measurement harness | `92a2760` corrects failed timings, injected-mode identity, asymmetric construction work, and within-case execution order. The integrated equivalent is `247ca15`. |
| Synthetic evidence | The committed report under the integration branch records one million synthetic documents, ten paired samples, 24 attempted cases, 20 eligible, four excluded, zero failed. At 1% DF the medians were 22.17 us for bitmap intersection and 158.72 us for sparse skip. This establishes neither a corpus threshold nor HTTP latency. |
| Reader lease experiment | Integration commit `34e8e86` tests real tiny indexes across replacement and release of the old reader. It does not implement sessions, file retention, or resource admission. |
| Historical local gates | `4d36ec8` documents 302 workspace tests passed and six ignored, plus 15 harness tests, frontend build, and 28,735 initial gzip bytes. Those results apply to that saved checkpoint only. |
| Uncommitted structural modules | The interrupted prior turn wrote typed sets, skip postings, and lazy verification in a temporary worktree. That worktree and test logs were absent at the planning checkpoint. Do not mark them implemented unless source and evidence are recovered and verified. |
| Production generation | Earlier authenticated state identified generation 2145. This planning turn did not query live readiness. P00 must verify the current value. |
| Retained schema-8 build | Earlier retained artifact was 8,208,104,746 bytes with 14,024 plans, 8,322,726 documents, and 34,611 videos. Activation failed; the recorded failure was admin unavailability with no OOM. Preserve it and revalidate identity. |

The previous uncommitted module tests are useful design history, not accepted durable source evidence.
Do not restart or reconstruct the retained schema-8 build just to recover temporary source code.

### Keep these questions open until P01 proves them

- Full-corpus moment and provenance scan below the combined first-response budget.
- Phrase/proximity cost when every admitted candidate gets complete scoring.
- Complete grouped-rank bounds and a retained frontier below the memory ceiling.
- Frozen Roaring versus dense/sparse structures under actual overlap and page-fault distributions.
- Fast-field histogram costs on the 2 GiB reader.
- Codec block size and physical I/O under cold access.
- Concurrent generation retention, cursor replay, and session admission under the real request mix.

A plan can state a contract before those experiments. It cannot label the contract achieved.

## Appendix B. Alternatives rejected

| Alternative | Reason and permitted replacement |
| --- | --- |
| Reuse schema-8 broad-single as the terminal engine | Its storage follows query plans and activation reads too much. Preserve it only for rollback/trial history. |
| Precompute answers for the or other popular strings | Breaks structural-only indexing and continuous growth. Use occurrence sets, skips, columns, and complete bounds. |
| Ignore stop words or trim expansions to make tests fast | Changes membership or ranking. Use cost-based execution while retaining the admitted query semantics. |
| Use raw document popcount for moment totals | Loses transcript/title provenance and anchored 20-second grouping. Use the exact reducer. |
| Keep only per-video first N matches | Loses deeper moments, pages, or best-ranked hits. Keep exact membership and lazy expansion. |
| Use only BM25 as the block upper bound | Omits other comparator dimensions and boosts. Use a complete conservative bound. |
| Discard blocks that cannot win page one | Those blocks may contain page-two results. Keep them in the retained frontier. |
| Rerun WAND with a score cutoff on every page | Repeats earlier work and does not satisfy the no-rescan contract. Resume retained state. |
| Keep arbitrary cursor history forever in RAM | Violates bounded memory. Use bounded replay plus browser history, explicit expiry, or a proven spill design. |
| Give each query the active generation on each request | Mixes totals/pages after indexing. Use one pinned lease per session. |
| Verify whole artifacts during reader open | Can stall activation and cold queries. Verify footer/directory at open and payload units on first use. |
| Remove SQLite during the schema change | It is the durable mutation/idempotency journal, outside the public scoring path. Preserve it unless separate evidence justifies a replacement. |
| Increase the reader to hide a failed 2 GiB gate | Changes the acceptance profile. Report the failed gate and optimize or obtain an explicit revised requirement. |
| Build a second publication/receipt controller | Existing locks, receipts, upload resume, and detached job reconnect already cover that role. Extend the shared path. |

## Appendix C. Risks and autonomous repair instructions

### Use one failure loop

1. Save the failed request, seed, full source SHA, generation/corpus identity, machine profile, command, exit status, and unabridged failure log.
2. Classify the failure before editing. Use the table below.
3. Reproduce it on the smallest fixture that still fails. A transport timeout is not a relevance mismatch; an old-schema ranking mismatch is not a transport failure.
4. Add the regression fixture at the boundary that caused the failure.
5. Change the representation, state machine, bounds, or accounting responsible for the failure. Keep unrelated modules untouched.
6. Run the failing case, its owner suite, and every downstream gate that the changed invariant can affect.
7. Compare results and resource use at the same identities. Commit the repair with its evidence.
8. If three materially different local repairs fail to satisfy the same requirement, preserve the attempts and escalate the unresolved design decision with measured data. Do not repeat the same retry indefinitely or relax the gate.

| Failure or risk | Diagnose | Repair the cause | Reverification |
| --- | --- | --- | --- |
| Missing matches | Compare normalized AST, term alternatives, posting IDs, provenance, and final set in that order | Fix tokenization, skip boundary, set algebra, phrase verification, or fallback enumeration | P04 membership plus P05 totals and P06 all-page oracle |
| Wrong exact count | Compare transcript subset, title videos, wildcard mode, and sorted timestamps | Repair provenance or greedy anchor logic; never substitute adjacent-gap clustering | Boundary fixtures, every bucket, full snapshot |
| Title matches become false moments | Inspect expanded title VideoIds and matches_text evidence | Preserve title and transcript provenance through conjunction/negation and projection | Title-only, mixed-field, exclusions, wildcard |
| Rank differs only on broad queries | Test 128/129 and 50,000/50,001 matches, over four and over ten tokens | Remove path-dependent semantic omissions in v2; retain explicit legacy diagnostics | Versioned scoring oracle and reviewed relevance cases |
| False safe pruning | Log actual full keys and alleged upper bounds for skipped units | Raise or recompute bounds conservatively, include missing dimensions, or temporarily expand the affected blocks | Deliberately adversarial bound tests and complete pages |
| Float score bits differ | Compare ordered per-token contributions and rounding steps | Fix operation order or outward bound rounding; do not use epsilon equality | Bitwise v2 scores, legacy-difference classification |
| Page duplicate or missing video | Inspect cursor sequence, committed replay record, emitted IDs, pending groups | Serialize session transition, make duplicate requests idempotent, retain deferred work | Concurrent retries through final page |
| Session memory grows with pages | Account for heaps, candidate tables, emitted IDs, replay pages, sets, scratch, mappings | Bound admission/state, compress emitted identity sets, or use proven bounded spill | Deep enumeration and repeated old-generation sessions |
| Generation changes mid-session | Compare lease identity at page, summary, and moments requests | Carry one atomic snapshot lease through every path; remove active-reader reacquisition | Activation between each pair of request types |
| Old files disappear under a reader | Inspect cleanup ownership and active leases, including process/machine boundaries | Couple retention to leases and rollback policy; centralize deletion authority | Delayed requests and cleanup/crash tests |
| Common phrase exceeds 200 ms | Break down set construction, position verification, exact moments, ranking, serialization, proxy time | Improve data layout, streaming, skips, vectorization, bounds, and request hops without dropping semantics | Full exact matrix on the same 2 GiB profile |
| Cold open reads too much | Measure actual physical read bytes and accessed units, not only hash counters | Remove eager decode/hash; bound footer tables/hot metadata; defer units | Fresh-process cold lane and corruption checks |
| Hashing causes first-query spike | Record unit size and number of touched units | Choose a proven unit size, improve locality, prefetch only bounded structural hot metadata | Cold first query plus residency and complete verification |
| Writer exceeds RAM/disk | Inspect run sizes, merge fan-in, dictionaries, sort buffers, scratch lifetime | Stream external runs, cap buffers/fan-in, reclaim only verified scratch | Full build with peak resource and failure/restart lanes |
| Activation retry says files are missing | Check whether incoming directory already became the installed generation | Resolve exact installed manifest before requiring incoming files; validate recovered artifact | Every crash point and repeated successful activation |
| Old job prevents startup after schema bump | Inspect persisted job format and file list | Retain explicit schema8-v1/schema8-v2 descriptors and converters; preserve WAL | Startup with each legacy persisted state |
| Upload or SSH polling fails | Verify original job handle, receipt, completed file list, and artifact identity | Resume same job/file transfer; preserve mounted output and lease material | Lost upload response and interrupted observer |
| Indexed data acknowledged too early | Compare durable mutation target, active generation sequence, and queue receipt | Acknowledge search visibility only after activation; retain next-batch writes | Update/delete/restart and queue-batch tests |
| Frontend highlights wrong substring | Compare canonical token offsets, Unicode boundaries, and rendered spans | Build highlights from verified match spans; escape text and merge only valid overlaps | convex/con, pronoun I, CSS phrase, punctuation, Unicode |
| More button or Next at end | Compare exact group total, loaded hits, total videos, and cursor exhaustion | Derive visibility from authoritative totals/exhaustion, not response array length alone | Last page and full-video expansion under retries |
| Benchmarks unexpectedly look perfect | Check removed failures, substituted dataset, warmed cache, omitted network, samples, and counters | Repair the measurement harness and rerun; retain rejected results | All denominators and controlled parent/head trials |
| Tests fail on localhost permission | Inspect actual socket bind failure | Use authorized local socket access for mock servers; do not modify search code | Original socket tests |
| Frontend build lacks Vite | Inspect installed dependencies and lockfile | Install frozen dependencies or reuse an identical dependency tree deliberately | Build and budget check at the target SHA |
| A temporary worktree vanished | Inspect branches and real process handles before rebuilding | Restore committed source; classify uncommitted data as missing; preserve existing remote jobs | Source identity and clean checkout |
| Required remote upload is rejected | Read the automatic review reason and exact destination | Finish local verification, present the specific payload/destination for approval, and wait for that action | Confirm remote SHA only after authorized push |
| Exact workload cannot fit the latency budget | Confirm optimized lower-level work and realistic cold/concurrency costs | Present the remaining lower bound and a concrete revised architectural option | External design review; gate stays failed until requirements are met or explicitly revised |

### Recognize requirements that cannot be guaranteed by declaration

Exact ranking may need to score many nonwinning candidates when safe bounds are loose.
The phrase "score only returned windows" means scoring work is requested by page retrieval and is separate from counts.
It cannot literally guarantee that only returned documents are ever scored while also proving the exact best page.
The reviewer must accept this interpretation or specify a different achievable contract before P06.

Arbitrary infinite session lifetime, unlimited back-navigation, zero rescans, and bounded RAM cannot all hold without extra retained storage.
This plan chooses bounded sessions and explicit expiry, with already loaded browser pages retained locally.
If arbitrary old-page server replay is required after cache eviction, P01 must prove a bounded spill design before P07.

A 200 ms browser guarantee from every global network location cannot be inferred from a 200 ms server result.
The release gate fixes the browser location/network and reports both server and public timings.
If the user requires a global geographic SLO, add those locations explicitly and fail that larger gate when any location misses it.

In the worst case, exact phrase verification and query-dependent moments require inspecting all matching positions/timestamps.
If complete scoring or summaries cannot meet the budget on 2 GiB, do not hide that by omitting work or changing numbers.
The plan contains the repair process but cannot promise that an arbitrary workload has a feasible sub-200-ms implementation.

## Appendix D. Links, commands, and reading list

### Read the actual integration points

| Concern | Existing source |
| --- | --- |
| Chosen design | [Architecture sketch](docs/architecture/lazy-exact/design.md) and [arena synthesis](docs/architecture/lazy-exact/arena-synthesis.md) |
| Query behavior | [Engine](crates/youtube_raw_search/src/engine.rs), [query support](crates/youtube_raw_search/src/engine/query_support.rs), [conjunction](crates/youtube_raw_search/src/engine/conjunction.rs) |
| Reader/open | [Storage](crates/youtube_raw_search/src/engine/storage.rs), [format](crates/youtube_raw_search/src/format.rs), [reloadable reader](crates/youtube_raw_search/src/reloadable.rs) |
| Index build | [Builder](crates/youtube_raw_search/src/engine/builder.rs) |
| Publication/recovery | [Publisher](crates/youtube_raw_search/src/publisher.rs), [remote publication](crates/youtube_raw_search/src/remote_publication.rs), [admin](crates/youtube_raw_search/src/admin.rs) |
| Shared protocol | [Index store](crates/youtube_index_store/src/lib.rs) and [HTTP client](crates/youtube_index_store/src/http.rs) |
| CLI resume | [CLI](crates/youtube_cli/src/main.rs) and [publication receipts](crates/youtube_cli/src/publication_receipt.rs) |
| Browser contracts | [Wire types](src/types.ts), [API](src/api.ts), [app](src/app.tsx) |
| Live targets | [Resource inventory](config/fly-production-resources.json) |
| Existing query lanes | [Benchmark contract](scripts/benchmark-search-contract.json), [cases](scripts/benchmark-search-cases.tsv), [typo cases](scripts/benchmark-search-typo-cases.tsv) |

Installed skill paths are machine-local instructions, not files guaranteed on remote trunk.
Read `/home/pronsh/.agents/skills/workflow/SKILL.md` and its `playbooks/multi-phase-plan.md`, `playbooks/autopilot-stack.md`, `playbooks/opening-a-pr.md` and `references/principles/prove-it-works.md`.
If those paths differ, discover the installed skill location first.

### Use existing verification commands

These commands exist in the inspected checkout. Verify package definitions at the target SHA before execution.

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
bun run test:frontend
bun run test:worker
bun run test:e2e
bun run typecheck
bun run check:docs
bun run build
bun run perf:bundle
bun run perf:worker
bun run check:all
```

New schema tests and the aggregate release gate are PROPOSED. P00/P02/P10 must create and register them.
Suggested interfaces below are specification targets, not commands that already exist.

```text
PROPOSED cargo test -p youtube_raw_search --test schema9_query_oracle
PROPOSED cargo test -p youtube_raw_search --test schema9_summary
PROPOSED cargo test -p youtube_raw_search --test schema9_ranking
PROPOSED cargo test -p youtube_raw_search --test schema9_publication
PROPOSED scripts/check-schema9-release.sh <receipt-directory>
```

If Cargo nextest is installed, use it for compatible test binaries. Also run doctests through Cargo because a compile-fail type test is not covered merely by nextest.

### Reuse the real publication commands

The current public operation is `ytsearch publish --backend raw`.
Current Fly publication wrapper accepts that operation. Stage-only or retry-upload CLI modes are PROPOSED until implemented.
Use the existing verified-build receipt to avoid repeating a completed build.

These existing commands can mutate external systems and are not planning-time checks.

- `scripts/build-raw-search-overlay.sh` and `scripts/build-raw-indexer-overlay.sh` can build/upload private source and images.
- `scripts/stage-raw-reader-canary.sh` defaults to inspection. Its apply token is `APPLY_RAW_READER_CANARY=stage-exact-isolated-canary`.
- `scripts/reconcile-raw-publication-builder.sh` defaults to inspection. Its apply token is `APPLY_RAW_PUBLICATION_BUILDER=configure-exact-inert-builder`.
- `scripts/run-raw-publication-canary.sh` requires `ALLOW_RAW_CANARY_GATE=run-exact-isolated-canary` and exact resolved image identities.
- `scripts/benchmark-publication-isolation.sh` with `ALLOW_PUBLISH_BENCHMARK=1` starts publication and is not a read-only benchmark.

Inspect the scripts' current usage and required environment names.
Do not paste secrets, cookies, tokens, or complete environment dumps into logs.
Use existing loaders and private header files.

### Apply the planning workflow without invented infrastructure

The installed plan checker contains an obsolete hardcoded model label.
Run the original checker and retain its output. If its only model-routing failures are the unavailable model label, run a temporary compatibility copy that changes only that label to the available-agent lane wording used here.
Do not alter requirement counts, verification predicates, or acceptance thresholds to satisfy a checker.
The compatibility copy and output are part of the plan review receipt.
Backend live lanes save structured output. A static screenshot of a terminal does not prove a backend invariant.

## Appendix E. Normative engineering contracts

### Define the identity boundary

Use these logical types. Final module names may change only with an updated map and tests.

```rust
struct GenerationId(u64);
struct SegmentId(u32);
struct CorpusDigest([u8; 32]);
struct DocId(u32);
struct VideoId(u32);
struct ExternalVideoId(String);
struct Universe<Id> {
    generation: GenerationId,
    segment: SegmentId,
    length: u32,
    marker: PhantomData<Id>,
}
struct QueryIdentity {
    grammar_version: u16,
    rank_version: u16,
    normalized_terms_and_ast: CanonicalQueryBytes,
    explicit_filters: CanonicalFilterBytes,
    sort: SearchSort,
    scope: CanonicalCreatorId,
    generation: GenerationId,
}
```

Do not use length alone as a universe identity or dense IDs as external stable tie breakers.
A session cache key includes all grammar, rank, field, prefix, typo, phrase/slop, explicit filter, sort, and generation options.
If one option can change membership, score, summary, or spans, it belongs in the key.
Use canonical binary encoding with lengths and tags, not ambiguous concatenated strings.

### Define match sets and exact moments

A match result must retain at least the information below.

```rust
struct MatchSet {
    candidates: DocSet,
    transcript_matches: DocSet,
    title_matches: VideoSet,
    matched_videos: VideoSet,
    mode: MatchMode,
    // Lazy or compact per-candidate token/field evidence for the scorer.
    evidence: MatchEvidence,
}
enum MatchMode { Query, Wildcard }
struct ExactTotals {
    matched_documents: u64,
    videos: u64,
    moments: u64,
}
```

The same candidate document may receive title and transcript contributions.
For mixed-field conjunctions, provenance means the fields that actually contributed to an admitted match after exclusions.
Do not independently union all title terms into provenance after intersecting the final predicate.
Verify `transcript_matches` is a subset of the final candidate universe.

For a query-mode result, project matched videos exactly.
For each matched video, scan the selected transcript timestamps in nondecreasing order with stable document tie order.
Start with no anchor. Start a moment on the first timestamp.
Start another only when `timestamp - anchor > 20_000`, then replace the anchor.
The contribution for a matched video is `max(1, transcript_cluster_count)`.
The one-count floor represents a title-only matched video.
Wildcard mode follows the frozen compatibility contract of one visible moment per video.
Never use the number of expanded title documents as its moment count.

Required vectors include these exact outcomes.

| Selected transcript times in one matched video | Moments |
| --- | --- |
| None, title match only | 1 |
| 0 | 1 |
| 0, 0, 19,999, 20,000 | 1 |
| 0, 20,001 | 2 |
| 0, 19,000, 38,000 | 2 |
| 0, 20,000, 40,000 | 2 |
| 19,000, 38,000 after excluding time 0 | 1 |

The builder must prove video/time locality if the reducer relies on dense ID iteration.
Otherwise use exact ordered merging of per-video/time streams. Do not assume document insertion order is chronological.

### Specify grammar and scoring before changing execution

P00 produces a complete grammar table from actual accepted request fields.
Include quoted phrases, negated terms/phrases, Unicode normalization, duplicate terms, numerical typo rules, per-field prefix behavior, wildcard/empty cases, and unknown-parameter rejection.
Explicit phrases cannot cross title/transcript fields or transcript chunk boundaries unless the contract explicitly allows cross-chunk matching and stores the required structure.
Malformed quotes and invalid numeric parameters must produce a documented validation error.

A query with no positive universe must either have an explicit full-universe predicate or be rejected.
Never accidentally implement global NOT by subtracting from an empty set.
Treat operator-provided filters as predicates, never as optional hints.

Ranking has two references.

1. The historical legacy path documents observed behavior and permits intentional differences only through an explicit versioned expectation.
2. The schema-9 v2 reference computes the entire specified score independent of cardinality, codec, skip path, or bitmap availability.

The current legacy comparator starts with exact-token count, exact-mask, ordered-phrase flag, f32 score, and external video ID.
Numeric sorts compare their numeric key, then relevance score, then external video ID.
The engagement sort uses coverage/mask/phrase, blended score, raw score, and external video ID.
The best video tuple comes from one real best document. It is not a tuple assembled from unrelated documents' independent maxima.

The v2 default proposal preserves the text relevance priority and adds engagement only after the complete text key.
Freeze this tuple in P00 and have the external reviewer approve or explicitly amend it.

```text
default relevance
  exact distinct query-token coverage, descending
  exact token-evidence vector in canonical query order, lexicographically descending
  ordered-phrase preference, descending
  complete text score, descending
  normalized generation-wide engagement, descending
  external video ID, ascending
within a video
  the frozen document relevance key
  timestamp, ascending
  external chunk ID, ascending
```

A longer query needs an exact evidence vector. The existing four-bit mask cannot represent arbitrary token counts.
Repeated tokens inside phrases remain positional constraints even if unquoted repeated tokens normalize to one query term.
No scoring contribution may silently stop after ten terms or 50,000 matches.
Do not preserve those execution-path omissions while claiming complete v2 scoring.

Preserve existing scoring constants until a separately reviewed relevance change modifies them.
Record current text/title TF-IDF, first-position, phrase/proximity, and engagement formulas and exact constants in the v2 contract.
The current title IDF estimate uses average documents per video. Replacing it with another statistic is a semantic change and must get explicit v2 expected outputs.
Engagement normalization uses generation-wide maxima, never the current result set's maxima.
Missing numerical metadata and nonfinite score values must have explicit validation semantics.

Accumulate contributions in canonical normalized query order with specified f32 operations.
P00 must also freeze the within-token reduction table before P06 starts.
Give each admitted contribution an identity of query slot, indexed term bytes, field, expansion branch, and split component.
Enumerate alternatives by canonical term bytes and then frozen expansion-kind order; enumerate text before title within an alternative.
Record whether overlapping branches represent distinct additive contributions or one deduplicated semantic contribution.
Preserve the current sum behavior for distinct union contributions unless the v2 judgment explicitly changes it.
Deduplicate only repeated instances of the same canonical contribution identity, never different terms merely because they hit the same document.
Apply each expansion/field multiplier at the specified contribution stage, reduce in the frozen order, and apply document-level proximity/phrase factors at their specified stage.
Export this table and the actual constants/formulas into query-contract-v2.md with exact golden score bits.
Test a document matching several alternatives in both fields and permute execution order while keeping the same canonical reduction.
An unresolved sum-versus-max or multiplier-order cell is a failed P00 contract gate, not discretion for the optimizer.
Forbid reassociation and opportunistic FMA if they change reference bits.
Use `total_cmp` consistently for finite scores. Compare exact bits where the specification requires bit equality.
Do not accept epsilon differences for ordering at a page boundary.
Bounds use outward rounding or conservative representable upper values.

The other six v2 sorts are normative as follows.
Newest and Oldest compare publication timestamp descending/ascending, then complete v2 raw relevance score descending, then external video ID ascending.
Most viewed, Most liked, and Most discussed compare their named numeric field descending, then complete v2 raw relevance score descending, then external video ID ascending.
Relevant plus engagement compares full exact coverage/evidence/phrase priority, then the frozen blended score, then complete v2 raw relevance score, then external video ID.
All video-level sorts use the same v2 document relevance order for moments inside each video; numeric video order does not reorder its moments by popularity.
Missing numeric values follow the frozen fast-field missing-value policy and cannot become NaN score keys.

An ordinary query may need proximity positions to compute its complete score.
The old requirement of no remaining-position reads for every non-phrase query conflicts with complete unquoted proximity scoring unless a sufficient positional summary is available.
P01 must choose a proven structural summary or accept lazy positions for scored candidates under the defined scoring mode.
Never skip the boost merely to satisfy a read counter.
P10 must report position reads separately for predicate-only, phrase, and proximity-scoring paths.

### Bound the complete rank and retain deferred work

Give every block enough structural metadata to derive a conservative upper bound under the chosen rank program.
A max-frequency statistic alone is insufficient.
Cover token presence, field weights, phrase/position multipliers, normalization lengths, engagement/numeric fields, and tie-breaking dimensions.
If a dimension lacks a tight proved bound, use a safe global upper value and record the lost pruning.

Prefer a retained best-first block frontier for the first exact implementation.
Canonical score evaluation remains a separate deep module.
An alternative BMW/MaxScore implementation can replace it only after matching the same page and no-rescan tests.

For every page request follow this algorithm.

1. Reuse the immutable match/provenance sets and exact totals.
2. Look at the highest possible full key among deferred blocks and partial groups.
3. Expand work that can outrank the best uncertified candidate or its selected within-video hits.
4. Score each newly consumed document once in canonical contribution order.
5. Update the best real document key and bounded required hit state for its video.
6. Certify a candidate only when every remaining competing bound proves it cannot beat the candidate under all comparator dimensions.
7. Emit each video once. Retain deferred blocks and un-emitted candidate/group state for later pages.
8. Stop after the requested certified window. Do not discard low-ranked matches or reset the frontier.
9. Derive `has_next_page` from exact remaining video cardinality and emitted state.
10. Keep count/summary scans separate from scorer counters so a popcount is not misreported as zero search work.

For equal bounds, expand unless the complete tie-break order proves the candidate wins.
A block skipped for page one must remain available for page two.
The exact group rank and its initial top hits both need certification.
Full-video moment expansion may perform separate lazy work, but must not silently alter the video rank or totals.
State explicitly which previously evaluated score evidence it reuses.

No-rescan means no repeated scoring or replay of exhausted rank postings to advance one live page chain.
It does not mean that the OS can never refault an mmap page or that a first direct deep URL does zero initial work.
Keep these counters distinct.

### Build a structural file format

Use a versioned directory. Do not serialize native Rust struct layout.

| File family | Required contents |
| --- | --- |
| Footer/directory | Schema and codec versions, generation/corpus identity, file lengths, unit sizes/digests, counts, normalization/rank versions, bitmap budget, directory checksum |
| Term dictionary | Canonical UTF-8 term to field-specific posting descriptors; deterministic lookup order |
| Posting doc/frequency stream | Independently addressable blocks with count, base doc, deltas/frequencies and codec tag |
| Skip directory | Validated offsets/ordinals for each level, monotonic IDs, bounded block references |
| Block rank metadata | Conservative structural statistics sufficient to derive full rank bounds |
| Occurrence bitmaps | Bounded field-specific high-DF term sets; directory-selected by corpus data only |
| Position streams | First positions and complete remaining occurrences addressed separately by term/field/doc |
| Document fast fields | Video ID, timestamp, length, field flags, stable external document mapping |
| Video fast fields | Creator, kind, year, duration, publication time, views, likes, comments, missing-value bits |
| Facet sets | Exact creator/kind/year/duration membership over video IDs |
| Document payload units | Independently compressed text/title/neighbor data for returned hits |
| Channels/external IDs | Canonical metadata and deterministic external-ID lookup |
| Integrity units | 2 to 8 MiB bounded checksums with correct short final unit handling |

P03 chooses and documents exact byte widths, tags, endian order, signedness, limits, and offset base for every field before implementation.
Require little-endian fixed integers and checked arithmetic throughout.
Use u64 file offsets where files may exceed 4 GiB. A u32 document ID does not imply a u32 byte offset.
Record zero-length file rules, optional sections, duplicate term rejection, and maximum decoder output.
A compressed unit must declare its exact uncompressed length and obey a reader allocation cap before decompressing.

Structural file size may grow with documents, terms, positions, facets, and configured structural metadata.
It must not grow because another benchmark query or fallback plan was observed.
Test this by building the same corpus with different or absent query logs and comparing structural identities.

Footer-only open validates bounded metadata and file lengths.
Lazy integrity verifies all units intersecting a returned byte range before decoding it.
A unit state is Unchecked, Valid, or Corrupt and is synchronized so overlapping first reads hash it once.
A cached successful checksum does not authorize modifying the backing file.
Never mutate or truncate any mapped generation file.
Cold-open physical I/O must be measured independently of the bytes-hashed counter.

### Keep sessions bounded and consistent

A session owns the reader lease, normalized query, MatchSet/provenance, exact totals/Tier 0, frontier, replay state, and accounting.
Account for all memory before admission, including heap capacities, bitmap containers, pending groups, cached pages, compressed/decompressed scratch, and shared-generation attribution.
For later frontier growth either reserve a proved worst-case capacity at admission or acquire additional byte reservations atomically before each allocation.
Allocation and page-state commitment must be transactional with that reservation. Release unused reservation on cancellation or failure.
If growth exceeds capacity, use the P01-verified spill path or explicitly expire the session before allocating; rejecting only new sessions does not protect an existing growing session.
Inject memory pressure halfway through enumeration and assert no partial page is committed, no response mixes generations, and no hidden rank restart occurs.
Use an operator-visible total byte budget and per-session cap.
Choose initial limits from P01 and record them in configuration. Do not invent a hidden constant to make a fixture pass.

A cursor authenticates version, instance/session ID, generation, query identity, sequence, page size/order identity, and expiry.
Use a vetted MAC or authenticated token implementation and constant-time verification.
Do not generate an ad hoc cipher or trust client-provided offsets, scores, or serialized heaps.
Keep keys out of browser bundles, URLs printed in logs, and source.
A server restart may explicitly expire all in-memory sessions.

The session request state machine is Ready, Computing(sequence), Committed(sequence, replayable response), and Expired.
Concurrent duplicate requests join the same computation or read its committed replay result.
A lost response must not advance the frontier twice.
Commit page response metadata and frontier progress together under session ownership before returning the cursor.
A cancelled waiter does not corrupt the shared page computation.

Replay storage is bounded.
Keep browser-loaded pages in navigation state. For an expired server replay window return the documented refresh-required response instead of secretly rebuilding.
If the requested URL names page N without a session, start a new consistent session and enumerate preceding windows once.
Do not merge those results with an older stream.

When generation advances, existing sessions continue on their leased generation.
Return `results_may_have_changed=true` when it differs from active and show an explicit refresh action.
The overview always states the pinned generation.
New sessions use the new generation.

Cleanup is legal only after a generation is inactive, outside required rollback retention, unleased by sessions or requests, and past grace.
In-process Arc counts do not protect files from a separate publisher process.
Define one authoritative deletion coordinator with durable retention evidence for every process that can map files.
Test multiple replicas or explicitly constrain session routing to one reader and expire sessions honestly on failover.
Do not claim replica failover support from a single-process lease test.

### Define summary units and response evolution

Tier 0 must report full-set exact totals plus creator/kind/year/duration facets.
Every facet count declares its unit. Default facet membership is videos; include exact moments only when computed by the same reducer.
Unknown values get explicit buckets and matching predicates.
For a single-valued dimension, bucket video totals equal the full video total.

Tier 1 includes views, likes, comments, recency, and duration.
Bucket intervals are lower-inclusive and upper-exclusive, with explicit underflow/overflow/missing handling.
Use integer arithmetic where possible and preserve numeric precision across JSON.
For u64 values beyond JavaScript safe integers, use a documented string representation or reject invalid source values according to the schema contract.

Tier 2 runs only when requested and returns Pending/Complete/Failed with a generation-bound job identity.
Tier 3 requires an explicit approximate request and labels every approximate result.
Neither may overwrite exact Tier-0/1 numbers.

Preserve `query, total_moments, total_videos, totals_approximate, total_found, out_of, time_to_first_result_ms, request_duration_ms, page, per_page, sort, mode, has_next_page, hits, videos` and existing hit/video shapes.
P00 must freeze the current units of `total_found` and `out_of` rather than infer them from their names.
Add fields compatibly for generation, rank version, cursor, overview status, and timings.
Do not assign engine-only time to a browser request-duration field.
Each coalesced caller gets its own observed request duration.

### Make publication recoverable at every transition

Storage schema, transfer protocol, rank version, grammar version, and persisted job version are separate concepts.
Changing storage schema to 9 does not automatically change protocol version 1.
Version only changed wire semantics and prove explicit old/new combinations.

Use one shared descriptor for the current schema and immutable named descriptors for retained legacy formats.
The current schema8-v2 format is recognized through a current-format constant in existing code.
When that constant changes, add an explicit legacy schema8-v2 descriptor or startup will reject old jobs.
Keep the prior v1 variants and exact old 12-file allowlists where required for state recovery.

Reuse existing publication mechanisms.

- Work lock prevents concurrent build workspaces.
- The existing mounted build receipt binds job, snapshot sequence/hash/length, compatibility, and output manifest identity. It does not itself bind source SHA or image digest.
- P09 must add or link an immutable source/image release attestation and validate it with the existing artifact receipt. Do not describe that proposed source binding as already implemented.
- Upload resume skips completed verified files. Current interrupted-file retry restarts that file; it is not byte-range resume.
- SSH polling reconnects to the same detached job. A missing observation is not permission to launch another.
- Local cleanup requires confirmed activation and matching identity, then uses the existing reclaim protocol.
- WAL remains authoritative for durable mutations and captured publication sequences.

Implement and test these durable states, either as explicit state variants or exact reconciled facts.

| State | Evidence or reconciliation fact | Allowed next operation |
| --- | --- | --- |
| Prepared | Job identity and snapshot receipt | Build/resume matching candidate |
| Built | Complete source-bound output receipt | Upload missing files |
| Uploaded | Exact manifest and all verified file receipts | Install candidate |
| Installed | Exact destination generation manifest | Validate/open candidate and activate pointer |
| Pointer active | Durable active generation identity | Reopen/swap reader if needed |
| Reader active | Serving reader identity equals pointer | Mark captured mutation sequence published |
| Published | WAL publication sequence reconciled | Persist Activated job and return receipt |
| Acknowledged | Client verified active receipt | Reclaim eligible build scratch |

Reader active is an in-memory observation, not a durable record. Re-establish it from the durable pointer and validated artifact after restart.
Schema-specific file descriptors decide whether an empty file is valid. The current receiver rejects zero-byte files universally; P09 must change that for valid schema-9 optional sections and test transfer, activation, and restart.

Do not require the incoming upload directory once Installed is established.
Validate the exact destination manifest and integrity before treating a retry as success.
Reject a different artifact at the same sequence.
Never roll the active sequence backwards because an old activation request was retried.
If the requested generation is already superseded, return a typed status that distinguishes historically published from currently active.
Queue visibility acknowledgement must use the contract for its captured target and cannot pretend the old generation is currently serving.

Inject crashes and response loss after every durable boundary.
On restart reconcile durable pointer, installed artifact, in-memory reader, WAL publication sequence, and job state.
Keep the administrative readiness/status route responsive while expensive open/hash work runs in the existing background-work executor.
Separate build/upload/activation operation locks from read-only status access.

Preserve old open sessions and newly arriving mutations across activation.
One indexing batch publishes one captured target. Later writes remain for the next batch.
The existing publish_and_wait loop may resolve a pending job and then prepare further jobs to satisfy readiness; prepare captures the current WAL sequence.
P09 must make batch target capture explicit. Finish or classify the existing pending job first, capture the authoritative target S once, and bind the new batch receipt to S.
Build only from the immutable snapshot at S. Mutations after S stay durable for the next batch even if they arrive before upload or activation finishes.
If a prior pending job is intentionally processed first, record it as a separate existing job, not a duplicate build of the new batch.
The worker acknowledges visibility only after a verified activated receipt.
Do not infer complete creator indexing from public video-count equality; that is a separate migration requirement outside this plan's immediate scope.

### Define rollout order

P09 must implement an explicit capture/stage/adopt path before this sequence is executable.
This is a proposed capability. Current publish does not adopt an arbitrary candidate built by another reader.
 Capture the authoritative production snapshot and job identity once, including S, snapshot bytes/hash, compatibility, corpus identity, and source/image attestation. Bind the final manifest only after the candidate build completes and its output is verified.
Use that captured input to build the immutable promotion candidate. Stage read-only copies of that candidate on the canary through the new identity-checked path.
Run synthetic canary update/delete/crash tests on separate candidate copies or separate synthetic generations. Their reader-local WAL and job identities are not the production job.
After synthetic lifecycle tests, reopen the untouched promotion candidate and repeat its required qualification gates.
At production adoption, validate its captured snapshot/job identity and every manifest/compatibility field against the authoritative prepared job.
Do not rewrite job identifiers or substitute snapshot hashes to make an incompatible canary artifact acceptable.
If the exact artifact cannot be reused, build a newly identified candidate, rerun affected qualification, and obtain any renewed artifact-specific approval before switching.

1. Build and verify source-bound images and a schema-9 candidate in isolation.
2. Deploy a backward-compatible reader/API path that can keep serving the old active generation while understanding the new transfer and session contracts.
3. Deploy or stage the schema-aware builder/client after compatibility readiness is proven.
4. Upload the existing verified candidate to the isolated canary and pass all P10 gates.
5. Present the exact production switch and rollback recipe for its required approval.
6. Install/activate the same verified candidate in production. Do not rebuild from a changed snapshot under the same approval identity.
7. Enable compatible frontend behavior only when the server contract is available, using a capability response if overlap is needed.
8. Observe the actual daily/queue path through one publication.
9. Keep the retained prior artifacts until the independent retention/destruction gate permits removal.

If a release fails, select the verified compatible rollback artifact and preserve WAL plus failed candidate evidence.
Do not rebuild a rollback artifact while production is failing.
A schema rollback may require its compatible reader/API image; the receipt must name that matrix in advance.

## Appendix F. Preserve every website contract

P08/P10 must map every row to an actual test ID or live receipt. No row is satisfied only by source inspection.

| ID | Scenario and exact pass condition |
| --- | --- |
| U01 | Home renders all indexed avatar channels in the oval swarm; hovering shows names and clicking opens scope. |
| U02 | Magnetic cursor behavior runs only for fine pointers and is disabled on touch and reduced-motion preference. |
| U03 | Search input has the global or scoped placeholder, sort select, Search button, Searching loading label, and correct disabled state. |
| U04 | Availability shows 24/7 for always_on and exact scheduled availability/return text with a one-second countdown. |
| U05 | Global, scoped, and unknown-handle request-mode help text remains correct and scope reset searches globally. |
| U06 | Typing @ opens at most eight creators with avatar/name/handle/indexed count. ArrowUp/Down/Enter/Escape and combobox/listbox/option/active-descendant semantics work. |
| U07 | Unknown handle opens request-index confirmation. Approved queue flow and pending/queued/running/failed/rejected/idle cards poll every ten seconds. |
| U08 | Recent searches remain private to this device, at most six, labeled by scope/sort, and clearable. |
| U09 | Relevant/Newest/Oldest always exist; engagement-related sorts appear only with the existing flag. |
| U10 | Footer keeps configured Ko-fi URL and fallback. |
| U11 | URL preserves q, optional youtuber, optional nondefault sort, optional page. Reload executes the same search; Back returns home and clears the search surface. |
| U12 | Result header shows query, channel/global badge, exact pluralized count label, request timing, and search timing. |
| U13 | Shown-of labels collapse to exact totals only when nothing is hidden. Approximately appears only for explicitly approximate totals. |
| U14 | Videos and Shorts have separate current-page sections. Unknown kind stays in Videos with no dropped card. |
| U15 | Posters load lazily; selecting a moment opens inline YouTube at its timestamp with autoplay=1 and no duplicate query parameter. |
| U16 | Cards retain timestamp, highlighted title, creator/name/handle/date, compact views/likes/comments, copy link, and external YouTube action. |
| U17 | Initially show up to three moments. Show all appears only when match_count exceeds loaded hits, uses the pinned generation, shows its loading label, and reports inline failures. |
| U18 | Excerpts render muted previous/next text plus canonical highlighted match; title-only results use title highlights. |
| U19 | Useful/not-useful controls preserve aria-pressed and tracking for feedback, selection, and expansion. |
| U20 | Matched spans render as mark elements without substring errors, invalid Unicode slicing, or unsafe HTML. |
| U21 | Previous/windowed page numbers/current-minus-three through current-plus-one/Next remain available and disable correctly at ends. Cursor continuation never appends duplicate videos. |
| U22 | Copied links use #video-ID; initial load and hashchange scroll and focus the card, respecting reduced motion. |
| U23 | Loading renders three skeleton cards. Error uses alert. Empty names the query. Welcome invites an indexed creator search. |
| U24 | Currently-indexing toast shows creator/avatar at the bottom for five seconds. |
| U25 | Accessibility retains sr-only h1, status/alert/listbox/combobox roles, keyboard access, labels, and visible focus. |
| U26 | Results/picker/dialog/recent/index-state remain lazy chunks; initial static import graph is below 30,000 gzip bytes. |
| U27 | Search runtime prewarms after 900 ms debounce without speculative network traffic. |
| U28 | In-flight identical requests coalesce, unmount aborts the caller correctly, and each caller gets its own request duration. |
| U29 | Analytics preserve search submitted/loaded, channel opened, result opened, timestamp selected, moments expanded, feedback, and link copied with existing metadata fields. |
| U30 | Existing SearchResponse, SearchHit, and SearchVideoResult shapes decode compatibly during mixed-version rollout. |
| U31 | Whole-result overview is explicitly separate from showing X of N loaded; neither changes just because another page loads. |
| U32 | Every facet click submits its exact predicate and refreshes whole-set totals, summary, and the page stream together. |
| U33 | Generation advance shows results-may-have-changed and an explicit refresh action; no page or summary silently switches generation. |
| U34 | Exact totals and Tier 0 arrive with page one in reserved layout. First-result and overview timings are measured separately. |
| U35 | Pending Tier-2/3 work shows a labeled placeholder or explicit failure, never a guessed number. |
| U36 | Default ordering follows the frozen exact rank then engagement policy. Optional presentation buckets never remove results. |
| U37 | React, one fire, coding is dead, CSS which I think is dead now, curspr in Cursor scope, convex/con, and createEffect regressions have expected ordering/highlights plus latency cases. |
| U38 | blackhole runs globally and scoped to Veritasium and Vsauce; transcript and title matches remain exact and complete. |

Use fixture screenshot assertions for text and structure, real browser interaction for event behavior, and network/process receipts for request/generation/performance facts.
A screenshot cannot establish that a server count covers unloaded results.

## Appendix G. Evidence, fixtures, and quantitative gates

### Create the fixed 300-query matrix

| Category | Cases | Mandatory coverage |
| --- | --- | --- |
| Common/stop-word queries | 40 | the, in the, this is the, all scopes and common-word combinations |
| Phrase/proximity | 40 | quotes, slop, repeated tokens, early/late occurrences, cross-field negative controls |
| Typos | 30 | curspr, numerical rules, per-token combinations, exact alternatives beating fuzzy alternatives |
| Prefix and compact identifiers | 25 | createEffect/create effect, convex/con, camel-case and token-boundary cases |
| Negation | 25 | terms/phrases, title/text provenance, explicit positive universe, malformed operators |
| Explicit filters | 35 | creators, metadata buckets, combinations, invalid/unknown parameters |
| Rank and engagement | 35 | every sort, exact ties, partial videos, field effects, stable external IDs |
| Unicode and punctuation | 30 | normalization forms, accents, apostrophes, hyphens, multibyte offsets |
| Long and threshold edges | 20 | over four/ten terms, 128/129 and 50,000/50,001 match fixtures |
| Empty/no-match/validation | 20 | empty, wildcard, no result, invalid ranges, unsupported sorts and filters |
| Total | 300 | Every ID unique and immutable in the release receipt |

One case is a full request definition, including scope, sort, grammar/rank version, and expected outcome.
Run valid cases at page sizes 1, 3, 10, and 20 through exhaustion.
Additional randomized tests and sort cross-products are extra cases and must not replace the named 300.
The exhaustive reference must operate independently of the optimized iterator, bitmap, bound, and summary implementations.
Store human relevance judgments separately from mathematical exactness expectations.
For deliberate v2 improvements, keep the legacy output and reason beside the v2 expected result.

### Record measurements honestly

The primary performance budget is the complete exact first response, including totals, Tier 0, page one, and required highlights.
Record backend processing, API processing, edge/public request, browser first-result, and first-overview time separately.
Use a controlled same-region browser lane, then add geographic lanes if the requested SLO includes them.
Do not hide proxy or serialization time by reporting only engine time.

For the canonical regression set, require every measured successful request below 200 ms.
Report maximum, p50, p95, p99, sample count, timeout/error count, cold/warm state, concurrency, and request distribution.
Use at least 30 samples per canonical query per warm concurrency lane and three independent fresh-process cold runs.
The 300-case exactness sweep is not itself enough performance sampling.
No failed request disappears from a percentile denominator.

P00 must encode this gate matrix as machine-readable configuration. An owner cannot substitute one row's timer for another.

| Lane | Timer start and end | Cache and concurrency | Hard gate |
| --- | --- | --- | --- |
| Internal first response | Engine HTTP request received to complete serialized exact response | Warm, concurrency 1 and 4 or observed production concurrency if greater | Each canonical request below 200 ms |
| Public first response | Test client begins same-origin request to complete body received | Controlled same-region network, warm, same concurrency levels | Each canonical request below 200 ms |
| Browser first result | Search submission event to first real result card painted | Controlled browser/network, warm server, no speculative query request | Each canonical request below 200 ms |
| Browser Tier-0 overview | Same submission event to exact totals and complete Tier-0 overview painted | Same browser run as first result | Each canonical request below 200 ms |
| Cold first query | Request after reader readiness to complete exact response | Fresh process with verified cold payload cache, concurrency 1, no hidden warmup | Internal and public canonical request each below 200 ms |
| Publication overlap | Same internal/public timers during build, transfer, activation and warmed-new window | Concurrency 4 or observed production concurrency if greater | Every canonical successful request below 200 ms and no unexpected error |
| Cursor continuation | Next-page request begins to complete exact page received | Same pinned session, representative early/middle/final pages | Each measured canonical continuation below 200 ms, zero repeated rank work |
| Reader open | Process start to readiness, plus physical bytes read | Three independent cold runs | Record time; physical reads below 200,000,000 bytes. Open time is not silently counted as a warm-query timer |
| Tier-1/Tier-2 work | Request begins to exact summary completion | Explicitly requested full set | Record latency and resource budgets from P01; no invented 200 ms claim for arbitrary expensive aggregation |

Record the exact canonical query list and scope/sort combinations in the gate matrix.
Include all historical regression queries named in U37/U38 and common phrases the, in the, and this is the.
Negative tests declare expected status/code/body before execution. Count them in attempted/negative-test totals and validate those outcomes exactly.
They do not enter the successful-search latency distribution. Unexpected failures still count as release failures and cannot be relabeled after measurement.
The wider 300-case valid first-page sweep must report all latency outliers as findings even when a case is outside the repeated canonical performance lane.
The external reviewer must approve the canonical workload definition; do not shrink it after seeing slow cases.

Reader allocation is 2 GiB, or 2,147,483,648 bytes.
RSS must be below 1.5 GiB, or 1,610,612,736 bytes.
The 80% reader cgroup gate is 1,717,986,918 bytes.
These are different metrics; report both plus file-cache/anonymous breakdown.
Require zero swap and OOM events, zero unexpected search/admin/scrape errors, and disk below the configured 80% bound.
Deliberate corruption, validation, conflict, and expired-cursor cases must return their predefined typed errors and remain in the evidence denominator.
The temporary 4 GiB builder has its own profile-derived 80% cap of 3,435,973,836 bytes.
Do not apply that builder cap to every profile or confuse it with the reader budget.
Cold open must physically read fewer than 200,000,000 bytes, measured from the reader process and validated in a cold-filesystem canary lane.

The publication lane must include baseline, build/upload overlap, activation, and at least 60 seconds of warmed new-generation load.
Validate actual active request interval coverage using union of intervals.
Idle gaps must not count as load duration.
Check the publisher exit code before reading optional activation-marker files under set -e.
A 120-second build window that ends before activation is incomplete.

### Save one receipt per gate

Use JSON with these fields. Extra fields may extend it.

```json
{
  "gate": "P10-exact-pages",
  "status": "pass|fail|incomplete|blocked",
  "source_commit": "full SHA",
  "parent_commit": "full SHA",
  "corpus_digest": "BLAKE3 digest",
  "generation": "exact generation",
  "storage_schema": 9,
  "grammar_version": 2,
  "rank_version": 2,
  "profile": {"reader_memory_bytes": 2147483648, "cpu_kind": "measured", "cpus": 2},
  "command": "exact command without secrets",
  "started_at": "UTC timestamp",
  "finished_at": "UTC timestamp",
  "process_handle": "observed handle or completed exit identity",
  "attempted": 300,
  "passed": 0,
  "failed": 0,
  "skipped": 0,
  "samples_path": "retained path",
  "samples_digest": "digest",
  "metrics": {},
  "failure_cases": [],
  "reviewer": "independent reviewer identity"
}
```

The placeholder example is not a successful receipt. It must be populated from the actual run.
A pass requires zero failed and zero unexplained skipped mandatory cases, all artifacts present, and matching identities.
Tool completion, healthy status, and a source SHA are not substitutes for the required observation.

## Appendix H. Give an implementing model one bounded task

Copy this instruction and fill the identifiers before assigning a phase.

```text
Implement only PR Pxx from schema9.md at parent SHA <verified parent>.
Read its Depends on, Files, Build, You see, unit/live/perf blocks and Appendices E/F/G.
Inspect actual source and artifacts first. Do not infer completed work from prior summaries.
Before edits report your file ownership and the invariant you will change.
Do not edit shared schema/protocol files unless you own them.
Use existing abstractions and commands. Proposed commands must be created and tested first.
Keep exact membership, provenance, scores, totals, summary units, and pinned generation.
If a gate fails, follow the matching Appendix C row and preserve the failure receipt.
Do not lower thresholds, omit candidates, approximate numbers, or switch production to make a test pass.
Return the source SHA, exact diff, test and live receipts, performance samples, independent review,
remaining failures, and the next dependent phase. Never mark deployment complete from a local test.
```

For the external reviewer, request a verdict for each normative contract and risk.
Ask them specifically to challenge rank bounds, score arithmetic, provenance/moments, bounded cursor state,
cross-process generation retention, installed-generation activation retry, and real 200-ms feasibility.
Unresolved findings must become explicit gate failures or documented contract amendments before implementation.
