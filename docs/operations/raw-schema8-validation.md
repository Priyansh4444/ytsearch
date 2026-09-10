# Schema 8 validation: pending

Baseline release source: `94d01790b558fce23d61554c31c710a5c0a009f9`.
These observations do not establish production readiness.

## Baseline on the previous generation

The isolated canary ran 126 requests against the previous index generation
using the new binary. Evidence is in
`.search-experiment/results/raw-canary-94d01790b558-1788821908/publication/`.

| Measurement | Result |
| --- | ---: |
| Median | 99.614 ms |
| p95 | 431.470 ms |
| Maximum | 798.151 ms |
| Response validation failures | 3 |

The slowest successful queries were `and the`, `in the`, and `this is the`.
All three response validation failures were `blackhole` scoped to Veritasium.
They returned HTTP 200; the failing contract predicate has not yet been
isolated. Do not relabel them as transport errors or update the expected
ranking without examining the results.

A subsequent authenticated canary capture isolated the failing predicate:
the first video is `fu3645D4ZlI` ("Spinning Black Holes"), while the contract
requires `ORxKf1FN3ro`. The expected video is present in the first result page.
All returned handles match Veritasium, and the first group's snippets correctly
highlight `black` and `hole`. The ranking change still needs explanation;
the contract has not been relaxed. This later request succeeded during the
build, so the earlier timeout does not establish persistent search blocking.

The baseline gate stopped before publication. Therefore this run does not
measure schema 8. Build the new generation separately, then rerun the unchanged
200 ms maximum gate, including publication load and correctness checks.

## Preparation observations

The detached preparation run reported active generation 2145 and durable
sequence 2151, then downloaded the existing prepared snapshot through sequence
2150. A subsequent publication must reconcile any remaining sequence before
declaring the reader current.

During startup, the canary used 231,833,600 bytes of current cgroup memory,
zero swap, and 17% disk. A sampled process wait channel was
`folio_wait_bit_common`; that identifies a page wait, not its complete cause.
These observations alone do not justify a RAM upgrade.

A Veritasium request through the local Fly tunnel timed out after ten seconds
while snapshot transfer was underway. Reader work, transfer contention, and
tunnel behavior must be distinguished before attributing the delay. The
builder subsequently reported 277,082,112 bytes of current cgroup memory,
zero swap, and 2% disk. These are individual samples, not peak measurements.

## Required evidence

Fly subsequently reported the detached builder stopped at 2026-09-08 04:34 UTC
with `requested_stop=true` and exit code 130. The local preparation process
handle was gone and no final builder report was present. This is not evidence
of successful activation; inspect the durable generation state before retrying.
The queue worker was separately observed retrying failed private admin readiness
every 30 seconds. A running Machine is therefore not proof of queue recovery.

- Completed schema-8 manifest and exact activated sequence.
- Correct Veritasium results and highlights with the failing predicate identified.
- Every measured canonical request below 200 ms, with zero validation errors.
- Publication/restart correctness and resource measurements throughout the run.
- Equivalent production-path measurements before final cutover and retirement.

## Common-word optimization contract

The user permits special treatment of stop words. Preserve quoted phrases,
negative terms, token positions, and meaningful matches. For mixed queries,
prefer selective terms to identify candidates before checking common terms.
For all-common queries, evaluate frequency-based plans or compact phrase
indexes without silently returning empty results or fabricated exact totals.
Any behavior change must be tested with common-word-only queries, mixed rare
and common terms, repeated words, and quoted phrases. Do not encode the measured
query strings as production exceptions.

The current unquoted broad conjunction visits complete posting streams and
scores each intersecting document before grouping. Exact moment counts prevent
a top-K-only shortcut from replacing this traversal. Schema 8 materializes
single-token plans; it does not currently materialize common multi-token plans.
An adjacent-pair index cannot replace unquoted AND: separated occurrences and
terms split across title and transcript still qualify.

