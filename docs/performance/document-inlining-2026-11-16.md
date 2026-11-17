# Document accessor inlining: 2026-11-16

Adding `#[inline]` to `DocumentTable::document` reduced retired instructions by
1.07–1.13% and cycles by 1.63–1.83% in two local full-reader trials. All 330
response payloads matched in each trial. These are local workload measurements,
not a production latency guarantee.

## Evidence

The previous [decoding measurements](document-decoding-2026-10-17.md) guided a
fresh profile. Broad scans used 15.70% self CPU, text cursor advancement 8.48%,
and document decoding 4.34%. The broad scan already has measured memory limits;
this change instead removes document accessor calls and lets callers eliminate
unused fields. Actual release disassembly contained 227 static calls to the
accessor before and zero after. This is a static codegen count, not a runtime
call count. The concrete record, corruption checks and error strings are unchanged.

Ratios below are after/before; lower is better.

| Replay | Instructions before | Instructions after | Ratio | Cycle ratio | Response mismatches |
| --- | ---: | ---: | ---: | ---: | ---: |
| A/A control | 177,149,584,587 | 177,531,149,275 | 1.00215 | 1.00152 | 0 |
| Inline trial 1 | 178,754,259,701 | 176,849,137,087 | 0.98934 | 0.98170 | 0 |
| Inline trial 2 | 178,902,279,976 | 176,874,074,905 | 0.98866 | 0.98371 | 0 |

A rejected candidate outlined multibyte varint decoding into a cold function.
Although it shrank the text cursor from 2,559 to 1,736 bytes, reader instructions
increased 2.38% and cycles increased 0.69%. That change was reverted.

## Method and reproduction

Baseline commit: `377dd27`. Rustc 1.99.1, release opt-level 3, line tables,
no stripping, identical flags for both binaries. Intel Core Ultra 7 255H;
readers pinned to P-cores, driver pinned to low-power cores. Schema 9 index:
8,142,650 documents, 32,822 videos, 293,152 terms. Replay uses the committed
330 reader requests and bypasses API caching. Two measured repeats alternate
which reader runs first per request; both readers also receive a warm-up pass.
Timing fields are excluded from response comparison. Background laptop load
limits wall-time interpretation; instruction reduction cleared the A/A drift.

Use the [reader performance playbook](../playbooks/reader-performance.md):

```bash
PERF_TARGET_DIR="$artifact_dir/target" scripts/perf/reader-build.sh 377dd27 "$artifact_dir/before"
PERF_TARGET_DIR="$artifact_dir/target" scripts/perf/reader-build.sh . "$artifact_dir/inline"
scripts/perf/reader-ab.sh "$artifact_dir/before" "$artifact_dir/before" 2
scripts/perf/reader-ab.sh "$artifact_dir/before" "$artifact_dir/inline" 2
scripts/perf/reader-profile.sh "$artifact_dir/inline"
```

Local binaries, logs, disassembly, machine/index/request fingerprints and parsed
measurements are in `/home/pronsh/.cache/ytsearch-perf-20261116/`.

## Verification

`bun run check` passed, including `bun run check:precommit`, workspace Rust tests,
all 23 browser scenarios, dependency/license/public-surface checks and operational
fixtures. Existing metadata tests cover literal little-endian records and every
truncated record boundary. Both full-index comparisons returned zero payload
mismatches. The repaired profiling script captured the post-change trace without
changing system permissions. No deployment was performed.
