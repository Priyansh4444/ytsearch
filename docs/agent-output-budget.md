# Agent command-output budget

Coding agents should route noisy shell commands through
[RTK](https://github.com/rtk-ai/rtk), an Apache-2.0 Rust CLI proxy. RTK filters
passing test output, groups repeated diagnostics, and condenses common Git and
search commands while preserving exit status. Full output remains available by
rerunning a command with `rtk proxy` when diagnosis needs it.

Install and configure Codex globally:

```sh
cargo install --git https://github.com/rtk-ai/rtk --tag v0.43.0 --locked
RTK_TELEMETRY_DISABLED=1 rtk init -g --codex
rtk telemetry disable
```

Restart Codex after installation. Codex does not currently expose RTK's
automatic Bash rewrite hook, so its integration adds a global instruction that
routes eligible commands through RTK. Commands whose exact raw output is the
subject of the task should use `rtk proxy` or the underlying command directly.

Measure this repository rather than relying on upstream estimates:

```sh
scripts/benchmark-agent-output.sh | tee /tmp/agent-output-benchmark.tsv
```

The benchmark compares representative Git history, recursive search, and Rust
workspace test output. It records exit status, lines, bytes, approximate tokens,
and wall-clock time. Approximate tokens are bytes divided by four; provider
tokenizers differ, so this is a stable comparison metric rather than a billing
measurement.

## Baseline on this repository

Measured on 2026-07-17 with RTK 0.43.0 and warm build caches:

| Case | Raw approximate tokens | RTK approximate tokens | Reduction | Added time |
| --- | ---: | ---: | ---: | ---: |
| Git log, 30 commits | 1,315 | 531 | 59.6% | 34 ms |
| Recursive search | 5,245 | 4,711 | 10.2% | 26 ms |
| Rust workspace tests | 2,238 | 13 | 99.4% | 58 ms |
| Combined | 8,798 | 5,255 | 40.3% | — |

All paired commands returned the same exit status. RTK is most valuable for
passing test/build output and verbose Git commands. Search output remains large
when most matching lines are unique, so agents should still narrow patterns,
paths, and result counts at the command level.
