# Rust perf analysis — synthesis and independent verification

Three independent audits of the Raw posting path were run in parallel
(`candidate-a-term-scan.md`, `candidate-b-posting-decode.md`,
`candidate-c-raw-posting-decode.md`). This file records the pick, the grafts, the
rejections, and the **independent re-measurement done by the implementing agent**
(the candidates' own numbers are not taken on trust).

## Pick and grafts

- **Base: candidate C.** It had the full chain (grounding → mathematical minimum
  → real assembly → measurement → revert record) and priced three separable
  changes, two of which it measured individually.
- **Grafted from A:** the finding that production multi-term queries go through
  `conjunction.rs`'s streaming plan rather than this batch decoder, so the win is
  scoped to single-term filtered, title-expanded, phrase, and materialized
  multi-term conjunction paths (confirmed by the independent review below) — but not
  to single-term unfiltered queries, which never enter this loop. Its
  `DocumentTable::tokens` accessor is the same idea as C's R2.
- **Grafted from B:** the parity method — byte-identical responses (hash +
  length) rather than only "tests pass" — and `#[cold]` on the debug error paths
  as a follow-up.
- **Rejected / deferred (with reasons):**
  - A's columnar `tokens` layout (49.2 → 9.7 ns/posting): real, but it changes the
    on-disk index format. Separate PR with a rebuild/migration story.
  - A's title-heap removal and driver pass fusion: hypotheses, not measured.
  - A/B's `select_nth_unstable` / sort changes: not the production path.
  - B's per-request term memo (868k postings walked for a 2-hit query): the
    biggest structural waste, but it is a product/UX trade (exact `found` totals),
    not a safe micro-patch.
  - Reordering the score computation (hoisting `idf` or `0.15*rsqrt`): changes f32
    rounding, which can flip tie order. Parity risk without a measured win.
  - B's claim that `#[inline(always)]` is a no-op is **refuted for this build** —
    see the deterministic check below.

## Independent verification (implementing agent, not the candidates)

Corpus: 50,000 synthetic chunks / 2,000 videos / 73 terms, built with the real
`youtube-raw-search build` (33 MB index). Harness: a temporary `#[ignore]` test
calling the private `matches_for_term(term, fields)` (text,title), min of 11
iterations, `black_box`ed. Removed before commit; nothing else was instrumented.

| term    | postings |      baseline ns/posting | patched ns/posting |       speedup |
| ------- | -------: | -----------------------: | -----------------: | ------------: |
| `the`   |   47,886 | 16.89 / 18.63 (two runs) | 4.97 / 4.38 / 4.49 | **≈3.4–4.2×** |
| `fable` |    2,971 |            15.02 / 17.73 | 7.24 / 5.73 / 7.34 | **≈2.2–2.6×** |

**Superseded.** Those baseline runs were anomalously slow. The machine-profile
section below, plus an independent review that measured a 10.9–11.7 ns/posting
baseline on the same hardware class, put the transferable figure at
**≈2.3–2.8× on the decode loop**, not the 3.4–4.2× row above.

Posting counts are identical before and after (47,886 / 2,971), and the crate's
own suite is green on the patched tree: `cargo test --release -p youtube_raw_search`
→ **162 passed, 0 ignored**, plus the CLI integration test (the temporary timing
harness was removed before commit, so nothing is ignored in the committed tree). `cargo clippy
--workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

**Caveat and scope.** These were loaded 16-core laptop runs and the absolute
ns/posting figures move with load (baseline varied 15.0–18.6 across runs), which
is why the machine-profile section below is the one to trust. The scope is
narrower than "all queries" and wider than "single-term queries": an independent
review with its own harness and corpus measured the end-to-end effect as
filtered `the` 4.49 → 2.24 ms, `the and of` 19.6 → 15.1 ms, and `"the fable"`
4.42 → 3.35 ms, while single-term unfiltered `the` was unchanged within noise (it
takes the `broad-single` fast path and never enters this loop).

## Deterministic evidence for `#[inline(always)]`

Timing on a loaded machine cannot settle whether forced inlining is doing
anything, so `nm` was used on the release artifacts:

```
# baseline
$ nm -A --demangle target/release/libyoutube_raw_search.rlib | rg read_varint
  ...cgu.01.rcgu.o: U youtube_raw_search::format::read_varint
  ...cgu.03.rcgu.o: U youtube_raw_search::format::read_varint
  ...cgu.05.rcgu.o: U youtube_raw_search::format::read_varint
  ...cgu.06.rcgu.o: U youtube_raw_search::format::read_varint
  ...cgu.10.rcgu.o: T youtube_raw_search::format::read_varint

# patched (with #[inline(always)])
  (only) ...read_varint::{closure#2}   — the cold error builder
```

Baseline has an outlined `read_varint` in one codegen unit called from four
others; with `codegen-units = 16` and no LTO those cross-unit calls cannot be
inlined, so the decode loop pays a call per varint. The patched artifact has no
outlined definition and no references left. That is a structural fact, not a
timing, and it is why the change is kept despite candidate B's contrary
measurement (which likely reflects a different CGU partition where the call had
already been inlined).

## Later candidates (D) and what happened to them

A fourth audit (`candidate-d-raw-query-path.md`) landed after this change was
written. Its findings, and the disposition:

| D's finding                                                                                                                                                          | Status                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Lexicon binary-search probes decode six posting fields and validate UTF-8 per probe, though only the equality hit needs them (measured 560 → 390 ns/lookup, −31–38%) | **Measured, deliberately not shipped.** `lexicon.lookup` is called 1–3 times per query (`single_token.rs` term + plural fallback, `conjunction.rs`), so the saving is ≈0.2–0.5 µs per query against 7–20 ms budgets. That is not worth rewriting a correctness-critical binary-search probe (byte-compare vs `str`-compare, plus UTF-8 validation timing) in a PR that is otherwise mechanical. Keep as a candidate if a future path ever loops over the lexicon. |
| Tokens-only metadata read: `document()` 15 ns vs `search_document()` 5.4 ns (≈ −10 ns/posting)                                                                       | Corroborates the shipped `document_tokens` accessor (same mechanism, independent measurement).                                                                                                                                                                                                                                                                                                                                                                    |
| Varint inlining repro: 2.4 → 1.2 ns/varint                                                                                                                           | Corroborates the shipped `#[inline(always)]`.                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Thin LTO: no gain (112.5–116 vs 108.5–121.5 ms)                                                                                                                      | Independent agreement with the build-flag experiment above — LTO stays off.                                                                                                                                                                                                                                                                                                                                                                                       |
| "Verbatim-inline split", bigram-side benefit, live-index latency                                                                                                     | Labelled hypotheses by D itself; not acted on.                                                                                                                                                                                                                                                                                                                                                                                                                    |

## Follow-ups this analysis surfaced but did not implement

1. Columnar `tokens` (A: 49.2 → 9.7 ns/posting on the loop) — needs an index
   format version and a rebuild plan.
2. `#[cold]` on the `read_varint`/`read_u32` error constructions (B).
3. Title-heap removal and streaming-driver pass fusion (A, unmeasured).
4. Exact-total vs early-exit trade for `found` (A, B) — product decision.
5. **Page-cache policy on the reader (machine-specific, unmeasured).** The index is
   mapped with `MmapOptions::new().map(&file)` — no `madvise` hints anywhere
   (`format.rs`), while `memmap2` 0.9 exposes `advise_range`. The reader has 2 GB
   of RAM, and at this change's local density (500k chunks → 704 MB) the
   production corpus (~8.32M documents) would be roughly an order of magnitude
   larger than RAM, so the query path is dominated by faults rather than
   instructions: per-posting metadata reads are effectively random over the
   document table, while posting streams are scanned sequentially, and the
   kernel's default readahead assumes the latter everywhere. Candidate (do NOT
   apply blind): `Advice::Random` on the metadata/document region and
   `Advice::Sequential`/`WillNeed` on the postings streams; `MAP_POPULATE` would
   be actively harmful for an index larger than RAM. This cannot be measured from
   a dev box — a local file stays in page cache — and `flyctl ssh` to the reader
   timed out from here, so it needs a measurement on the machine itself (or a
   canary with Axiom `backend_search_ms` before/after).

