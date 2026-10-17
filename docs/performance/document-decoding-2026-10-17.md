# Document decoding: 2026-10-17

`DocumentTable::document` now checks one complete 24-byte record before decoding
its six little-endian fields. In five local full-reader trials, the median retired
instruction ratio was 0.9769: **2.31% fewer instructions**.
Every trial matched all 330 response payloads. This is a local workload result,
not a production latency claim.

## Scope and floor

The fresh reader profile assigned 6.15% self CPU to document decoding. The ideal
success path needs an ID check, one complete record check, address arithmetic,
and six u32 fields; contiguous little-endian data permits a 16+8-byte copy on x86.
No helper call or allocation is necessary for a successful decode.

The actual baseline had four inlined per-field range guards, two retained
`read_u32` calls with their own checks, Result tests, and register spills. The
actual changed binary has two guards total, a 16+8-byte copy, and zero success
helper calls. `nm -S` reports **469 -> 250 bytes** (46.7% smaller) for this function,
not the whole executable. Saved registers fall from six to one. The 32-byte frame
and an ID stack store remain because error formatting shares the function;
Result tagging and repeated address arithmetic also remain. The payload reaches
the ideal copy shape; this does not establish a global performance ceiling.
A matching post-fix profile assigns 4.74% self CPU to this function versus 6.15%
before; sample shares are contextual rather than a standalone speedup metric.
After annotation puts 88.88% of this function's samples on its first 16-byte
metadata load, consistent with remaining metadata-access latency rather than
helper overhead. That is an inference from sampling, not a cache-miss count.

The change adds six production lines and explicit byte offsets, matching the
neighboring `search_document` parser. There is no unsafe code, additional
abstraction, dependency, visibility change, schema change or ranking change.

## Machine and method

- Intel Core Ultra 7 255H: 16 physical/logical CPUs, six P-cores, eight E-cores,
  two LP E-cores; 480 KiB aggregate L1d, 768 KiB L1i, 28 MiB L2 and
  24 MiB L3, one NUMA node; powersave governor, turbo enabled.
- Rustc 1.101.0-nightly (c36f14571), LLVM 23.1.1,
  `x86_64-unknown-linux-gnu`. Release opt-level 3, line tables, no stripping;
  no added target-cpu, LTO or codegen-unit override. Same flags for both binaries.
- Reader pinned to P-cores 0-5; load driver pinned to LP cores 14-15. Background
  load was present, so wall time is contextual. Instructions are the primary
  acceptance metric; min/median and per-trial ratios are reported below.
- Schema 9 index: 8,142,650 documents, 32,822 videos, 293,152 terms.
  Replay the committed `scripts/perf/reader-requests.txt`, bypassing API caching.
- A/A control used the original binary on both sides with five request repeats:
  instructions 377,986,856,319 / 377,800,343,352 (0.05% difference), cycles ratio
  0.995, wall ratio 1.011; zero response mismatches.
- Five fresh A/B trials used one request repeat each, A then B for each request.
  The existing driver also warms each reader once; counters include that pass
  for both readers. Each trial therefore counts 660 requests per reader.
  Timing fields are excluded from payload comparison. This fixed order is a
  limitation for cycle/wall interpretation; independent confirmation uses two
  repeats per request to swap the first reader on the second repeat.

## Measurements

Ratios are after/before; lower is less work or time.

| Trial | Instructions before | Instructions after | Instruction ratio | Cycle ratio | Wall ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 126,488,795,792 | 123,686,153,164 | 0.9778 | 0.9829 | 0.9807 |
| 2 | 126,428,511,179 | 123,669,167,430 | 0.9782 | 0.9824 | 0.9866 |
| 3 | 126,333,382,051 | 123,297,483,352 | 0.9760 | 0.9764 | 0.9856 |
| 4 | 126,453,262,933 | 123,529,632,831 | 0.9769 | 0.9798 | 0.9805 |
| 5 | 126,124,959,434 | 123,172,202,813 | 0.9766 | 0.9793 | 0.9902 |

| Metric | Before min / median | After min / median | Ratio min / median |
| --- | ---: | ---: | ---: |
| Instructions | 126,124,959,434 / 126,428,511,179 | 123,172,202,813 / 123,529,632,831 | 0.9760 / 0.9769 |
| Cycles | 43,235,362,065 / 43,688,786,245 | 42,391,066,019 / 42,798,309,885 | 0.9764 / 0.9798 |
| Summed request wall ms | 9,376 / 9,600 | 9,250 / 9,416 | 0.9805 / 0.9856 |

The median cycle ratio corresponds to 2.02% fewer cycles in these trials;
wall variation and background load prevent a portable latency guarantee.

An independent verifier repeated the replay with two measured repeats per query,
swapping the first reader on the second repeat: **330 payloads matched**, and
instructions fell from 189,617,385,802 to 185,607,652,673 (ratio 0.97885,
**2.11% fewer**). This run counts 990 requests per reader including warm-up.
Its cycle ratio was 0.98349. Two-repeat displayed wall times use the driver's
upper-middle sample, so they are not pooled with the one-repeat table above.

## Behavior and rejected work

The handwritten RAWMET02 fixture predates the parser edit. It checks all six
fields, high bits, u32::MAX, the last record, exact out-of-bounds errors, 64
truncated document/block lengths and trailing bytes. Its two tests passed on the
old and new parser. An isolated old/new sibling also matched 5,267 returned
values/errors; actual production disassembly, not the sibling layout, proves
the codegen result. Full-reader parity preserves ordering and wire data too.

`open` checks the complete table length with checked arithmetic. Every accepted
ID consequently has a complete record. The new range check retains defense in
depth and the existing corruption message; public malformed-file errors are
unchanged. No trusted unchecked access is introduced.

A fuzzy-trie parser sibling showed the same assembly gap but was deferred: the
whole fuzzy walk consumed only 0.86% self CPU in this trace. No fuzzy change was
integrated and no fuzzy timing claim is made. Broad-scan metadata layout, ranking
changes and native-only codegen were also left outside this unit.

## Verification

An independent verifier reran the behavior lock and balanced replay, reviewed the
actual binaries and invariant proof, then ran `bun run check` (including
`bun run check:precommit`). It passed: 352 Rust tests with four existing ignored,
137 frontend tests, 40 Worker tests, all 18 browser scenarios, dependency/license
checks, the public-surface audit, operational fixtures and shell syntax checks.

## Reproduce

Use the same full index and machine controls described in the
[reader playbook](../playbooks/reader-performance.md). `before` is
`9a4077e`; `.` is this change. Keep binaries and build cache in a durable directory.

```bash
PERF_TARGET_DIR="$artifact_dir/target" scripts/perf/reader-build.sh 9a4077e "$artifact_dir/before"
PERF_TARGET_DIR="$artifact_dir/target" scripts/perf/reader-build.sh . "$artifact_dir/after"
scripts/perf/reader-ab.sh "$artifact_dir/before" "$artifact_dir/before" 5
# Run five times, sequentially; retain each output and current /proc/loadavg.
scripts/perf/reader-ab.sh "$artifact_dir/before" "$artifact_dir/after" 1
nm -S --demangle "$artifact_dir/after"
objdump -d -C -Mintel --no-show-raw-insn "$artifact_dir/after"
```

Local raw logs, machine capture, index/request hashes, source hashes, real asm,
profiles, JSON measurements and independent verification are in
`~/.cache/ytsearch-perf-20261017/`. These machine-specific files are not checked in.
