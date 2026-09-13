# Candidate D — Raw engine query path: theoretical minimum vs generated machine code

Measured on this worktree (rustc 1.100.0-nightly, x86-64 Linux host), default `release`
profile. Temp patches used for measurement were reverted; exact diffs in §4.

## 1. Grounding

**What a query is.** `youtube_raw_search` (`crates/youtube_raw_search`) serves
`/api/search` through `RawSearchEngine::search` ([engine.rs](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L619)).
Everything is mmap-backed borrowed slices ([format.rs](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/format.rs#L17-L45)); visitors decode varint streams in place. On-disk shape:

| File | Layout (per storage.rs / bigram.rs) |
| --- | --- |
| lexicon | 16 B header, (T+1)×u64 relative offsets, entries `[u16 len][term][32 B fields]` (34+len bytes) |
| postings | concatenated per-term streams `[varint doc-delta][varint frequency]`; title side stores video ids |
| positions | (2T+1)×u32 offsets, then cold varint delta stream per term/field; first position is one varint inside the hot sidecar |
| metadata | 24 B/doc record (block, offset, length, video, tokens, t_start_ms) + 16 B/block records; 64 docs per zstd block |
| bigrams | 24 B fixed entries + varint `(doc-delta, earliest_end_offset)` streams; fixed-size binary search |

Input scale: canary generation has **8.32 M transcript documents**
([raw-production-cost.md L115](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/docs/operations/raw-production-cost.md#L115)).
Required output: ≤ `per_page` video cards with transcript snippets + timestamps, JSON.

**Budgets (docs).** Backend first-result average **7–20 ms**, end-to-end p50 141–197 ms,
p95 253–780 ms — network/proxy dominates p95; backend CPU is the slice we can touch
([performance-research.md L35-44](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/docs/performance-research.md#L35-L44)).
Measured canary p95 on the deployed shape: shared-cpu-2x/2 GB **40.8 ms median / 254.3 ms
p95** vs performance-2x/4 GB 38.5/344.7 ms ([raw-production-cost.md L131-136](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/docs/operations/raw-production-cost.md#L131-L136)).
Idle process RSS 138.45 MiB (publication peaks 2.46 GiB) per staging-architecture.md.

**Platform.** [fly.raw-search.toml](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/fly.raw-search.toml#L33-L36): `ytsearch-raw-production`, sjc, shared CPU ×2, 2 GB. ELF x86-64 PIE (checked:
`file target/release/youtube-raw-search`). Allocator: none configured → glibc malloc.
Threads: axum on tokio multi-thread runtime. Profile: workspace `Cargo.toml` has **no
`[profile.release]`** → `opt-level=3, codegen-units=16, lto=off, panic=unwind` (verified by
`grep profile Cargo.toml` → no match). All assembly below comes from that profile.

Assembly commands (exact):

```sh
RUSTFLAGS="-C debuginfo=line-tables-only -C llvm-args=-x86-asm-syntax=intel" \
  cargo rustc --release -p youtube_raw_search --lib -- --emit=obj
O=$(find target/release/build/youtube_raw_search -name 'youtube_raw_search-*.o')
# symbols via: llvm-objdump --syms $O   (v0 mangling; demangled by hand below)
llvm-objdump -d --source --no-show-raw-insn --disassemble-symbols=$SYM $O
```

(`cargo asm`/`cargo-show-asm` not installed; binary is stripped, so the lib object is used.)

## 2. Mathematical minimum for one query

Canonical case: `q` = 2 terms, creator filter, per_page 10, video grouping. Two streams:
a streaming conjunction (for terms with many postings) or targeted decode (bounded).

Unavoidable work per query:

1. **Parse** `q`: O(len(q)) bytes.
2. **Lexicon binary search** ×2: `log₂(T)` probes; each probe *must* read 2×u64 offsets
   (16 B) + the candidate term bytes (~10 B) and compare once. On hit: one full entry
   decode (34+len bytes). Floor ≈ `log₂(T)` × (3 loads + 1 compare).
3. **Posting decode**: the whole posting range of each matched term is a varint delta
   stream with no skip index — every byte must be consumed once: ~2–3 B per posting,
   ≥1 compare/load per byte. `O(F)` varint decodes is the hard floor for a term with
   `F` postings; intersection merge adds `O(F₁+F₂)` integer work.
4. **Per candidate document**: one `tokens` u32 for BM25 (~6 fp ops) → floor is one
   bounds-checked 4-byte load into a 200 MB table (1 cache line).
5. **Output**: for ≤10 video groups: record read (24 B) + first-touch zstd decode of ≤10
   distinct 64-doc blocks (~45 KB each) + JSON build. Unavoidable on cold cache.

Allocation floor: result Vec + output JSON. That is it.

Incidental work the code adds on top (all shown in §3): UTF-8 validation per lexicon
probe, `TermEntry` construction per probe, out-of-line reader calls with 88–104 B stack
frames per posting, 6-field metadata read where 1 field is used, fresh `Vec` per zstd
block decode, `HashSet` per snippet.

## 3. What the machine code actually does

**D1 — readers are real function calls in hot loops.** The `decode_text_postings` loop
inside `matches_for_entry` ([engine.rs L1316-1332](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1316-L1332))
compiles to (loop body ≈ 0x3e0–0x5a3, 451 bytes; trimmed):

```asm
; matches_for_entry — per-posting iteration:
0x409/0x448/0x48e: callq read_varint        ; 3 out-of-line varint decodes
0x4d2:             callq DocumentTable::document ; 6-field record, out of line
; movss/divss chain = bm25 (inline); cmpq capacity/jne grow_one = Vec push (amortized)
```

Verified by relocating each call target: the three loop calls resolve to
`format11read_varint`, the fourth to `DocumentTable8document`. Each out-of-line
`read_varint` call carries `push r14; push rbx; sub rsp,0x68; …; add rsp,0x68; pop…` — a
104-byte stack frame for a ~1-byte decode. Same pattern in `PositionCursor::read_into`,
`PositionedPostingCursor::next_at_least`, and the bigram decode loop (2 varint calls/posting).

The reader bodies themselves are fine — `read_u64`'s hot path folds `from_le_bytes` into
one unaligned load (`cmpq $-0x9,%rcx` overflow check → `cmpq %rdx,%rax` span check →
`movq (%rsi,%rcx),%rax`): the cost is the call + `Result` plumbing, not the load. None of
`read_u16/u32/u64`, `read_varint`, `DocumentTable::document` carries `#[inline]`, and
adding it did not change the count of remaining out-of-line call relocations (55 with and
without — measured, §4): the residual calls are size-heuristic-bound, not CGU-bound.

**D2 — metadata record: 6 fields decoded, 1 used.** `decode_text_postings` calls
`metadata.document(document)` ([engine.rs L1323](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L1323))
and uses only `record.tokens`. `DocumentTable::document`
([storage.rs L260-273](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/storage.rs#L260-L273))
disassembles to 4 merged span compares + 4 direct loads + 2 remaining out-of-line
`read_u32` calls + an 88-byte frame (6 pushes) — per posting. Same one-used-field pattern:
`decode_targeted_text` (conjunction.rs L594) uses only tokens; `reaches_*` threshold loops
(single_token.rs L177/L195), `position_targets` (engine.rs L997), and
`retain_position_phrase_matches` (engine.rs L861) use only `video`.

**D2' — the streaming path already reads lean.** `visit_plan_conjunction` calls
`search_document` (3 fields, all used for grouping) — measured 5.4 ns vs 15 ns. Not a
target.

**D3 — lexicon probe overhead.** `Lexicon::lookup`
([storage.rs L37-50](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/storage.rs#L37-L50))
calls `entry_at` per binary-search probe; `entry_at` disassembles to (trimmed,
`…engine::storage::Lexicon::entry_at`): 2×u64 offset reads → slice get → u16 length →
`cmpq fields+32, entry.len()` → **out-of-line `callq *(%rip)` to `std::str::from_utf8`**
→ 6 field loads → `TermEntry` construction. Per probe: ~10 instructions of real work plus
UTF-8 validation of every candidate term and 6 reads that the comparison never uses.

**D4 — `read_varint`'s own hot path is sane**: one `movsbl (%rsi,%rax)` load + test +
return for 1-byte varints (after cursor `get(cursor..end)` setup compares). The overhead
is the frame + the extra slice-check per call, not per-byte bounds checks. Allocation
patterns: `decode_text_postings` pre-reserves exactly (`Vec::with_capacity` — good).
Incidental: `union_sorted` builds a third Vec per term ([query_support.rs L376-414](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/query_support.rs#L376-L414));
`document_value` does `zstd::stream::decode_all` → fresh Vec per block miss
([engine.rs L2078](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine.rs#L2078));
`highlighted_snippet` builds a `HashSet` + per-char `String` token per hit
([query_support.rs L1080-1128](worktree://bf320e23-94fa-426e-8da1-a17c0dc9edbf/crates/youtube_raw_search/src/engine/query_support.rs#L1080-L1128))
— bounded by per_page, low priority. No memchr dependency; the only byte-scan loops are in
build/output paths, not the query path (not a bottleneck — hypothesis).

## 4. Proposed optimizations (measured where stated)

Harness: temporary `#[cfg(test)]` benches appended to `storage.rs` / `format.rs`
(reverted; exact harness diffs summarized below). Release profile, `--test-threads=1`,
7 runs, warm medians quoted.

### P1 — tokens-only / video-only metadata accessors (measured via proxy)

Add `document_tokens(id)` and `document_video(id)` to `DocumentTable` (one `get(offset..+4)`
+ one load, same shape as `search_document`) and switch the one-used-field call sites:
`decode_text_postings`, `decode_targeted_text`, the `reaches_*`/threshold and `video`-only
loops (D2 list). Behavior-preserving: same bounds check, same error text; full record still
read where grouping/formatting needs it.

Evidence: same-binary bench, 500 k random reads into a 500 k-doc synthetic metadata table:

```text
TMP document()        5.7–10.4 ms / 500k   (warm median 7.3 ms) ≈ 15 ns/read
TMP search_document() 1.8–3.6  ms / 500k   (warm median 2.7 ms) ≈ 5.4 ns/read
```

A one-field read is ≤ the 3-field shape, so ≈ −10 ns per posting/candidate on the decode
and threshold paths. For a broad query decoding ~10⁶ postings that is ~10 ms off the loop
vs today's ~15 ns/posting; for typical filtered queries (10³–10⁵ postings) it is tens of
µs. Honest: sub-ms to low-ms, concentrated on broad plans.

### P2 — bytes-only lexicon binary-search probe (measured)

Compare raw term bytes per probe; run the full `entry_at` (validate UTF-8, read fields)
only once on a hit. Exact patch that was applied and reverted (adds `probe_term`;
`lookup` gains 2 lines + the hit path):

```diff
--- a/crates/youtube_raw_search/src/engine/storage.rs
+++ b/crates/youtube_raw_search/src/engine/storage.rs
@@ pub(super) fn lookup
-            let (candidate, entry) = self.entry_at(middle)?;
-            match candidate.as_bytes().cmp(term.as_bytes()) {
+            let candidate = self.probe_term(middle)?;
+            match candidate.cmp(term.as_bytes()) {
                 Ordering::Less => low = middle + 1,
                 Ordering::Greater => high = middle,
-                Ordering::Equal => return Ok(Some(entry)),
+                Ordering::Equal => return self.entry_at(middle).map(|(_, entry)| Some(entry)),
@@ + new private fn
+    fn probe_term(&self, index: usize) -> Result<&[u8]> {
+        let relative = read_u64(self.file.bytes(), HEADER_BYTES + index * 8)? as usize;
+        let next = read_u64(self.file.bytes(), HEADER_BYTES + (index + 1) * 8)? as usize;
+        let start = self.entries_start.saturating_add(relative);
+        let end = self.entries_start.saturating_add(next);
+        let entry = self.file.bytes().get(start..end).ok_or_else(|| {
+            Error::Corrupt("lexicon entry extends beyond file".to_string())
+        })?;
+        let term_length = read_u16(entry, 0)? as usize;
+        let fields = 2 + term_length;
+        if fields + 32 != entry.len() {
+            return Err(Error::Corrupt("lexicon entry has an invalid length".to_string()));
+        }
+        Ok(&entry[2..fields])
+    }
```

Corruption checks stay per probe (length invariant); UTF-8 validation moves to the hit.
Measured on a 200 k-term synthetic lexicon, 200 k random lookups, one binary, `const`
toggle between the two implementations:

```text
TMP lookup (today):        108.5–121.5 ms (median 112.6) ≈ 560 ns/lookup
TMP lookup_probe:           73.3–85.4 ms (median 78.2)  ≈ 390 ns/lookup
```

−31–38 % per lookup (~19 probes ⇒ ~9–13 ns/probe). End-to-end effect is µs-scale per query
(lookup runs O(tokens × alternatives) times, not per posting) — cheap and safe, not the
latency lever. Bench harness: a `mod perf_tmp` appended to storage.rs — synthesize a
lexicon with `write_header`-compatible bytes, mmap via `Lexicon::open`, time `lookup` of
LCG-random hits; both implementations compiled into one binary behind a `const` toggle.

### P3 — inline the varint/read helpers in the hot loops (repro-measured)

`#[inline]` on `read_u16/u32/u64/read_varint` is a one-line-per-fn patch (exact diff of the
experiment: `#[inline]` added above the four `pub fn`s in format.rs; reverted). It lowered
out-of-line reader calls 162→55 call relocations but did **not** change the count at the
remaining hot sites, and the lexicon bench moved within noise. Standalone repro (2 M
varints, 87 % 1-byte / 13 % 2-byte, exact harness in the reverted format.rs `mod perf_tmp`):

```text
TMP read_varint (out-of-line): 4.76–6.86 ms / 2M (median 4.81) ≈ 2.4 ns/varint
TMP inline-always minimal copy: 2.33–2.63 ms / 2M (median 2.4) ≈ 1.2 ns/varint
```

The repro's inline copy also drops the per-call `get(cursor..end)` slice setup, so it is an
upper bound: ~1–2 ns saved per varint (hypothesis for verbatim-inline); the 104-byte frame
per call (D1) is the mechanism. Expected: ~2–3 ns/posting on streaming/bigram decode loops
— few-% of broad-query backend time. Verify by re-running the repro with verbatim
`#[inline(always)]` on the real body and inspecting `matches_for_entry`'s loop.

### P4 — (sketch, unmeasured) reuse zstd scratch buffer

`zstd::bulk::decompress_to_buffer` into a reused `Vec<u8>` in `document_value` (≤10 block
misses per cold query). Expected: one 45 KB alloc+copy saved per miss — µs-scale.
Hypothesis, not measured here.

## 5. Alternatives considered and rejected / unverified

- **`lto = "thin"` in `[profile.release]`** — measured: baseline `lookup` 112.5–116 ms vs
  default 108.5–121.5 ms (no improvement), +63 s full rebuild. Rejected.
- **u32 lexicon offsets / format changes** — halves offset-table reads but requires a
  reindex and a schema bump; violates "small, behavior-preserving PR". Rejected.
- **memchr for scans** — no hot forward scan in the query path uses a byte search loop
  today; token scans live in output formatting. Rejected as speculation.
- **LTO/codegen-units as the fix for D1** — the residual calls are size-heuristic-bound
  (55 relocs unchanged with `#[inline]`), so forcing inlining via attributes is the
  surgical fix, not a profile change.

**Unverified / hypotheses:** exact end-to-end latency effect of P1–P3 on the live index
(no production calls in this exercise); the instruction-level split of P3's 2.4→1.2 ns
delta between call overhead and dropped bounds checks; bigram-side benefits of P3 (same
loop shape, not separately benched).