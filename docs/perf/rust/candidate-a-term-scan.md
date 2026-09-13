# Arena candidate A — the raw term scan

Audited hot path: **scanning one query term's posting list to produce scored
document matches** in `crates/youtube_raw_search` — the batch decoder
(`matches_for_entry`/`decode_text_postings`) and its cursor twin
(`conjunction::TextAlternativeCursor`/`StreamingTermPlan`). Grouping, response
JSON and the public API are outside this audit.

## 1. Grounding

* Input `SearchParams` (`/api/search`: `q`, `filter_by`, `group_by`, `per_page`)
  → `parse_query` → mmap'd `Lexicon::lookup`; output of the audited span is
  `Vec<DocMatch>` (12 B each). It must score **every** document containing the
  term: per-video relevance comes from the full match set before
  `sort_group_window` partial-selects, so "return 10 hits" does not shrink it.
* Corpus: 8,322,725 documents / 32,822 videos = **253 chunks/video**
  (`docs/operations/raw-production-cutover.md`, promotion gates). Machine:
  `shared-cpu-2x`, 2 GiB + 512 MiB swap, always on, 17% of a 20 GB volume
  (`docs/operations/raw-production-cost.md`). x86-64 baseline ISA (`-C
  target-cpu` unset), glibc malloc (`debian:bookworm-slim`), tokio runtime.
  Searches are **serialized** — `MAX_CONCURRENT_BACKEND_SEARCHES = 1`
  (`crates/youtube_search_service/src/lib.rs`) on a blocking thread — so one
  request's CPU time is one request's throughput. Gates: 200 ms search ceiling,
  5 s request timeout, 812 MiB live cgroup memory.
* Profile: `crates/youtube_raw_search/Cargo.toml` has none, so the workspace
  release default applies — `-C opt-level=3`, codegen-units 16, no LTO,
  `panic=unwind` — plus `strip = true` from `.cargo/config.toml`. Symbols here
  came from the `CARGO_PROFILE_RELEASE_STRIP=false` env override.
* Fixture (synthetic; no production data, no network): 1,000,000 documents /
  4,000 videos (250 chunks/video) / 16 tokens per chunk / 30,000-word cubic-skew
  vocabulary, built by the crate's own `build_index` in ~24 s — production-shaped
  at 1/8 size: **24.25 B metadata per document** vs ~202 MB in production, so
  every cache figure below understates production.
* Four shapes implement "scan one term", all four visible in the profile:
  **(A)** batch `decode_text_postings` for filtered queries, non-broad
  conjunctions, fuzzy/prefix, uncovered single tokens; **(B)** streaming cursors
  when every term is ≥ `STREAMING_CONJUNCTION_MIN_DOCUMENTS` (100,000 docs);
  **(C)** target-driven `matches_for_plan_targets` past the 128-target threshold;
  **(D)** `broad_single` decode for unfiltered single-token queries — profile
  shares, in order: 277 instr/posting (A), 37% `take_current` + 15.5% heap `pop`
  (B), `matches_for_plan` 1.9% (C), `broad_single::decode_answer` 6.8% (D).

Measured whole requests (`group_by=video_id`, `per_page=10`, best of 3×20,
`taskset -c 2`, release):

| query | ms | instructions |
| --- | ---: | ---: |
| single token, 89,964-posting term | 2.31 | — |
| single token, 577-posting term | 0.54 | — |
| two co-occurring terms (89,964 + 577 postings) | 4.18 | 43.8 M |
| three terms, empty intersection | 105.4 | ≈1.5 G |
| same, with `filter_by=youtuber_handle:=…` | 125.1 | (avg of both) |

A full batch decode of the 89,964-posting term alone costs 4.43 ms, so the 2.31 ms
single-token request cannot be scanning that list — and its profile is all
`broad_single::decode_answer`/`read_byte`, confirming shape D. The 3-term rows are
*no-match* queries — the exact conjunction visited **0** documents (counted via
`visit_plan_conjunction`) — so their 105–125 ms is the token-drop/typo fallback
ladder (`MAX_TOKEN_DROP_CANDIDATES = 20` per direction).

## 2. The mathematical minimum

Per posting, the required output forces:

