# API changes

## Fast path

```bash
# Every benchmark case x sort x page, single-video searches ("show all
# moments") for each case's top two videos, and a few other endpoints and
# errors, through the API built at both refs against one local reader. Prints
# `requests=N mismatches=M` and the first difference of each mismatch.
scripts/perf/api-diff.sh origin/master .
```

Run it before shipping any `youtube_api` refactor that should not change
responses. Only timing fields may differ. It serves the same index as the
reader tools (`RAW_INDEX` overrides it) and the cases in
`scripts/benchmark-search-cases.tsv`, shared with `reader-capture.ts` through
`scripts/perf/search-cases.ts`.

## Traps

- **Local `master` can be far behind.** Linked worktrees share refs with the
  main checkout, where `master` may be stale (282 commits behind on
  2026-10-05). Compare against `origin/master` after `git fetch`.
- **The reader is built from the head ref**, since an old base cannot read the
  schema-9 index; set `READER=<binary>` to pin one.
- **Prove a probe reaches the response.** `SearchVideoResult::from_hit` sets
  `embed_url`, but `video_results_from_hits_with_policy` overwrites it later,
  so a probe there showed 0 mismatches. Check the probe with `curl` first.
- **Run with production's flags.** Both APIs get the ranking flags from
  `fly.raw-api.toml`; without `YTSEARCH_PRESERVE_BACKEND_RELEVANCE_ORDER`, a
  difference on the production path would never show.
- **An intended change hides the next one**: each request reports only its
  first difference. Skip known changes by exact path, e.g.
  `API_DIFF_IGNORE=body.hits scripts/perf/api-diff.sh <old-prod-commit> .`
  (how the 2026-10-06 API release was checked against the 2026-09-24 one).
- Ports 18140-18142 are reserved for this script.
