# Schema 9 P00 baseline

Status: in progress with failed gates. This directory is evidence, not a release
claim. Nothing in this capture deployed, activated, indexed, or deleted data.

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
- The exact retained schema-8 artifact path and digest were not visible from the
  active reader. Historical size/counts remain explicitly unverified.

The machine-readable facts and failures are in
[`baseline-receipt.json`](baseline-receipt.json).

## Search baseline

[`public-api-samples.txt`](public-api-samples.txt) retains every request from the
first public baseline attempt. [`public-api-summary.tsv`](public-api-summary.tsv)
contains case, attempts, errors, p50, p95, p99, and maximum milliseconds.

The attempt intentionally remains failed. The public edge accepted the first
120 requests, then returned HTTP 429 for 360 requests. `react_global` also had a
490.436 ms maximum. A later run must pace or explicitly provision the test lane,
retain errors in the denominator, and measure internal, public, and browser
timers separately. It must not discard this run or call its valid subset a pass.

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

## Next autonomous repairs

1. Resolve every failed gate in the v2 query contract using its named fixture,
   independent reference, explicit decision, and exact golden evidence.
2. Validate the fixed 300-case matrix and preserve its immutable IDs.
3. Add a single read-only P00 collector and fixture test so future captures do
   not depend on hand-assembled commands.
4. Locate and hash the retained schema-8 artifact without changing it.
5. Run paced internal/public/browser baselines against identical corpus and
   source identities, then request an independent verdict at the exact head.
