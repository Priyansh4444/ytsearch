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
