# Playbooks

Short, task-shaped notes for work that has been done before. A playbook
exists so that the next person (or agent) spends minutes, not hours,
rediscovering what is already known: which commands to run, what fooled the
last person, and the facts that are expensive to look up.

| Playbook | Use it when |
| --- | --- |
| [reader-performance.md](reader-performance.md) | Profiling or speeding up the Raw reader, or proving a reader change returns identical results |
| [dependency-updates.md](dependency-updates.md) | Updating JS or Rust dependencies, or a bundle budget fails |
| [frontend-reactivity.md](frontend-reactivity.md) | The UI flashes, remounts, refetches or loses state; reproducing timer-driven bugs |

## Format

Each playbook has three parts, in this order:

1. **Fast path**: the commands to run, in order, to get an answer.
2. **Traps**: things that cost real time before, each with the reason, so it is
   clear when one no longer applies.
3. **Facts**: measurements and environment details that are slow to
   rediscover (with the date they were measured).

Keep entries terse. Prefer a command over a paragraph, and a script in
`scripts/` over a command that needs explaining.

## The rule

When a task takes noticeably longer than it should because something was not
written down, fix that in the same pull request:

- If a command sequence was rebuilt by hand, turn it into a script and link it.
- If a trap cost time, add it to the relevant playbook with its cause.
- If no playbook fits, start one and add it to the table above.
- If an entry was wrong or stale, correct or delete it. A wrong playbook is
  worse than none.

## General traps

These apply to any long-running shell work in this repo:

- `pkill -f <pattern>` also matches the shell running it when the pattern
  appears in that shell's command line, killing your own command. Use
  `pgrep -f` to check first, or kill a saved PID.
- Background jobs in a non-interactive shell ignore `SIGINT`, so
  `kill -INT` never stops a backgrounded `perf stat`. Nest the measured
  command under `perf stat -- <command>` instead (as `scripts/perf/reader-ab.sh` does).
- Under `set -euo pipefail`, `producer | head -1` makes the producer die of
  `SIGPIPE`, the pipeline fails, and the script exits silently at that line.
  Write the producer's output to a file and read it with `grep -m1` or `awk`.
- `bun run test:e2e` (and so `check:all`) serves Vite on `--strictPort 4174`.
  Two worktrees running it at once fail with `net::ERR_CONNECTION_REFUSED`;
  give each run its own `PLAYWRIGHT_PORT`.
- On a laptop, check `powerprofilesctl get` and whether it is on battery before
  trusting any timing. Power-saver mode changed wall-clock results by up to
  2.7x between consecutive identical runs.
