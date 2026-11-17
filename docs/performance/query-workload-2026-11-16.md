# Query workload follow-up: 2026-11-16

The captured workload's slow queries include “thank you”, “clearly the world is
ending”, and common phrases such as “this is the”. The reader profile puts the
broad matched-document scan at 17.66% self CPU after document accessor inlining.
This follow-up removes release bookkeeping from that scan's moment counters.

`MomentCounter::last_start_ms` exists only to check timestamp order in a debug
assertion. Its field and update now compile only with debug assertions. Cluster
counting, the twenty-second coalescing window, title-only counts and debug
ordering checks remain unchanged. A source-extracted layout probe using the same
release compiler reports both `MomentCounter` and `Option<MomentCounter>` shrink
from **24 to 16 bytes** on this x86-64 target. At 32,822 videos this saves 262,576
bytes per partition's dense counter vector, plus a timestamp update per match.
This is transient counter memory, not a 33% reduction in total reader memory.

## Measurements

The baseline is the preceding [document accessor inline change](document-inlining-2026-11-16.md),
including its uncommitted source change. Same schema-9 index, compiler, P-core
reader pinning and low-power-core driver as that record. All comparisons use the
committed 330 requests with two measured repeats that alternate reader order,
plus one warm-up pass. Timing fields are excluded from payload comparisons.
Counters include warm-up. Requests represent the captured benchmark cases,
not production frequency weights. No API cache shortcut is measured.

Ratios are candidate/baseline. The last row swaps which binary occupies each
reader slot; its printed B/A ratio is inverted here.

| Run | Baseline cycles | Candidate cycles | Cycle ratio | Instruction ratio | Payload mismatches |
| --- | ---: | ---: | ---: | ---: | ---: |
| A/A noise control | 62,424,729,990 | 62,132,640,078 | 0.99532 | 1.00112 | 0 |
| Moment trial 1 | 62,759,284,383 | 61,997,344,923 | 0.98786 | 0.99930 | 0 |
| Moment trial 2 | 62,201,959,382 | 61,392,610,919 | 0.98699 | 0.99840 | 0 |
| Swapped slots | 62,089,523,877 | 61,859,256,059 | 0.99629 | 0.99963 | 0 |

Raw cycle reductions range from 0.37% to 1.30%. The A/A run itself favored
slot B by 0.47%; adjusting for that observed bias suggests roughly 0.8% less
cycle work, but this is an inference from one control, not a portable latency
claim. Retired instructions are effectively unchanged relative to control drift.
The reliable structural result is smaller release counters; local CPU evidence
supports a modest gain, with explicit slot-order sensitivity.

## Rejected experiments

- Reusing title tokens across adjacent candidate documents: instructions rose
  0.39%; cycle ratio 0.99571 did not clear the 0.99532 A/A control. Reverted.
- Lowering the unconditional document-validation cutoff from 128 targets to 16:
  instruction ratio 1.00086, cycle ratio 0.99684. Inconclusive; reverted.
- Selectively parsing only document text was considered but not implemented.
  Preserving arbitrary JSON validation, duplicate-key behavior and scalar roots
  would need a custom validating visitor. That complexity was not justified.

## Reproduction and artifacts

Use `scripts/perf/reader-build.sh`, `reader-ab.sh` and `reader-profile.sh` as in the
[reader playbook](../playbooks/reader-performance.md). Compare the preceding
inline-only source with inline plus the debug-only moment field; comparing
against `377dd27` instead combines both optimizations.

Local query timings, design alternatives, binaries, layout source/probes,
A/A and A/B logs, parsed `measurements.json`, and post-change profile are under
`/home/pronsh/.cache/ytsearch-query-perf-20261116/`. The baseline binary is
`/home/pronsh/.cache/ytsearch-perf-20261116/inline`.

## Verification

`bun run check` passed, including `bun run check:precommit`, workspace Rust tests,
all 23 browser scenarios, dependency/license/public-surface checks and operational
fixtures. Both targeted moment-coalescing tests passed before measurement.
The release reader matched all 330 payloads in each of three comparisons, and
the post-change profile was captured successfully. No deployment was performed.