`engine/tests/common_words.rs` covers phrase order, adjacency, repeated stop
words, exclusions, mixed queries, cross-field AND, and prefix matches. Its
960 full-response differential comparisons exercise streaming versus
materialized grouping with field selections, position priority, eight sort
settings, five group limits, and moment-boundary timestamps. These small
fixtures prove equivalence for those cases, not production latency.

## Interrupted build and transport recovery

The subsequent `8336ff64b233f0385e35d8b13f9f3f63e21b841d` preparation
also did not produce a final report or completed manifest. Its local log ends
with `ssh shell: session forcibly closed`; Fly reports a requested stop at
2026-09-08 06:35:13 UTC. The log alone does not distinguish a connection loss
from the wrapper's configured 2,400-second timeout. Neither establishes a
successful publication or an out-of-memory failure.

Inspection after that stop found a 1.7 GB bigram spool and a 3.3 GB workspace.
The earlier interrupted run had a 3.1 GB bigram spool and 4.6 GB workspace.
These are incomplete-run snapshots, not comparable completed-build peaks.

The publication wrapper now starts one bounded remote job and polls its
completion record. An atomic job-directory claim prevents a lost launch
acknowledgement from starting the same job twice. Short SSH failures retry the
same job until the original deadline. A claimed job that does not acknowledge
startup within 30 seconds fails without launching a duplicate. The remote command has its own timeout;
the caller still owns the exact Machine lease and stops the builder on exit.
An absent or malformed completion record is never treated as success.
The remote log location and deadline are printed once. This supports recovery
from transient SSH failures within a run, not resuming across controller
termination, Machine restarts, or source changes.

Run `scripts/test-fly-publication-job.sh` to exercise dropped acknowledgements,
repeated status checks, command failure, remote timeout, abandoned launch
claims, malformed status, and persistent transport failure. The last failed
SSH diagnostic is printed with a 20-line limit before deadline cleanup. The existing
publication-wrapper tests still cover lease cleanup and stop failures.
Live transport recovery and a completed production-size build remain unproved.

## Preparation handoff after local session loss

A later preparation started the isolated reader at 2026-09-08 11:00:16 UTC
using source `8336ff64b233f0385e35d8b13f9f3f63e21b841d`. It allowed a
7,200-second publication command and held a 14,400-second reader lease.
The last observed phase was reader startup; the builder was still stopped.
The queue worker was independently confirmed stopped after repeated private
readiness failures. No queue data or volumes were deleted.

After the local session changed, process handle `40236`, its temporary evidence
directory, and the temporary Fly credentials were unavailable. A fresh Fly
read request failed with `no access token available`. The reader's final state,
whether publication subsequently started, and cleanup are therefore unknown.
A lease expiring does not establish that a Machine stopped. After restoring
authentication, inspect the exact reader and builder states, lifecycle events,
remote publication job records, and active generation before retrying. Do not
infer success or termination from the missing local process handle.

Authentication was subsequently restored. Authoritative inspection found the
canary still running generation 2145. The builder ran from 11:03:44 until a
requested stop at 12:15:14 UTC on 2026-09-08, with no OOM flag. Brief inspection
of its retained volume found a 9.0 GB workspace, including
`broad-single.payloads.spool` at 7,729,086,584 bytes. The unigram and bigram
spools were gone and their output files existed. A manifest existed, but the
final `broad-single.bin` did not: manifest existence alone does not prove
completion of schema-8 materialization. Temporary remote job logs did not
survive the observed restart, so the exact interruption cause remains unknown.

The next build investigation must address materialized-plan payload growth,
not repeat the already-completed unigram/bigram optimization. Measure plan
counts and duplicated retained documents, then bound build/artifact cost while
preserving the generic query fallback and exact response semantics.

