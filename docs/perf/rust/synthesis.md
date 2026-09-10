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
  scoped to single-term, filtered, title-expanded and phrase paths. Its
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

| term | postings | baseline ns/posting | patched ns/posting | speedup |
| --- | ---: | ---: | ---: | ---: |
| `the` | 47,886 | 16.89 / 18.63 (two runs) | 4.97 / 4.38 / 4.49 | **≈3.4–4.2×** |
| `fable` | 2,971 | 15.02 / 17.73 | 7.24 / 5.73 / 7.34 | **≈2.2–2.6×** |

Posting counts are identical before and after (47,886 / 2,971), and the crate's
own suite is green on the patched tree: `cargo test --release -p youtube_raw_search`
→ **162 passed, 1 ignored**, plus the CLI integration test. `cargo clippy
--workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

**Caveat.** This is a loaded 16-core laptop against a synthetic corpus, and the
absolute ns/posting figures move with load (baseline varied 15.0–18.6 across
runs). The direction and rough magnitude replicated across every run; the
end-to-end `search()` effect was **not** cleanly measurable at this scale
(single-term unfiltered queries take the `broad-single` fast path and never enter
this loop), so no end-to-end claim is made. Confirm on production-shaped data.

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

## Follow-ups this analysis surfaced but did not implement

1. Columnar `tokens` (A: 49.2 → 9.7 ns/posting on the loop) — needs an index
   format version and a rebuild plan.
2. `#[cold]` on the `read_varint`/`read_u32` error constructions (B).
3. Title-heap removal and streaming-driver pass fusion (A, unmeasured).
4. Exact-total vs early-exit trade for `found` (A, B) — product decision.