| must happen | cost |
| --- | --- |
| decode doc-delta, frequency and `first_position` varints | 3 loads (3.00 B/posting measured) |
| read the BM25 length field `tokens(doc)` | 4 B from *some* array |
| score and write the `DocMatch` | 2 `divss`, 1 `sqrtss`, ~9 mul/add, 9 B store |

Floor ≈ **4 sequential loads, 3 stores, ~25 instructions per posting**, plus one
comparison per merge step. Best inner loop I could write (`min_columnar`): 9.7 ns,
158 instructions, 0.02 LLC misses per posting — so per query the floor is `Σ Fᵢ`
postings read once plus a k-way merge, no per-posting allocation. Two-term row:
0.88 ms / 14.3 M instructions floor vs 4.18 ms / 43.8 M measured (**≈4.7×**);
three-term row: 0.12 ms / 2.0 M vs 105–125 ms / ≈1.5 G (**≈900×**).

## 3. What the machine code actually does

```bash
CARGO_PROFILE_RELEASE_STRIP=false cargo test --release -p youtube_raw_search --lib --no-run
objdump -d --demangle --no-show-raw-insn <test-binary> > /tmp/objdump.txt   # perf stat / perf record for counters
```

`decode_text_postings` loop (0x1c5431–0x1c55c1, trimmed, demangled):

```
1c5442: call 281e70 <youtube_raw_search::format::read_varint>   ; doc delta
1c5450: cmp  $0xffffffffffffffff,%rcx ; 1c5454: jne <+0x4a3>    ; Result discriminant + cold path
1c5477: call 281e70 <…read_varint>                              ; frequency
1c54a9: call 281e70 <…read_varint>                              ; first position
1c54e9: sqrtss %xmm0,%xmm0                                      ; position multiplier
1c550c: call 1cf840 <DocumentTable>::document                   ; 24-byte record -> %rbp
1c5528: movss 0x15c(%rax),%xmm1                                 ; reload manifest per posting
1c554c: divss %xmm1,%xmm0 ; 1c557e: divss %xmm0,%xmm1 ; 1c55a6: cmpb $0x0,0x34(%rsp)
1c5400: mov %r13d,(%rax,%rcx,4) ; movss %xmm0,0x4(…) ; mov %r12b,0x8(…)  ; 3 stores
1c55c1: jne 1c5400                        ; bm25, prioritize_position, push cap/grow_one
```

`StreamingTermPlan::take_current` (0x2214c0) is **507 disassembly lines** for one
document's contribution: `tzcnt` over the active-alternative bitmask, then per
alternative a bounds check (`cmp 0x10(%r13),%rdi ; jae <error>`) into a 96-byte
cursor array (`lea (%rdi,%rdi,2) ; shl $0x5`), a reload of the engine pointer and
`average_document_tokens`, and the loop-invariant `average_tokens > 0.0` guard as
a branchless `cmpltss/andps/andnps/orps` select — while
`TextAlternativeCursor::advance_to` and `validate_complete` stay out of line.

## 4. Deviations from the minimum

Per posting on the 89,964-posting term, same fixture/binary, `perf stat` minus
the open-only baseline; ns = best of 3 repeats:

| build | instr/posting | LLC misses/posting | ns/posting |
| --- | ---: | ---: | ---: |
| shipped `decode_text_postings` | 277.3 | 2.71 | 49.2 |
| + `DocumentTable::tokens` patch (D2) | 170.4 | 2.04 | 42.7 |
| harness: 1-load accessor, same layout | 162.3 | 2.09 | 43.0 |
| harness: columnar `u32` tokens | 158.5 | 0.02 | 9.7 |
| harness: columnar, no `DocMatch` store | 149.5 | — | 8.2 |

Whole `matches_for_entry` (text + title + merge; units = returned matches): 246.0
instr/unit and 41.0 ns/unit shipped → 188.8 (−23%) and 36.3 (−11%) patched; the
12,463- and 577-posting terms went 8.94 → 7.58 and 20.46 → 14.75 ns/unit. The
loop is **stall-bound**: −39% instructions bought 11–28% wall clock.

* **D1 — three out-of-line varint calls per posting**, each returning
  `Result<u32, Error>` through a stack slot with a discriminant branch
  (`read_varint` is a separate non-`#[inline]` fn). Unavoidable: reading 3
  varints. Incidental: 3 × (call + out-param + discriminant + epilogue) plus
  reloading every clobbered `self`/`manifest` field (277 vs 162 instructions).