## Machine profile — measured for the Fly target, not the dev laptop

The first numbers above came from a 16-core laptop (Intel Core Ultra 7 255H, 24 MB
L3). The production target is a **Fly `shared-cpu-2x` / 2048 MB** reader in `sjc`
(`flyctl machines list -a ytsearch-raw-production`), built by
`cargo build --locked --release` with **no `RUSTFLAGS` and no profile overrides**
— i.e. `codegen-units = 16`, `lto = false`, and the **generic x86-64** feature
baseline (no AVX2), because no `target-cpu` is set anywhere.

Everything below was re-measured with `taskset -c 0,1` (two pinned vCPUs) to
approximate that machine class. It is an approximation, not an emulation: Fly
`shared-cpu-*` vCPUs are quota-throttled and share physical cores with other
tenants, while `taskset` pins two dedicated cores of a 16-core laptop. The
_relative_ before/after behaviour is what should travel; absolute ns/posting on
the real machine will be higher and more variable.

| index                    | term    | postings | baseline ns/posting | patched ns/posting |   speedup |
| ------------------------ | ------- | -------: | ------------------: | -----------------: | --------: |
| 33 MB (50k chunks)       | `the`   |   47,886 |       11.42 / 13.61 |        4.45 / 3.98 | ~2.8–3.2× |
| 33 MB                    | `fable` |    2,971 |       12.95 / 13.38 |        7.36 / 5.39 | ~1.8–2.5× |
| **704 MB (500k chunks)** | `the`   |  477,746 |       12.21 / 13.36 |        4.96 / 4.49 | **~2.7×** |
| **704 MB**               | `fable` |   29,148 |       11.11 / 11.86 |        6.92 / 6.84 | **~1.7×** |

