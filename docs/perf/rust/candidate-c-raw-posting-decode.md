# Candidate C — Raw posting decode and BM25 scoring

Audit date: 2026-09-10. Scope: the one loop that turns a query term into scored document
matches in `youtube_raw_search`, plus the two changes that loop justified.

## 1. Grounding

**Chosen path.** `RawSearchEngine::matches_for_term` ([engine.rs:1265](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1265)) → `matches_for_entry`
([engine.rs:1272](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1272)) → `decode_text_postings` ([engine.rs:1301](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1301)): lexicon lookup →
varint posting decode → BM25 + position scoring → `Vec<DocMatch>`, plus the title variant
([engine.rs:1343](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1343)) and the merge. Every concrete query term reaches it: single-token
searches ([single_token.rs:65](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/single_token.rs#L65)), every term of a conjunction, prefix/fuzzy
alternatives ([engine.rs:1442](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1442)), phrase candidate sets.

**Input / required output.** One lowercased term; its lexicon entry (term id, text/title byte
ranges, frequencies) and three mmap'd streams: `postings.bin` (delta-varint doc id, varint
frequency per posting), `positions.bin` (one varint first-position per posting, parallel to the
postings), `documents.meta` (24-byte records, `tokens` at `+16` — writer
[builder.rs:627](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/builder.rs#L627)). Output: one 12-byte `DocMatch { document: u32, score: f32, flags: u8 }`
([engine.rs:430](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L430)) per posting, ascending by `document`, `score` exactly
`bm25(freq, tokens(d), avg_tokens) * idf(t) * 4.0 * (1 + 0.15/sqrt(p+1))`
([query_support.rs:971](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/query_support.rs#L971)); ranking fixtures and the parity gate compare order, so the
float evaluation order is contract. Callers then `retain_not_excluded`, union with
prefix/split/fuzzy alternatives, early-exit at `reaches_result_threshold`
([single_token.rs:167](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/single_token.rs#L167)), then group and format.

**Budget and platform.** Production reader `ytsearch-raw-production` is `shared-cpu-2x`,
2 GiB, x86_64, glibc `malloc`, tokio multi-thread + `spawn_blocking`, and backend searches are
**serialized to one at a time**
([youtube_search_service/src/lib.rs:31](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_search_service/src/lib.rs#L31)): four concurrent large posting scans once
exceeded a 2 GiB Machine, so this loop's transient `Vec<DocMatch>` is a production memory limit.
Documented gate: 200 ms search ceiling; measured public p95 254–366 ms, 812 MiB cgroup RSS
([raw-production-cost.md](../../operations/raw-production-cost.md)). Index: 8.32 M documents.

**Build profile.** `.cargo/config.toml` is exactly `[profile.release] strip = true`; no
workspace override, so release uses defaults (`opt-level=3, lto=false, codegen-units=16`).
The shipped artifact is stripped: `nm target/release/youtube-raw-search` → `no symbols`.

## 2. Mathematical minimum

For a term with `P` text postings over `D` documents:

| Item | Why it must happen | Minimum count |
| --- | --- | --- |
| posting bytes | delta + frequency per posting | measured **2.00 B/posting** (head term), 2.16 B (3 k postings), 2.95 B (41 postings) |
| position bytes | 1 varint per posting | measured **1.00 B/posting** |
| metadata | BM25 needs `tokens(d)` | 4 B used of a 24 B record ⇒ 1 cache line / 2.7 postings |
| decode | 3 varints | 3 loads + 3 sign tests + masks ≈ 8 instr |
| score | exact formula | 2 `divss` + 1 `sqrtss` + ~6 mul/add |
| output | 12 B/posting + grow check | ≈ 4 instr |

Lower bound **≈ 26 instructions per posting**: ≈ 3 B read from the two streams, one 4-byte field
out of the 24-byte metadata record (1 cache line per 2.7 postings), 12 B written. Per term that is
≈ 26 P instructions; q terms are q independent lists to merge.

**Unavoidable.** Walking all `P` postings (the caller needs the exact `found` total, §4); one
random `tokens` fetch per posting; both divisions and the `sqrt` (score parity); `12P` bytes of
output; the title→document expansion ([engine.rs:1380](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1380)) when the term is in titles.

**Incidental.** Per posting: 3 non-inlined calls to `read_varint` and 1 to
`DocumentTable::document`, each with `Result` stack traffic and a cursor kept in memory (it is
passed as `&mut usize`); materialising six metadata fields when one is used; recomputing
`0.15/sqrt(p+1)` even when `prioritize_position = false`; an extra `Vec` allocation plus a full
move in `union_sorted` ([query_support.rs:376](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/query_support.rs#L376)) when one side is empty.

## 3. What the machine code actually does

```bash
nm target/release/youtube-raw-search    # "no symbols": the shipped binary is stripped
cargo rustc --release -p youtube_raw_search --lib -- --emit asm -C debuginfo=0   # writes the .s
# → target/release/build/youtube_raw_search/<hash>/out/youtube_raw_search-<hash>.s
```

The decoder is inlined into `matches_for_entry` (`_RNvMs1_..._17matches_for_entry`, block
`.LBB1116`, 918 asm lines). Inner text loop, trimmed, demangled (`_1`=1.0, `_3`=0.75, `_7`=0.15):

```asm
.LBB1116_33:                                   # <- loop head, once per posting
        leaq    8(%rsp), %rdi                  # &mut Result<u32> (32 B stack slot)
        leaq    96(%rsp), %rcx                 # &mut cursor — kept in memory
        callq   youtube_raw_search::format::read_varint        # doc-id delta
        movq    8(%rsp), %r13; movl 16(%rsp), %r12d; cmpq $-1, %r13   # unwrap Result by hand
        jne     .LBB1116_35
        addl    %r12d, %r14d                   # document += delta (saturating)
        ...                                    # read_varint #2 (frequency), #3 (first position)
        cvtsi2ss %rax, %xmm0; addss .LCPI1116_1(%rip), %xmm0; sqrtss %xmm0, %xmm0
        callq   youtube_raw_search::engine::storage::DocumentTable::document
        movl    32(%rsp), %eax                 # ...of which only .tokens is used
        divss   %xmm1, %xmm0                   # tokens / average_document_tokens
        ...                                    # x0.75, +0.25, x1.2, /, xidf, x4.0
        movss   .LCPI1116_7(%rip), %xmm0       # 0.15
        divss   4(%rsp), %xmm0                 # 0.15 / sqrt(p+1) — also when disabled
        addss   .LCPI1116_1(%rip), %xmm0
.LBB1116_63:
        movl %r14d, (%r15,%rbx); movss %xmm0, 4(%r15,%rbx); movb %cl, 8(%r15,%rbx)
        addq $12, %rbx                         # DocMatch push + capacity check -> grow_one
```

`read_varint` and `DocumentTable::document` are both their own symbols — and the latter checks
every field it returns:

```asm
        pushq %r14 / pushq %rbx / subq $104, %rsp     # frame for the sret Result
        movq (%rcx), %rax                             # reload cursor from memory
        movq %r8, %rdi; subq %rax, %rdi; jb .LBB1327_5  # end-cursor underflow
        cmpq %rdx, %r8; jbe .LBB1327_2                # second check: end vs slice length
        movsbl (%rsi,%rax), %r8d; testl %r8d, %r8d; js .LBB1327_8   # then write back the cursor
        --- DocumentTable::document ---
        cmpq %rax, 16(%rsi); jbe .LBB1149_4      # id >= documents (else format!+__rust_alloc)
        leaq (%rax,%rax,2), %rcx; addq $28, %rcx; cmpq %r14, %rcx; ja .LBB1149_2
        ...                                      # 4 per-field bounds checks
        callq youtube_raw_search::format::read_u32     # 5th field via a call
        movups 36(%rsp), %xmm0; movups %xmm0, 12(%rbx) # 24 B record through sret
```

Instruction accounting, counted from these listings (per posting):

| Segment | Current | Minimum |
| --- | ---: | ---: |
| 3x `read_varint` (5 setup + call + 2 reload + compare in caller, ~14 in callee) | ≈ 87 | 8 |
| `DocumentTable::document` (call, 5 checks, nested `read_u32`, 24 B sret) | ≈ 58 | 2 |
| float score | 16 | 12 |
| `DocMatch` store + capacity check | 7 | 4 |
| loop glue | ≈ 5 | 0 |
| **total** (≈ 6.5x the minimum) | **≈ 170** | **≈ 26** |

## 4. Measured effect of two small changes

Harness: temporary ignored test in `engine/tests.rs` that opens a locally built index, warms,
then times 15 iterations of `matches_for_term` (and of `search`) per term and prints medians
(`RAWPERF_INDEX=<dir> cargo test --release -q -p youtube_raw_search --lib -- --ignored
--nocapture arena_posting_decode_timing`). Corpus: 200 k synthetic documents / 2 k videos /
6.6 k terms built by the real `youtube-raw-search build` (58 MB index, 8.5 s). Machine: 16-core
laptop under load average 10–14, so **min-of-runs ns/posting**, 3 runs per variant, is reported.

| measurement | postings | current | + narrow accessor | + `#[inline(always)]` |
| --- | ---: | ---: | ---: | ---: |
| `the`, text only | 199,308 | 16.02 | 10.67 | **7.05** |
| `fable`, text only | 2,942 | 18.61 | 12.46 | **8.27** |
| `w0`, text only | 3,142 | 17.59 | 15.47 | **11.66** |
| `fable`, text+title (no title postings) | 2,942 | 18.40 | 14.41 | **8.62** |
| `the`, text+title | 199,623 | 23.63 | 18.48 | **14.39** |
| `blackhole`, text+title | 41 | 23.12 | 20.90 | **15.56** |

The clean baseline was re-measured on the reverted tree minutes later under the same load (`the`
text-only 18.27, `fable` 22.92, `the` text+title 25.25 ns/posting): it did not drift down, so the
delta is the change, not the machine.

**R2 — narrow accessor (recommended).** The loop needs only `tokens`, but `document` materialises
six fields, does five bounds checks and returns 24 bytes through sret (its fifth field even calls
`read_u32`). Added `DocumentTable::document_tokens` (one bounds check, one `u32` read) beside the
existing narrow precedent `search_document`
([storage.rs:275](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/storage.rs#L275)), used in `decode_text_postings`. Inlined it is one compare and one load:

```asm
        cmpq %rax, 496(%rdx); jbe .LBB1116_55      # single: id < documents
        leaq (%rax,%rax,2), %rcx; addq $44, %rcx
        movq 480(%rdx), %rcx; movl 40(%rcx,%rax), %eax   # tokens
```

Measured: −33% on the loop by itself (16.02 → 10.67 ns/posting), identical match counts and
scores. Scope: the unfiltered single-token fast path (`broad-single.bin`, 2.32 ms for `q=the`) never
enters this loop and is unchanged; the win lands on filtered, multi-term, and fuzzy paths.

**R1b — `#[inline(always)]` on `format::read_varint` (recommended).** Measured: a further
−31%…−34% on the same loop (10.67 → 7.05 for `the`; 12.46 → 8.27 for `fable`). Structural
confirmation in the regenerated `.s`: no outlined `read_varint` remains, the only surviving call
sites are cold `read_varint::{{closure}}` error builders, and `matches_for_entry` grows 914 →
1685 asm lines. With R2 the text-posting loop is ≈ 2.3x faster than the shipped binary.
Trade-off: forced inlining at 31 call sites bloats code; the five error paths are cold.

**R3 — `union_sorted` empty-side fast path (needs its own benchmark).** For a term absent from
titles the merge allocated a `P_text`-capacity buffer and moved every element to return the same
list; two `is_empty` returns are exactly equivalent. Measured only jointly (inert for `the`,
active for `fable`), so treat it as a hypothesis until a targeted benchmark exists.

**Verification.** `cargo test --release -p youtube_raw_search` → 162 passed, 1 ignored, CLI test
passed with all changes applied; `found` identical across builds (2000 / 17 / 1533 / 0).

**Left on the table (measured, not fixed).** (a) Title expansion + merge: for `the`, text-only
2.16 ms vs text+title 3.69 ms — **42% of the decode to add 315 net matches** out of 199,623.
(b) The exact `found` total forces the full walk: the documented filtered shape
(`q=I&youtuber=@t3dotgg`) took 7.83 ms of which 4.72 ms is this decode, and it can only early-exit
after materialising the list — `docs/performance-research.md` already flags "exact video totals" as
an open UX/cost trade. Arithmetic extrapolation (not measured end to end): the 2.3x loop should
remove ~2.7–2.9 ms from that shape; confirm on production-shaped data.

## 5. Alternatives considered and rejected

- **Plain `#[inline]` on `read_varint`** — rejected by measurement: the regenerated `.s` is
  structurally identical (31 call sites, 0 inlined ladders, outlined symbol present) and timing did
  not move. LLVM declines a decoder with five `format!` error paths even when available cross-CGU.
- **Thin LTO / profile changes** — [rust-performance-record.md](../../rust-performance-record.md) already
  rejected Thin LTO (3.2x slower clean build, larger artifacts); not re-tested here.
- **`-C target-cpu=native`** — not tested: it must be baked at image build time and Fly's fleet is
  not homogeneous, so a `native` image is not deployable evidence.
- **Threshold-aware decode (early exit inside the loop)** — rejected: it would change `found`,
  which is part of the response contract; that is a product decision (approximate totals).
- **Block-Max WAND pruning** — not applicable: the exact total and ordering forbid skipping
  postings on a score bound.
- **Position-multiplier lookup table (bit-exact, `p < 64`)** — not measured: `0.15/sqrt(p+1)`
  is ~2 float ops of ~16, small next to the removed calls. Hypothesis only.
- **Hoisting `1.0/average_document_tokens`** — rejected: replacing the per-posting `divss` with a
  multiply changes every score's rounding and would need a relevance-parity gate first.

## 6. Reproduction and revert record

The working tree was clean at the start (`git status --short`: empty). Temporary edits were
made under `crates/**` only and all reverted; the final tree contains exactly this document.

Patch 1 (harness, reverted): ignored test `arena_posting_decode_timing` at the top of
`crates/youtube_raw_search/src/engine/tests.rs` (80 added lines) reading `RAWPERF_INDEX` and
`RAWPERF_PHASE` (`decode` = time `matches_for_term` per term with `query_by=text,title`, then with
`query_by=text`; otherwise time `engine.search` for four shapes), 15 timed iterations each, printing
`ARENA ... ns_per_posting=...` through `std::hint::black_box`.

Patch 2 (the measured changes; added lines verbatim, unified-diff file headers elided; the
`#[inline]` on `read_varint` is the variant that was measured and rejected):

```diff
crates/youtube_raw_search/src/format.rs
+#[inline(always)]                       # R1b; plain #[inline] is a no-op (see §5)
 pub fn read_varint(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u32> {
crates/youtube_raw_search/src/engine/storage.rs  (new method, next to search_document)
+    /// Reads only the BM25 length field. The scoring loop needs `tokens` and
+    /// nothing else from the record, so it must not materialize five more fields
+    /// and their bounds checks per posting.
+    #[inline]
+    pub(super) fn document_tokens(&self, id: u32) -> Result<u32> {
+        if id as usize >= self.documents {
+            return Err(Error::Corrupt(format!("document {id} is out of bounds")));
+        }
+        read_u32(
+            self.file.bytes(),
+            METADATA_PREFIX_BYTES + id as usize * DOCUMENT_RECORD_BYTES + 16,
+        )
+    }
crates/youtube_raw_search/src/engine.rs  (in decode_text_postings)
-            let record = self.inner.metadata.document(document)?;
+            let document_tokens = self.inner.metadata.document_tokens(document)?;
             let score = bm25(
                 frequency,
-                record.tokens,
+                document_tokens,
crates/youtube_raw_search/src/engine/query_support.rs
 pub(super) fn union_sorted(left: Vec<DocMatch>, right: Vec<DocMatch>) -> Vec<DocMatch> {
+    if left.is_empty() {
+        return right;
+    }
+    if right.is_empty() {
+        return left;
+    }
     let mut output = Vec::with_capacity(left.len().saturating_add(right.len()));
```

Revert: `git checkout -- crates/youtube_raw_search/src/{format.rs,engine.rs}` and
`crates/youtube_raw_search/src/engine/{storage.rs,query_support.rs,tests.rs}`, re-verified with
`git status --short` plus a final `cargo test --release -p youtube_raw_search`. The measurement index
was built outside the repo (under `/tmp`), so no corpus or index artifact is committed. No production
endpoint, secret, `flyctl` command, or deploy was touched; `src/` was not opened.
