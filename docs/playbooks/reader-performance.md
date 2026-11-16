# Reader performance

## Fast path

```bash
# 1. Build the two versions to compare (any git ref, or . for the working tree).
scripts/perf/reader-build.sh master /tmp/reader-base
scripts/perf/reader-build.sh . /tmp/reader-head

# 2. Noise floor: the same binary on both sides. Ratios far from 1.000 mean
#    the machine is too noisy to judge small changes.
scripts/perf/reader-ab.sh /tmp/reader-head /tmp/reader-head 3

# 3. The comparison. `parity: mismatches=0` is required for a pure performance
#    change; any mismatch prints the request path and fails the script.
scripts/perf/reader-ab.sh /tmp/reader-base /tmp/reader-head 5

# 4. Where the time goes, and what one hot function waits on.
scripts/perf/reader-profile.sh /tmp/reader-head 'TextAlternativeCursor>::advance_to'

# Call stacks (who calls the hot function):
scripts/perf/reader-build.sh --frame-pointers . /tmp/reader-fp
scripts/perf/reader-profile.sh /tmp/reader-fp
perf report -i <printed dir>/perf.data --no-children -g caller --symbol-filter=search_document
```

All scripts serve the full-size index from the main checkout
(`.search-experiment/raw-index-schema9-accel-test`; override with `RAW_INDEX`)
and replay `scripts/perf/reader-requests.txt`, the 330 reader requests the
benchmark cases produce through the real API. After changing
`scripts/benchmark-search-cases.tsv` or how `youtube_search_client` builds
reader URLs, regenerate that file with `scripts/perf/reader-capture-requests.sh`.

Judge changes by `cycles` and `instructions` from `reader-ab.sh`, not wall
time: with both readers pinned to the P-cores, an A/A run agrees within about
1% on counters but varies about 4% on wall time.

## Reading a profile

- One instruction holding most of a function's samples, right after a load
  (`mov (%reg,...)`, `cmp %reg,(...)`), is a cache miss. The fix is fewer or
  earlier loads (prefetch, a smaller structure, arithmetic instead of a search),
  not fewer instructions.
- Stores to `self` fields inside a decode loop (`mov %r15,0x40(%rsi)`) mean
  LLVM could not keep the state in registers, usually because an error path in
  the loop allocates. Copy the fields into locals and write them back once at
  the exit (see `TextAlternativeCursor::advance_to`).
- `call` to a small accessor in a hot loop, returning `Result` through the
  stack: mark it `#[inline]`.

## Traps

- **The API caches responses.** The API asks the reader for
  `use_cache=true&cache_ttl=60`, and repeated `/api/search` calls are served from
  that cache in under 5 ms. Benchmark the reader directly with the captured
  requests (header `x-raw-search-key`), as the scripts do.
- **`time_to_first_result_ms` is rounded** to whole milliseconds; time requests
  on the client instead.
- **DWARF call graphs stop at the leaf** in this crate (`perf --call-graph
  dwarf`); build with `--frame-pointers` and use `--call-graph fp` (`-g`).
- **User-only profiling with `perf_event_paranoid=2`** needs both user-space
  events (`-e cycles:u`) and user-only callchains (`--user-callchains`).
  `reader-profile.sh` also limits the ring buffer to `-m 128` to fit the
  unprivileged locked-memory budget. No system permission change is needed.
- **Hybrid-CPU counters are multiplexed** when one `perf stat` asks for both
  `cpu_core/` and `cpu_atom/` events; the ratios then disagree by 3% even for
  identical binaries. Pinning readers to one core type avoids it, and
  `reader-ab.sh` warns when it happens.
- **`_mm_prefetch` is `unsafe`** on this toolchain (it carries an `sse` target
  feature) even though SSE is baseline on x86-64 and a prefetch cannot fault.
- **Seek targets usually ascend, but not always.** Any search that resumes from
  the previous position must validate the hint and fall back (see
  `gallop_partition_point`).