The bounded implementation writes broad-single format version 2 and retains
five hits per video instead of 250. This covers the frontend's three-hit
request and the API's default five. The reader uses the full posting reducer
for requests above five; version-1 files keep their original 250-hit coverage.
Exact group and moment totals remain encoded independently of retained hits.
Full-response comparisons at requested depths 1, 3, 5, 6, and 250 pass on an
eight-moment-per-video fixture. Older readers reject format version 2 rather
than treating partial retained hits as a complete 250-hit answer.
Rollback to an older reader must therefore also activate a compatible retained
generation; swapping only the binary after publishing version 2 is unsafe.

This change bounds retained document payload per plan and video, not the total
number of plans or posting visits during building. It does not establish a
50-fold total artifact reduction, faster all-common multi-token queries, or
sub-200-ms latency for large group limits. Those remain measured release gates.

## Release sequence and reported failures

Production release remains required, after validation, not replaced by isolated
staging. Run the canonical workload in `scripts/benchmark-search-cases.tsv`
against the completed latest-schema canary, including publication load and
restart checks. Require every measured request below 200 ms and zero response
validation failures before production activation. Repeat the workload through
the production path after activation, retaining the compatible rollback
generation until that verification passes.

The workload now also includes the reported `curspr` query scoped to
`@cursor_ai`, standalone `the`, and spaced `create effect`. Existing rows cover
the previously reported React, fire, coding, CSS, blackhole, compact-token,
convex, and common-word failures. These new rows are correctness-and-latency
checks, not recorded passing production measurements. Queue admission,
pagination, highlights, and sorting also require their focused contract tests;
page-one timing does not establish those behaviors.

The `f4663a4` isolated baseline on the existing generation measured 135
requests with three ranking failures, a 104.245 ms median, 537.176 ms p95, and
891.380 ms maximum. After the posting decoder changes in `a2e0e2c`, the same
baseline measured three ranking failures, a 90.532 ms median, 491.278 ms p95,
and 801.400 ms maximum. `and the`, standalone `the`, `in the`, and
`this is the` remained over 200 ms. The reader's I/O summary recorded about
1.1 seconds of thread CPU across about 26 seconds of query wall time in the
first run. The decoder reduced CPU work, but the existing generation remains
I/O-bound on the 2 GB reader.

`RAW_CANARY_MIGRATION_MODE=record-old-generation` exists only to cross this
format boundary. It records the old baseline and build-load phases, then applies
the zero-error, sub-200 ms gate to the post-swap and warmed-new phases. A normal
strict canary publication from the new generation is still required afterward.
This prevents an old format from blocking construction of its replacement
without weakening the final release gate.

`cargo nextest` was checked but is not installed in the current environment.
The available Cargo library suite passed 162 tests after the workload update.
Neither runner substitutes for the deployed HTTP latency gate.

## Correcting the cgroup-v1 swap probe

