# ytsearch autoresearch

This is a bounded, reproducible adaptation of Karpathy's autoresearch. The
upstream design uses a dedicated branch, one agent-editable file, an immutable
evaluator, a fixed budget, one scalar objective, and keep/discard logging. This
repository applies that contract without granting an experiment access to
staging data or deployment credentials.

Upstream reference: <https://github.com/karpathy/autoresearch>.

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

## Experimental vector loop

This is the first mechanically enforced Karpathy-style track in ytsearch.

Setup a dedicated branch and isolated worktree:

```bash
scripts/autoresearch-init.sh jul17-vector
cd /tmp/ytsearch-autoresearch-jul17-vector
```

The contract is intentionally small:

- `autoresearch/vector.env` is the only file the research agent may modify.
- `scripts/autoresearch-vector-experiment.sh` and the Rust benchmark are the
  fixed evaluator. Do not change them during a run.
- The objective is lowest median unfiltered ANN p95 across three trials.
- Minimum recall@10 is a hard 0.95 guardrail. The minimum observed recall, not
  the average, must pass.
- Qdrant is local and ephemeral. No staging service, corpus, key, or queue is
  available to the experiment.
- Commit each proposed `vector.env` change before evaluating it.

Run the baseline, then change one value and repeat:

```bash
scripts/autoresearch-vector-experiment.sh "baseline hnsw_ef=64"
# edit only autoresearch/vector.env, commit it, then:
scripts/autoresearch-vector-experiment.sh "raise hnsw_ef for recall"
```

Raw logs go under ignored `autoresearch/runs/`. The ignored
`autoresearch/vector-results.tsv` records commit, configuration, median p95,
minimum recall, status, and description. `keep` means it beat every earlier
passing run; `discard` means it failed recall or did not improve latency.

Unlike upstream, the evaluator does not automatically reset a discarded commit.
The isolated worktree protects the operator's tree, but the agent must use
`git revert` for a discarded experiment so history remains auditable. Never use
`git reset --hard`. The loop may continue autonomously only when the user has
explicitly requested ongoing experiments; it never deploys or merges itself.
