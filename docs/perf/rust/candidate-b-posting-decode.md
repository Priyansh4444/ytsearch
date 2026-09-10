# Arena candidate B — Raw posting decode: minimum work vs. emitted machine code

Audited commit `cc9044f` (`perf/frontend-render-path`). Every number below comes
from a command in this document; anything not measured is marked **hypothesis**.
No `src/`, no `docs/perf/arena/*`, no deploy, no network. Helper scripts lived in
`/tmp/rawbench` (outside the repo) and the corpus was built there too.

## 1. The one hot path

**Chosen:** the per-term posting-list decode + BM25 scoring loop —
`matches_for_entry`, which inlines `decode_text_postings`
([engine.rs:1272](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1272)), its targeted twin
`decode_targeted_text` ([conjunction.rs:559](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/conjunction.rs#L559)), and the
shared primitives `format::read_varint` ([format.rs:93](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/format.rs#L93)) and
`DocumentTable::document` ([storage.rs:260](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/storage.rs#L260)).

*Input:* `q` (≤ 512 B), unfiltered; the API sends `group_by=video_id`,
`group_limit=3`, `limit=per_page+1` (`youtube_api::search_handler`), so the grouped
path is the production default. *Output:* `{found, total_moments, out_of,
grouped_hits[]}`. *Index:* immutable mmap generation — `lexicon.bin` (sorted,
binary-searched), `postings.bin` (per-term varint `gap,frequency` + first
positions), `documents.meta` (24 B records), `documents.blocks` (zstd, 64
docs/block). *Platform:* Fly `shared-cpu-2x`, 2 GB, `sjc`, x86-64, glibc malloc (no
`#[global_allocator]`), `[profile.release] strip = true`, no LTO/target-cpu
(`.cargo/config.toml`, `fly.raw-search.toml`; the overlay image builds with
`rust:1.97-bookworm`); search runs on a `spawn_blocking` thread serialized by
`MAX_CONCURRENT_BACKEND_SEARCHES = 1` (`youtube_search_service/src/lib.rs:31`).
Gates: "every measured canonical request below 200 ms" with zero validation
errors (`docs/operations/raw-schema8-validation.md`), 5 s edge timeout, 2 s health
probe. That document names the slowest canary queries — `and the`, `in the`,
`this is the` — i.e. exactly this loop.

## 2. What actually executes

Local corpus (4000 videos × 25 chunks): `./target/release/youtube-raw-search build …`
→ `documents 100000, videos 4000, terms 48010, index_bytes 70644984`. A lexicon
parse gives `the` = 70,543 text postings in 141,086 B and `and` = 70,177 in
140,354 B — 2.0 B per posting, a 1-byte gap plus a 1-byte frequency.

Profile of the production-shaped path (grouped, `and the` + `this is the`,
800 searches, `taskset -c 4`, Intel Core Ultra 7 255H):

```bash
taskset -c 4 perf record -F 2000 -o perf.data -- ./hotpath-bench <index> 400 "and the" "this is the"
perf report --no-children --percent-limit 1.0 --sort symbol
 14.78% DocumentTable::document           6.86% format::read_varint
 10.21% RawSearchEngine::matches_for_plan 5.91% format::read_u32
  8.47% query_support::retain_best_document 5.89% query_support::union_sorted
  4.67% hash_one (SipHash, HashMap<u32,_>) 3.99% record_group_match_values
  3.69% Vec<DocMatch> from_iter            3.15% matches_for_entry
  3.13% ZSTD_decompressSequences_bmi2
```

Temporary `AtomicU64` counters in `read_varint` / `document` / `search_document` (reverted, §5):

| query | `document()` calls | `read_varint` calls | postings walked | queried lists |
| --- | ---: | ---: | ---: | ---: |
| `and the` | 373,592 | 563,316 | 187,772 | 140,720 (2 lists) |
| `this is the` | 140,876 | 211,044 | 70,348 | ~211,000 (3 lists) |
| `the w123` | 491 | 2,605,112 | 868,371 | 70,555 (2 lists) |

Every walked posting costs exactly 3 varint decodes; `document()` is called ~2.0×
per walked posting (the decode loop, then the grouping loop re-decoding the same
record). `the w123`, which returns 2 hits, walks 868,371 postings — 12.3× the
`the` list and 25% of the corpus's 3,503,235 text postings — because the
typo/prefix/drop ladder re-scans the same list.

### Assembly

`cargo rustc --release --locked -p youtube_raw_search --lib -- --emit asm`:

* `format::read_varint` is **not inlined**: 155 instructions of body, 12 `callq`
  (alloc shims), `pushq %r14; pushq %rbx; subq $104,%rsp`, returning the 32 B
  `Result<u32, Error>` through a hidden pointer, with the `String`-allocating
  error path inline in the hot function:

```asm
_RNvNt…6format11read_varint:
    pushq %r14 ; pushq %rbx ; subq $104, %rsp
    movq (%rcx), %rax ; movq %r8, %r9 ; subq %rax, %r9 ; jb .LBB1327_5
.LBB1327_5:                                   # reached on every bounds failure
    callq *__rust_alloc@GOTPCREL(%rip)        # Error::Corrupt(String)
```

* The caller pays a memory round-trip per varint (`matches_for_entry`, baseline):

```asm
.LBB1116_33:                                  # ×3 per posting (gap, freq, position)
    leaq 8(%rsp), %rdi ; movq 40(%rsp), %rsi ; leaq 96(%rsp), %rcx ; movq %rdx, %r8
    callq …6format11read_varint
    movq 8(%rsp), %r13 ; movl 16(%rsp), %r12d ; cmpq $-1, %r13 ; jne .LBB1116_35
    …                                         # then: callq DocumentTable::document
    sqrtss %xmm0, %xmm0 ; divss %xmm0, %xmm1 ; … 3 × divss per posting
```

* `DocumentTable::document` = 115 instructions of body, 6 saved registers, 56 B
  frame, **6 `callq read_u32`**; `read_u32` is itself a 43-instruction function
  with 4 error-path `callq`s. Reading the one field the decoder uses (`tokens`,
  +16) therefore costs ~1 call + 6 calls ≈ 370 instructions of code.
* Static sizes: `matches_for_entry` 762 instructions / 41 `callq`; `read_varint`
  155 before / 140 after the change in §4.

## 3. The minimum, and what is incidental

For `T` terms, term `t` with `P_t` postings packed in `L_t` bytes, `S_t`
surviving postings, default `prioritize_token_position = true`:

| Quantity | Lower bound | Why |
| --- | --- | --- |
| bytes + bit tests | ≥ Σ ⌈L_t/8⌉ loads and Σ L_t tests | self-delimiting, no skip structure: reaching posting *k* requires walking 0…*k*, and each byte's bit 7 decides varint length. 8 B/load only with a continuation-bit mask; a byte loop costs Σ L_t |
| varint values | Σ (2P_t + P_t) | gap, frequency, first position — exactly 3.0 per walked posting, measured 3.0 |
| metadata reads | Σ S_t × 4 B | BM25 needs `tokens` only; 24 B striding ⇒ ≥ 1 cache line per posting |
| float ops | 13/posting | bit-exact BM25 + position boost = 3 `divss`, 1 `sqrtss`, 8 mul/add, incl. the loop-invariant `idf*4.0` that must stay unhoisted |
| writes | Σ S_t × 12 B | `DocMatch{u32,f32,u8}`; plus Θ(V) group records |

**Unavoidable:** the byte stream, 3 decodes/posting, one `tokens` load per posting, the float chain, the match writes. **Incidental (measured):**

1. **16 instructions of call/epilogue overhead per varint.** 563,316 × 16 = 9.2 M of the 209.8 M instructions per `and the` search — five duplicated `Ok` epilogues in the callee plus the inline `String` error paths.
2. **~63 instructions per wasted metadata decode.** P1a+P1b below cut 23.6 M instructions over 373,592 decodes = 63 each: all six fields were decoded (115-instruction callee + 6 × 43-instruction `read_u32`) to use 4 bytes.
3. **≈2 record decodes per walked posting** — the grouping loop re-decodes each record the decode loop already read.
4. **Repeated posting-list walks** (`the w123`: 2.6 M varint decodes for 2 hits); no per-request term memo exists although a generation is immutable.

## 4. The change (measured)

Three edits: decode only the fields the caller uses (P1a decode loops, P1b record width for the grouping/format/proximity loops), and stop duplicating the success epilogue and error constructors in the decoder (P2).

`perf stat -e cpu_core/instructions/`, 21 searches per process, one pinned core,
instructions per search:

| query | base | P1a | +P1b | +P2 |
| --- | ---: | ---: | ---: | ---: |
| `and the` | 209.80 M | 191.71 M (−8.6%) | 186.21 M (−11.3%) | **177.00 M (−15.6%)** |
| `this is the` | 113.78 M | 106.32 M (−6.6%) | 100.27 M (−11.9%) | **91.73 M (−19.4%)** |
| `the w123` | 92.04 M | 91.78 M (−0.3%) | 91.69 M (−0.4%) | **82.08 M (−10.8%)** |
| `w123 the` | 129.58 M | 129.62 M (0%) | 129.68 M (0%) | **120.50 M (−7.0%)** |

Wall clock, interleaved A/B, min of 3 processes × 40 iterations, grouped (load average was 8–12 during the audit, so only same-session pairs are compared):

| query | base | patched | Δ |
| --- | ---: | ---: | ---: |
| `and the` | 13.262 ms | 12.308 ms | −7.2% |
| `this is the` | 4.803 ms | 4.175 ms | −13.1% |
| `the w123` | 3.478 ms | 3.293 ms | −5.3% |
| `w123 the` | 9.821 ms | 9.680 ms | −1.4% |
| `"and the"` (quoted) | 9.721 ms | 8.676 ms | −10.8% |
| ungrouped `and the` | 35.605 ms | 35.662 ms | 0% (memory-bound sort) |

Behaviour: identical response bytes. `parity.sh` compares JSON hash **and** byte length across 10 query shapes (`and the`, `this is the`, `the w123`, `w123 the`, `fable on high`, `"and the"`, `"black hole"`, `*`, `the`, `a`) in grouped and ungrouped mode: 20/20 identical.

`#[inline(always)]` on `read_varint` was measured too: **0** `callq` sites instead
of 28 but **+58.7 KB** of emitted assembly (+0.4%), retired instructions unchanged (3.7172 G vs
3.7179 G) → rejected. `#[inline]` alone changed nothing (3.7176 G vs 3.7179 G),
so the gain is the outlined error path, not inlining, and the patch keeps no
inline attribute. (The wall-clock table used the `#[inline]` variant; the shipped
variant differs by ≤ 0.02% instructions.)

## 5. Exact diffs applied (all reverted; `git status --porcelain` is empty)

Compressed record of the 189-line unified diff — only the six identical
`?`-site substitutions are folded. Nothing else was edited.

```diff
--- a/crates/youtube_raw_search/src/engine/storage.rs   (after `document`, :273)
+    /// Token count for BM25 scoring: the text posting decoders need exactly
+    /// this field, so a six-field decode is wasted work per posting.
+    pub(super) fn document_tokens(&self, id: u32) -> Result<u32> {
+        if id as usize >= self.documents {
+            return Err(Error::Corrupt(format!("document {id} is out of bounds")));
+        }
+        read_u32(self.file.bytes(),
+            METADATA_PREFIX_BYTES + id as usize * DOCUMENT_RECORD_BYTES + 16)
+    }
--- a/crates/youtube_raw_search/src/format.rs  (read_varint, :93)
+    /// Build a corruption error out of line: keeping the String-allocating
+    /// constructors inline duplicated the success-path epilogue at each return.
+    #[cold] #[inline(never)]
+    fn malformed(detail: &'static str) -> Error { Error::Corrupt(detail.to_string()) }
-        .ok_or_else(|| Error::Corrupt("postings range extends beyond file".to_string()))?;
+        .ok_or_else(|| malformed("postings range extends beyond file"))?;
-    … the same substitution at the 5 × "invalid postings varint" sites and 1 × "overflows u32" …
--- a/crates/youtube_raw_search/src/engine.rs
-:1323  let record = self.inner.metadata.document(document)?;
+:1323  let document_tokens = self.inner.metadata.document_tokens(document)?;
-:1326      record.tokens,         +:1326      document_tokens,
-:801 :968 :997 :1772 :1797   document(candidate.document())? → search_document(…)?
-                             (those callers read only video / video+t_start_ms)
-:1632  record: DocumentRecord, +:1632  record: SearchDocumentRecord,
--- a/crates/youtube_raw_search/src/engine/conjunction.rs
-:594   let record = self.inner.metadata.document(document)?;
+:594   let document_tokens = self.inner.metadata.document_tokens(document)?;
-:598       record.tokens,       +:598       document_tokens,
```

Instrumentation used for the call counts, also reverted: `pub static
READ_VARINT_CALLS` + one `fetch_add` in `format::read_varint`; `DOCUMENT_CALLS` /
`SEARCH_DOCUMENT_CALLS` + one `fetch_add` in `DocumentTable::{document,
search_document}`; `mod storage`/`mod format` widened to `pub(crate)` and one
`pub fn perf_document_calls()` added in `lib.rs`. Removed with `git apply -R` /
`git checkout --`.

Temporary harness (deleted; `git reset` unstaged it): 68 lines at
`crates/youtube_raw_search/src/bin/hotpath-bench.rs`, core loop:

```rust
for grouped in [false, true] {                        // production default is grouped
    params.group_by = grouped.then(|| "video_id".to_string());
    params.group_limit = grouped.then_some(4);
    engine.search(&params)?;                          // warm
    for _ in 0..iterations {
        let started = Instant::now();
        let out = engine.search(&params)?; std::hint::black_box(&out);
        samples.push(started.elapsed().as_secs_f64() * 1e3);
    }                                                 // prints median/p95/min,
}                                                     // JSON hash and byte length
```

## 6. Verification

```bash
cargo test --release --locked -p youtube_raw_search  # 162 + 1 tests passed (patched)
cargo fmt --check                                    # clean (patched)
./parity.sh base ; ./parity.sh final ; diff          # 20/20 identical responses
git status --porcelain                               # empty after revert
```

## 7. Rationale — considered, rejected, unverified

* **Hoist the loop-invariant `idf * TEXT_QUERY_WEIGHT`** (2 of the 13 flops):
  `bm25 * idf * 4.0` evaluates left-to-right, so precomputing `idf * 4.0` changes
  f32 rounding and can flip ties in `descending_doc_score` — rejected without a
  ranking-parity corpus. Same for `0.15 * rsqrt(x)` vs `0.15 / sqrt(x)` and
  `tokens * (B/avg)` vs `tokens/avg * B`.
* **`select_nth_unstable` in `format_ungrouped`** (the grouped path already does it in `sort_group_window`): the ungrouped comparator calls `video_for_document` twice per comparison — ~1.5 M × 2 record decodes for the 91 k-match `the` query, measured 60 ms vs 1.9 ms grouped. Real, but the production default is backend-grouped, so it is not the p95 gate — recorded, not patched.
* **Per-request memo of a decoded term list** (§3.4): the largest single waste found (≈ 25% of the corpus's text postings for a 2-hit query); safe in principle because a generation is immutable, but it reshapes plan evaluation and needs a `(term, field, prioritize_position)` key with the exact mask applied at consume time. **Hypothesis:** ladder-heavy queries gain most; confirm on a corpus whose term frequencies match production — mine are synthetic.
* **SWAR/SIMD varint scan**: the byte stream is not where the time goes (6.9%
  profile share); premature. Same for `target-cpu=native`/LTO (both change the
  deployed image; `docs/rust-performance-record.md` already rejected Thin LTO)
  and for an allocator swap (needs its own cost gate).
* **Could not verify here:** the deployed toolchain (`rust:1.97-bookworm`; this
  audit used nightly 1.100.0, so instruction counts may differ), the 8.32 M
  document corpus, real `shared-cpu-2x` timing, and the API's query mix. The
  milliseconds above are **not** production numbers; the direction, parity
  evidence, and attribution arithmetic are what the evidence supports (16
  instr/varint and 63 instr/record decode both match the measured call counts).