The initial builder sample reported 86,016 swap bytes by subtracting
`memory.usage_in_bytes` from a later `memory.memsw.usage_in_bytes` read.
That difference is unreliable: allocations can occur between reads, and the
kernel documents these usage counters as approximate. The probe now reads
the hierarchical `total_swap` counter from `memory.stat` directly. Missing or
malformed accounting still fails closed. The zero-swap threshold is unchanged.
See the [kernel memory-controller documentation](https://docs.kernel.org/admin-guide/cgroup-v1/memory.html#stat-file).

A subsequent direct sample found zero swap, 669,347,840 current memory bytes,
11% disk utilization, and zero OOM counters. `/proc/swaps` independently showed
zero used swap at an earlier check in the same run. These are current samples,
not proof that swap was zero throughout the build. Preserve the initial sample
as evidence of the old probe; do not retroactively relabel it as a valid zero.

The regression test executes the actual remote probe against local fixture
files, checking mismatched usage counters with zero swap, nonzero hierarchical
swap, and missing or invalid counters. This replaces reliance on mocked final
JSON alone for the cgroup-v1 calculation.

## Bounded build outcome

The `9398a34679dc` preparation reached its 7,200-second controller deadline
after SSH certificate requests failed with a connection reset. There is no
final controller report. Subsequent Fly inspection confirmed both isolated
Machines stopped; production remained started.

The retained builder generation does contain a schema-8 manifest with hashes
for all files, including `broad-single.bin` at 8,208,104,746 bytes. It reports
8,322,726 documents, 34,611 videos, and 66 channel records. The workspace is
9.5 GB. This demonstrates completion of the engine's manifest-writing phase,
not successful publication, activation, or an independent checksum audit.
The channel count includes preparation state and is not proof that the 61
requested creator profiles are complete.

Do not blindly restart the staged `9398a34679dc` CLI: its startup cleanup removes
saved job directories, and `publish_generation` always creates and builds a
fresh generation. Validate reusable completed artifacts against the exact job,
snapshot, compatibility, and hashes before adding safe resumability. The large
materialized artifact still needs size/build-cost investigation; the five-hit
retention change alone did not produce a compact index.

## Publication retry protocol

The updated CLI preserves existing job directories. Before downloading a new
snapshot, it durably records the prepared job's input identity. A retry can
remove that job's incomplete download and generation only when the input
record matches and all top-level artifacts have recognized names and types.
Older directories without that record remain untouched and require recovery.

After `GenerationBuilder::build` returns and its sequence matches the job,
the CLI atomically writes `verified-build.json`. The receipt binds the job ID,
sequence, snapshot size and hash, corpus compatibility, and manifest hash.
Resume checks those inputs and opens the engine with full integrity validation
before uploading remaining files. A partial receipt is not completion evidence.

Cleanup requires the reader's exact job status to be `Activated` and match the
receipt. A high active sequence alone does not authorize deletion. Failed
status lookups preserve local work and emit a warning without blocking new
publication. The CLI renames confirmed work to `.reclaim-<job-id>` before
removing it, so cleanup can finish after a crash even if the receipt is gone.

Local checks use `cargo test -p youtube_cli` and
`cargo clippy -p youtube_cli --all-targets -- -D warnings`. These checks do
not establish production publication, creator completeness, or search latency.
The retained production-size artifact has no completion receipt and is not
automatically reusable under this protocol.

## Compressed payload prototype

The retained 8,208,104,746-byte file contains 14,024 plans. A read-only full
scan took 487.669 seconds on the shared2/2 GB builder. Zlib level 1 compressed
its payloads to 3,061,243,735 bytes. SHA-256 payload identity found 12,481 unique
answers totaling 7,813,851,868 uncompressed bytes. Deduplication alone therefore
saves much less space than compression. These numbers describe storage, not
query latency or a validated activated generation.

The Zstd prototype reuses Raw's existing dependency. A release-mode local
benchmark used the largest retained answer, with BLAKE3
`7aa510e945658178ebd14b809071d69c03db341e076b9cb6cf57f98b9b14bf72`.

| Measurement | Raw | Zstd level 1 |
| --- | ---: | ---: |
| Payload bytes | 2,751,900 | 1,453,679 |
| Warm decode and parse median, 100 samples | 2.436 ms | 4.681 ms |
| Warm decode and parse maximum | 5.058 ms | 14.419 ms |

Encoding took 21.118 ms. Setup and byte-equality assertions are outside the
decode timer. The local CPU, warm data, and single largest answer do not prove
performance on the 2 GB Fly reader. Full-corpus Zstd size, build cost, cold
startup, peak allocation, and HTTP latency remain release gates.

Run the ignored diagnostic with an absolute `RAW_BROAD_PAYLOAD_FILE` path:
`cargo test --release -p youtube_raw_search --lib measure_retained_payload -- --ignored --nocapture`.
Normal tests cover both legacy payload versions, compressed roundtrips,
malformed frame lengths, forged allocation counts, and full-response parity.

After adding rejection of trailing Zstd frames, a repeat on the same payload
measured 2.419 ms raw and 4.674 ms compressed median decode and parse time.
Maximums were 5.427 ms and 8.774 ms; encoding took 21.351 ms. The encoded
size was unchanged. The final release-mode library suite passed 166 tests
with the one diagnostic test ignored. This does not validate production
latency or daily indexing.