* **D2 — reading a 24-byte record to get 4 bytes.** `decode_text_postings` wants
  only `record.tokens`, but `DocumentTable::document` (0x1cf840) is **132
  instructions** — 4 range comparisons, 6 field loads, 2 out-of-line `read_u32`
  calls, 6 spill/reload pairs, a 24-byte out-param write and a 24-byte random
  touch per posting. `search_document` (12 bytes, one check) already exists and is
  used by the streaming path; `format::read_varint` adds ~20 instructions per
  one-byte varint from out of line.
* **D3 — per-posting reload of loop-invariant state**: `average_document_tokens`
  re-loaded after every call, the `> 0.0` guard re-evaluated per posting,
  `prioritize_position` re-tested per posting.
* **D4 — `DocMatch` is 12 B for 9 B of payload** (see the
  `size_of::<DocMatch>() <= 12` assert), written as three stores through a
  `lea (%rbx,%rbx,2)` ×4-stride address behind a `Vec::push` capacity branch with
  a `grow_one` call site in the loop: 3 B × F = 270 KB per 90k-posting scan.
* **D5 — `union_sorted(text, title)` copies even when a side is empty**:
  `with_capacity(N+M)` + `Peekable` merge. On the 90k list — plain clone 0.31
  ns/posting vs `union_sorted(clone, Vec::new())` 1.38 ns/posting, i.e. **+1.07
  ns/posting (~96 µs/query)** for zero merging, paid whenever a term has
  `title_frequency == 0` — true for 6 of the 11 fixture terms probed.
* **D6 — title matching is materialized at document granularity through a binary
  heap.** `TitlePlanCursor::new` builds one cursor per title-matching video into
  `BinaryHeap<Reverse<(u32, usize)>>`, and `advance_to` pops and re-pushes per
  (video, document) step. The 89,964-posting term matches 451 titles × 250 chunks
  = 112,750 documents — more heap elements than postings — and production
  multiplies by 253 chunks/video. Minimum: one predicate per visited document,
  since the driver already loads `metadata.search_document(target)?.video`, so a
  dense `video → title contribution` slot table (see `empty_group_slots`) answers
  it in one load. Profile: heap `pop` 15.5% + `advance_to` 4.4% + much of
  `take_current` 37.4%.
* **D7/D8 — per-document driver and cursor overhead.** `visit_plan_conjunction`
  makes three passes over `streams` per output document (max-of-current,
  advance-all, verify-all; the last two are the same loop) and
  `refresh_current_document` re-scans all 96-byte cursors twice per document.

## 5. Proposed optimizations (behavior-preserving)

* **P1 — add `DocumentTable::tokens(id)` and use it in `decode_text_postings`.**
  10 lines, format unchanged; measured −23% instructions and −11…−28% wall on
  `matches_for_entry` at three frequencies. The same idea lets `format_matches`,
  `format_ungrouped` and `retain_video_filter` use the existing
  `search_document` accessor — they read `video`/`t_start_ms` today through the
  full record (hypothesis; same class as the measured patch).
* **P2 — early-return `union_sorted` when either side is empty.** 4 lines;
  removes the +1.07 ns/posting (≈96 µs) measured in D5.
* **P3 — columnar `tokens` sidecar (new generation file, schema 9).** Builder
  writes `u32` per document, the accessor reads it. Measured: 49.2 → 9.7
  ns/posting, 277 → 158 instructions, 2.71 → 0.02 LLC misses/posting — the largest
  lever here, but a format change: needs a full publication plus the 200 ms
  ceiling, `measure-broad-single` and relevance fixtures.
* **P4 — replace the title heap with a video-keyed contribution table** (D6),
  keeping title-as-producer only for documents reachable solely through a title
  alternative; cover with `streaming_group_accumulation_matches_materialized_response`
  and `common_word_streaming_reducer_matches_independent_materialized_json`.
  Should remove the 15.5% heap and part of the 37.4% `take_current` — hypothesis,
  **needs the benchmark after the change**.
* **P5 — fuse the driver's advance/verify passes** (D7/D8). Small, low risk, same
  tests; profile suggests ≤7% of a no-match query.
