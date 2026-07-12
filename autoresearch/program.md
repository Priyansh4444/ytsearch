# ytsearch autoresearch

This is a bounded, reproducible optimization loop inspired by Karpathy's
autoresearch. It improves one measurable system track at a time without
mutating production data.

## Invariants

- Never benchmark a code change against a different corpus or query set.
- Never run destructive indexing against staging.
- Never create or resize paid infrastructure unless `ALLOW_FLY_EXPERIMENT=1`
  and a maximum hourly cost, TTL, and cleanup command are supplied.
- Keep browser-facing responses as JSON unless browser compatibility and total
  end-to-end latency improve. Protobuf size or codec speed alone is insufficient.
- Use an isolated git worktree for experiments. Do not reset the operator's
  dirty working tree.
- Change one independent variable per experiment.
- A crash, timeout, HTTP error, unstable result count, or failed correctness
  check is an automatic discard.

## Tracks

Experiments within a track are comparable. Scores across tracks are not.

| Track | Primary metric | Guardrails |
| --- | --- | --- |
| Search | p95 end-to-end milliseconds | zero errors, stable corpus counts, ranking fixtures pass |
| Indexing | median milliseconds per successfully chunked video | no imports by default, same videos/cache state, same success count |
| Serialization | encode/decode milliseconds and bytes per chunk | round-trip tests pass, measure end-to-end consumer cost before adoption |
| Frontend | Core Web Vitals and transferred bytes | component tests, accessibility, and search workflow pass |
| Cost | dollars per successful workload unit | equal or better reliability and latency |

## Loop

1. Run `scripts/autoresearch-baseline.sh` on the unmodified candidate.
2. Record the commit, machine description, corpus fingerprint, and tool versions.
3. Form one falsifiable hypothesis and change only its relevant surface.
4. Run the same command with the same time budget at least three times. Use more
   repetitions for noisy network measurements.
5. Record `keep`, `discard`, or `crash` in `autoresearch/results.tsv`.
6. Keep only changes that improve the primary metric beyond noise while all
   guardrails remain green. Prefer simpler changes when results are equivalent.
7. For Fly architecture tests, deploy a separate app/volume, seed a fixed small
   fixture, apply a TTL label, benchmark it, and destroy it in the same run.

Do not autonomously promote an experiment to staging. Promotion requires a
reviewed diff plus a fresh staging smoke test before and after deployment.