- **Parallel worktrees share `/tmp` and ports.** With several agents working
  at once, binaries and `PERF_TARGET_DIR` under `/tmp` were deleted mid-session
  (2026-10-04), so keep them in a private directory such as
  `~/.cache/<name>/`. `reader-ab.sh` always binds ports 18120/18122,
  `api-diff.sh` 18140-18142, and the Playwright e2e step 4174. A second
  concurrent run fails to warm, or fails with `ERR_CONNECTION_REFUSED`; rerun
  it rather than debugging the change.

## Facts

Measured 2026-11-16 administrative I/O: a production Tokio worker blocked in
SQLite `pread64` while health requests queued. Bulk mutation summaries need a
covering index because their scalar columns follow large chunk BLOBs; completed
startup checks use a partial missing-summary index. Keep synchronous admin reads
off HTTP workers. Initial index construction still needs a one-time scan. See
[the diagnosis and proof](../performance/admin-io-2026-11-16.md).

Measured 2026-11-16 follow-up: keep `MomentCounter` timestamp-order debug state
out of release builds. Counter slots shrink 24 → 16 bytes on x86-64; three
full-reader comparisons preserved all responses and showed small, order-sensitive
cycle gains. Title-token reuse and a lower document-validation cutoff did not
clear control drift and were reverted. See the
[query-workload record](../performance/query-workload-2026-11-16.md).

Measured 2026-11-16: inlining `DocumentTable::document` removed accessor calls
and allowed callers to omit unused fields. Two full-reader trials matched all
330 responses and used 1.07–1.13% fewer instructions, with 1.63–1.83% fewer
cycles. A cold multibyte-varint split regressed and was reverted. See the
[measurement record](../performance/document-inlining-2026-11-16.md).

Measured 2026-10-17: `DocumentTable::document` retained two checked helper calls
despite the fixed 24-byte record. One complete range check plus direct LE fields
removed them without unsafe code. Five full-reader trials matched all 330
responses and used 2.31% fewer instructions at the median. See the
[assembly and measurement record](../performance/document-decoding-2026-10-17.md).

Measured 2026-10-01 to 2026-10-03:

- Production reader (`ytsearch-raw-production`): AMD EPYC, 4 vCPUs, x86-64-v3
  (AVX2, BMI2, FMA, MOVBE, POPCNT, LZCNT, F16C); no AVX-512. Resident memory
  about 430 MB, of which about 315 MB is anonymous.
- Index: 8,142,650 documents across 32,822 videos. 32,488 videos (99%) have
  contiguous document IDs; video index order follows document order.
- Profile after PR #43 (share of reader CPU): broad-scan document loop
  (`analyze_broad_plans` closure) 26%, `TextAlternativeCursor::advance_to` 12%,
  `StreamingTermPlan::seek` 11%, `TitlePlanCursor::advance_to` 8%,
  `StreamingTermPlan::take_current` 5%.
- Tried without measurable gain (2026-10-04; cycles 0.998, noise 0.4%):
  prefetching metadata in `decode_text_postings` (scoring 16 postings behind
  decoding), prefetching title targets in `decode_targeted_titles`, and
  prefetching both cache lines of a straddling record. Postings visit documents in
  ascending order with small gaps, which the hardware prefetcher already
  follows. Software prefetch paid off only in the broad scan, whose bitset
  order leaves irregular gaps.
- The broad-scan loop is now bound by memory bandwidth, because exact broad totals
  read every matched document's start time from `documents.meta` (24-byte
  records). Further gains there need a format change, such as a compact
  per-document start-time array, which is a schema decision.

## JavaScript

- V8 bytecode for one function:
  `node --print-bytecode --print-bytecode-filter=<function> file.js`. Copy the
  function into a standalone file with stubbed DOM globals.
- DOM costs only show in a browser. Measure in Chromium through Playwright's
  `page.evaluate`, with real inputs. The `textarea.innerHTML` entity decoder
  cost 48 µs per snippet there, but looked free in Node.