* **Negative result: `-C target-cpu=native` is not worth it** — instructions
  moved −0.1%, wall clock not at all on the same 60-query workload; the loop is
  stall- and call-bound.
* **Not proposed:** pruning (WAND/block-max) belongs to the engine
  (`docs/performance-research.md`), and the waste measured here is layout and
  per-call overhead, not comparison count.

## 6. Temporary edits, recorded (all reverted; the tree was clean before this doc)

The D2 patch, verbatim (`cp` from `/tmp/hotpath-out/{storage.rs,engine.rs}.orig`):

```diff
--- a/crates/youtube_raw_search/src/engine/storage.rs   @@ -272,6 +272,14 @@
+    /// TEMP PATCH: tokens-only accessor (arena candidate A, reverted before handoff).
+    pub(super) fn tokens(&self, id: u32) -> Result<u32> {
+        if id as usize >= self.documents {
+            return Err(Error::Corrupt(format!("document {id} is out of bounds")));
+        }
+        read_u32(self.file.bytes(), METADATA_PREFIX_BYTES + id as usize * DOCUMENT_RECORD_BYTES + 16)
+    }
--- a/crates/youtube_raw_search/src/engine.rs          @@ -1322,10 +1322,10 @@
-            let record = self.inner.metadata.document(document)?;
+            let document_tokens = self.inner.metadata.tokens(document)?;
             let score = bm25(
                 frequency,
-                record.tokens,
+                document_tokens,
                 self.inner.manifest.average_document_tokens,
             ) * idf
```

Second edit, also reverted: `engine.rs` gained `#[cfg(test)] mod hotpath_bench;`
after `mod conjunction;`, and the new file `engine/hotpath_bench.rs` (deleted)
held the harness. Its load-bearing parts:

```rust
// corpus: 1_000_000 docs / 4_000 videos / 16 text tokens per doc / 30_000 words
fn word(&mut self) -> u32 { let r = self.unit(); (30_000.0 * r * r * r) as u32 }  // cubic skew
while cursor < bytes.len() {                       // min_same_layout / min_columnar
    document = document.saturating_add(read_varint(bytes, &mut cursor, bytes.len())?);
    let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
    let first_position = read_varint(positions, &mut position_cursor, positions.len())?;
    let tokens = /* columnar */ column[document as usize]
               /* same layout */ read_u32(metadata, METADATA_PREFIX_BYTES + document as usize * 24 + 16)?;
    let score = bm25(frequency, tokens, average_tokens) * idf * TEXT_QUERY_WEIGHT
        * token_position_multiplier(first_position, true);
    matches.push(DocMatch::with_exact_mask(document, score, true, 1)); // no_store omits this
}
// variants (all #[ignore], run as `taskset -c 2 <binary> --ignored --nocapture
// --test-threads=1 engine::hotpath_bench::variant_<name>`): open_only, text_only,
// title_only, current, min_same_layout, min_columnar, min_no_store,
// union_empty_title, whole_query, filtered_query, cooccurring_query, conjunction_plan
```

## 7. Rationale and what is unverified

Rejected alternatives: **unsafe unchecked reads** — the crate has no `unsafe`
(only `MmapOptions`), `docs/rust-performance-record.md` treats that as a property
to keep, and every win above is reachable safely. **Thin LTO / profile changes** —
already measured worse there. **Protobuf/JSON changes** — outside this path and
excluded by the project's own rule. **Unifying the batch decoder with the cursors**
— different selectivities, and the batch loop is the cheap per posting (162–277
instructions vs the cursor's per-document call chain). **Pruning/ranking** —
semantics, not the measured problem.

Unverifiable here, therefore hypotheses: production term frequencies and the live
query mix (`youtube-raw-search measure-broad-single <index> <terms>` would settle
it); whether production corpus order makes `VideoMeta.documents` a contiguous
doc-id range, which would simplify P4; the live no-match/co-occurring split (Axiom
`search_query.duration_ms` bands); allocator behaviour on Fly (glibc both sides;
allocation is not a visible profile share); and every absolute latency — an Arrow
Lake P-core here, a shared 2-vCPU Fly vCPU there: read ratios and instruction
counts, not nanoseconds.