The 704 MB index is 21× larger than the 33 MB one and far larger than the local
24 MB L3, so it exercises the DRAM-backed regime rather than a cache-resident
one. Posting counts are identical in every configuration.

### Build-configuration experiments (both negative)

| variant                                           |       `the` |     `fable` | verdict     |
| ------------------------------------------------- | ----------: | ----------: | ----------- |
| default (`cgu=16`, `lto=false`, generic x86-64)   | 4.96 / 4.49 | 6.92 / 6.84 | reference   |
| `-C target-cpu=x86-64-v3` (EPYC/Rome-class, AVX2) | 4.73 / 4.44 | 8.05 / 6.21 | **no gain** |
| `lto="thin"`, `codegen-units=1`                   | 4.91 / 4.56 | 7.42 / 6.93 | **no gain** |

The loop is branchy varint decoding with scalar loads; it does not vectorize, and
`#[inline(always)]` already removed the cross-CGU calls LTO would have addressed.
**Recommendation: change nothing in the build configuration.** Widening
`target-cpu` would add a real `SIGILL` risk if the image were ever scheduled onto
a host without those features, for no measured benefit.

### What this profile still cannot prove

The production reader carries ~8.32 M documents on a 2 GB machine, so the real
index is far larger than RAM and a query walks pages that must be faulted in from
the volume. That cold regime is not reproducible with a local 704 MB file that
stays in page cache, and its cost is dominated by fault latency rather than
instructions — so the percentage win there is bounded by the fault cost, not by
this table. The instruction-count reduction is still real work removed, but the
end-to-end effect on production should be confirmed from Axiom `search_query`
(`backend_search_ms`, `duration_ms`) before/after a canary deploy.
