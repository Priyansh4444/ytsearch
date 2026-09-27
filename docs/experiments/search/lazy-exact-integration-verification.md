# Lazy exact prototype integration checkpoint

Verified code: `383652fe31ba5da3268bfcd64b3fc3a52a0ecf87`, based on merged
master `588dfa738d90fa22316f918838e7bc074cc47f58`. Date: 2026-09-13.
This is local prototype/test evidence, not schema-9 release acceptance.

| Gate | Observed result |
| --- | --- |
| Workspace Rust tests | 302 passed, 6 ignored, 0 failed across 28 reported suites |
| Raw all-targets tests | 168 library + 1 CLI + 15 example passed; 1 library test ignored |
| Prototype wrapper | Passed normal, repeatability, oracle/measurement fault injection, and invalid-input checks |
| Raw all-targets clippy | Passed with warnings denied |
| Rust formatting | Passed |
| Markdown links | Passed, 71 files at the measured code revision |
| Frontend build | Passed using the existing installed dependencies |
| Browser entry budget | 28,735 bytes gzip; limit 30,000 |
| Total JavaScript / CSS | 45,930 / 7,664 bytes gzip; both configured budgets passed |
| Independent reviews | Passed four corrected measurement defects and reader-lease test scope |
| Live schema-9 / production perf | Not run; no implementation or deployment claimed |

The first workspace attempt failed because the sandbox prohibited localhost
mock-server sockets (`TcpListener::bind("127.0.0.1:0")`). The rerun with local
socket access passed. The isolated frontend initially lacked Vite; reusing the
existing workspace dependencies allowed the unchanged frontend to build.

A first integrated repeatability run also failed while documentation edits
changed source provenance during the run. The clean committed-tree rerun passed;
the check now prints a compact provenance diff on mismatch. Verification must
not run concurrently with mutations to the worktree whose identity it records.

Ignored tests remain ignored; their presence is not counted as passing coverage.
This checkpoint does not substitute for the 300-query exhaustive oracle,
full-result summaries, authenticated generation-pinned sessions, no-rescan
ranking, real-corpus measurements, or 2-GB live resource/latency gates.

## Release and upload state

- Bounded-publication fix: `c60b3ad`.
- Prior PR stack #3, #4, #5: merged; final combined master CI passed.
- PR #2: still open and conflicting. Rebase deliberately or close; do not merge
  the conflict away without reconciling its documentation.
- Prototype branch: `integrate-lazy-exact-reviewed`, local only. Upload to
  `https://github.com/Priyansh4444/ytsearch.git` was blocked by the approval
  system pending explicit consent for this payload/destination. No new PR exists.
- No production deployment/switch or retained schema-8 artifact mutation occurred.

See [corrected benchmark evidence](lazy-exact-measurement-harness.md) and
[reader-lease scope](generation-pin-foundation.md).
