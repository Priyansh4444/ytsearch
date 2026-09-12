# Candidate E - single-token postings decode: theoretical minimum vs
# generated machine code

Scope: crates/youtube_raw_search postings decode (the Raw engine's
single-token hot path). Sections 1-3 ground the system, derive the
theoretical minimum work, and compare it with the release assembly.
Section 4 reports three measured-but-rejected micro-variants.
Sections 5-8 give the proposals, rejected alternatives, unverified
items, and the exact reverted diffs.

## 1. Grounding

Hot path: single-token search in the Raw engine
(crates/youtube_raw_search/src/engine.rs). RawSearchEngine::search
dispatches a one-term query to search_single_token (engine.rs:654)
unless the broad-single streaming index answers it first (engine.rs
648-653; broad queries are the documented fast path,
docs/experiments/search/raw-inverted-index.md and
docs/architecture/raw-broad-single-index.md). The decode core is
matches_for_term -> matches_for_entry (engine.rs:1265-1299),
decode_text_postings (engine.rs:1301-1341), decode_title_postings
(engine.rs:1343-1396), union_sorted (engine/query_support.rs:376), the
reaches_result_threshold gates (engine/single_token.rs:167), and
format_matches (engine.rs:1755).

Input shape (docs/experiments/search/raw-inverted-index.md): the mmap'd
postings.bin holds per-term delta-varint id + frequency pairs;
positions.bin holds first-position delta varints; documents are
24-byte fixed records. One text posting per 30-second caption chunk:
frequency is usually 1 (a token appears once per segment), so an id
varint is 1-4 bytes and frequency is 1 byte.

Required output: JSON with found, total_moments, and up to 10 grouped
video cards (5 hits each) with text/title highlight snippets.

Budgets: backend search is 7-20 ms at p50 on live corpora
(docs/performance-research.md, measured-backend table) versus public
p95 of 253-780 ms, so backend CPU is not the tail driver. Axiom splits
backend_search_ms / result_processing_ms
(docs/staging-observability.md). Full-corpus publication peaks at
2.46 GiB RSS on this machine class (docs/staging-architecture.md).

Platform (fly.raw-search.toml): ytsearch-raw-production, SJC,
shared-cpu-2x (2 shared x86-64 vCPUs, SMT sibling shared with
neighbors), 2048 MiB, always started; index mmap'd from a 20 GB volume;
axum plus #[tokio::main] multi-thread (worker threads = vCPUs = 2); one
ReloadableEngine behind a RwLock of an Arc of RawSearchEngine
(reloadable.rs:9). Allocator: default glibc malloc (no mimalloc or
jemalloc in crates/youtube_raw_search/Cargo.toml).

Build profile (verified by reading the files): the workspace Cargo.toml
defines no [profile] section at all, so release uses defaults
(opt-level 3, lto false, codegen-units 16, panic unwind);
.cargo/config.toml (commit ac58b09) adds only strip = true. Production
binaries carry no symbols, so only repo-side assembly can be inspected.

## 2. Theoretical minimum per single-token query

For a term with a text posting list of P entries, title list T entries,
response of H hits in G video groups (G at most 10, H at most 50).

Unavoidable:

1. Lexicon binary search: O(log L) mmap probes over L terms (storage.rs
   binary search through lexicon.bin).
2. Bytes touched: P id+frequency varint pairs (~1.5 B average) + P
   first-position varints when prioritize_position is on (default true;
   prioritize_token_position=false is rejected, query_support.rs:795) +
   P x 24 B document records + T x 24 B video groups. That is the
   posting-I/O floor.
3. Decodes: 2P varints (id, frequency) + P position varints, each at
   least 1 load + 1 compare, so at least 3P loads and 3P compares.
4. Scores: P BM25 evaluations (>= 1 divide + 1 sqrt for the position
   multiplier, ~6 mul/sub), T title evaluations (the ln(frequency)
   form, one multiply per pushed document).
5. Ordering: 0 comparisons. Text postings arrive ascending by
   construction (delta encoding), title postings likewise (section 3).
6. Grouping: touch G slots for the top groups; the dense slot table
   (engine.rs:1768) needs 0 hashes for group membership plus a bounded
   top-G selection (already select_nth_unstable_by + bounded sort,
   engine.rs:1893-1895).
7. Output: H zstd block decodes (deduped per block via BlockCache) + H
   JSON parses + H snippet builds.

Incidental (no output depends on it): per-varint bounds re-checks,
out-of-line call overhead, per-stage re-parsing of the request filter,
String/Value churn in highlighting, and any sorting of data that is
already ascending.

Lower bound for the decode loop: per posting one load + one compare +
one add + one cursor compare + one 12 B store, plus one 24 B record
load + BM25 FLOPs + one 12 B push. At P = 10^4 postings that is tens of
microseconds of pure CPU; anything above it is incidental.

## 3. Machine code

Build: cargo build --release -p youtube_raw_search. The crate compiles
to 16 codegen units (cgu.00..cgu.15 in
target/release/libyoutube_raw_search.rlib). Extraction command (exact):

```sh
cd $(mktemp -d) && ar x <worktree>/target/release/libyoutube_raw_search.rlib
objdump -dr --demangle youtube_raw_search-*cgu.11.rcgu.o > cgu11.asm
objdump -dr --demangle youtube_raw_search-*cgu.01.rcgu.o > cgu01.asm
objdump -dr --demangle youtube_raw_search-*cgu.13.rcgu.o > cgu13.asm
```

format::read_varint is never fully inlined into any decode loop. It
exists as one out-of-line T symbol in cgu.11 (331 instruction entries);
the per-posting loop in cgu.01 matches_for_entry calls it (cgu01.asm,
demangled):

```asm
; RawSearchEngine::matches_for_entry
 392:  lea 0x78(%rsp),%rcx
 397:  mov %r15,%r8
 39a:  call 39f <...+0x39f>
;      R_X86_64_PLT32 format::read_varint-0x4
```

The callee fast path (cgu11.asm, format::read_varint, trimmed):

```asm
  0:  push %r14
  2:  push %rbx
  3:  sub $0x68,%rsp          ; 104-byte frame
  7:  mov %rdi,%rbx
  a:  mov (%rcx),%rax         ; load cursor
  d:  mov %r8,%rdi
 10:  sub %rax,%rdi           ; end - cursor
 13:  jb 1a                   ; cursor > end
 15:  cmp %rdx,%r8
 18:  jbe 77                  ; end > len
;      --- 1-byte varint fast path ---
 d6:  movsbl (%rsi,%rax,1),%r8d
 db:  test %r8d,%r8d
 de:  js f9                   ; continuation bit
 e0:  inc %rax
 e3:  mov %rax,(%rcx)         ; store cursor
 e6:  mov %r8d,0x8(%rbx)      ; sret payload
 ea:  movq $-1,(%rbx)         ; sret Ok tag
 f1:  add $0x68,%rsp
 f5:  pop %rbx
 f6:  pop %r14
 f8:  ret
```

Concrete deviations from the minimum, each with the artifact that shows
it:

- call/ret + 104-byte frame + 2 register spills per varint. 13 call
  sites in cgu.01 alone; readelf -r counts 20 read_varint relocations
  there. Minimum: one in-register load + compare.
- Two redundant checks per varint: end <= len (cmp %rdx,%r8) and
  cursor <= end re-evaluated per call, although every production call
  site passes end = bytes.len() (all 10 sites verified by grep:
  engine.rs 1317/1318/1320/1367/1368/1370, positions.rs 67/88,
  storage.rs 188/199). One is loop-invariant, the other duplicates the
  loop cursor condition.
- Result u32 via sret: per call two stores (mov %r8d,0x8(%rbx) plus
  movq -1,(%rbx)) where the minimum is one add + one store. The cold
  Error::Corrupt(..to_string()) arms (calls to __rust_alloc at offsets
  +0x1a/+0x7f) also keep the loop from being a single no-alloc fast
  path.
- Per-iteration grow_one call: matches.push in decode_text_postings
  emits call *0x0(%rip) # RawVec<DocMatch>::grow_one (cgu.01,
  matches_for_entry+0x518) although the vector was pre-sized with
  Vec::with_capacity(entry.text_frequency) (engine.rs:1312): an
  indirect call per posting where the minimum is compare + store.
- Per-posting DocumentTable::document call (cgu.01, +0x45a, PLT32
  DocumentTable::document): a 24 B fixed-stride read behind a bounds
  check + Result (storage.rs:260-273), executed 2-3x per candidate per
  query (decode, threshold gate, format).
- decode_title_postings sorts an already-ascending list (engine.rs:
  1394, sort_unstable_by_key on candidate.document()). The builder
  writes title ids as ascending deltas (encode_postings,
  builder.rs:1510-1524: posting.id - previous; a descending id would
  panic in debug and wrap in release), and the decode loop
  (engine.rs:1366-1386) pushes each video documents in stored order,
  videos being filled in corpus scan order (builder.rs:284-341).
  pdqsort sorted-run detection reduces the sort to about one compare
  per element, but it is still a full extra pass per query per term
  that the minimum does not need.
- Scalar BM25 chain (cvtsi2ss/divss/sqrtss/mulss, cgu.01,
  matches_for_entry+0x3ea..0x4f2). Fine for f32 on default x86-64 (no
  FMA without target features); the two divss per text posting are the
  only arithmetic that is truly part of the minimum.
- What already matches the minimum (nothing to do): DocMatch is a
  12-byte packed struct with flag bits (query_support.rs:75 asserts
  size_of::<DocMatch>() <= 12); grouping uses a dense slot table of
  optional VideoMatches instead of hashing (engine.rs:1768-1783) with
  SmallVec<[DocMatch; 3]> inline per group; broad-single lookup
  (cgu.13) is one binary search + 2 indirect memcmp + 1 tail
  decode_answer call with 12 relocations total and no per-probe
  allocation; output work happens only for returned hits ("Decompress
  and parse document blocks only for returned hits", docs/experiments/
  search/raw-inverted-index.md step 12).
- search_single_token is not per-item hot (cgu.01, 1016 instruction
  entries, 74 calls): its 15 drop_glue + 9 __rust_dealloc + 8
  spec_extend relocations are the Vec<String> highlight groups and
  per-alternative union vectors, once per request, not per posting.
  hashmap_random_keys appears twice (the BlockCache::default() and
  format_ungrouped moment_counts), i.e. two SipHash-seeded maps per
  request even when empty.

## 4. Measured attempts (temporary patches, all reverted)

Fixture: 400 videos x 30 chunks = 12 000 docs, 500-token synthetic
vocabulary, term zebra in 1/3 of chunks (4 000 docs, 400 videos),
built and searched in-process in a temp dir via the crate's own
builder. matches_for_entry timed best-of-50 per invocation; search()
best-of-30; release profile; local 8-core host with noisy neighbors
(not the 2-vCPU Fly shape - treat as directional).

Baseline interleaved blocks (10 runs, same build, run order):

```text
BASE decode_all  78.3 48.8 47.5 40.9 48.4 41.1 41.1 42.8 41.2 44.2 us
BASE search_best 1454  986  880  898  950  813  854  796  820  846 us
```

- Variant A - early fast path in read_varint for end == bytes.len()
  (3 runs): decode 50.2/42.2/49.1, search 971/862/942. No win, within
  noise; the added branch costs as much as the compare it removes.
- Variant B - hoist a single-byte fast path into decode_text_postings
  (one bytes[cursor] load + one branch, fallback to read_varint for
  multi-byte; only when position_bytes.is_none()): 8 runs, decode
  median ~44 (min 37.7) vs baseline median 44-49 (min 40.9);
  search_best 777-948 vs baseline 796-1454 in the same wall-clock
  window. At most ~10 percent on decode, <= 5 percent end-to-end, not
  separable from load noise.
- Variant C - #[inline] on read_varint (1-line diff): decode medians
  48.3-61.7 in the post-rebuild batch vs baseline 41.1-44.2 in the
  following block. No improvement. After the attribute, LLVM still
  emitted read_varint out-of-line in one CGU and inlined only some
  call sites (nm: 1 ool definition; 5 remaining PLT32 refs in cgu.01).

Stage isolation (same bench, 3 runs): stage_exact_best_us = 49-52 for
matches_for_entry alone vs search_best_us = 1083-1190 for the whole
request on the same engine. The postings decode is about 4-5 percent
of a served single-token request; the rest is threshold gating,
group-slot filling, top-G selection, zstd + JSON + highlight assembly.
Optimizing the varint loop cannot move end-to-end p50 by more than
~2 percent.

Conclusion: measured, rejected - the decode-loop variants do not pay.
The premise "already tuned" holds: the loop is within about 2x of its
floor, and the floor is a small share of the request.

## 5. Proposals (small, evidence-ranked)

1. Drop matches.sort_unstable_by_key in decode_title_postings
   (engine.rs:1394); replace with
   debug_assert!(matches.is_sorted_by_key(|c| c.document())).
   Evidence: builder invariant (section 3, encode_postings). The sort
   is a no-op pass that still costs one compare per element per query
   per term; it only matters for title-heavy terms decoded without the
   broad-single path (e.g. creator-name queries). Expected effect:
   small; runtime delta is a hypothesis (the call is visible in the
   asm; no isolated timing was taken). Behavior change to flag in the
   PR: on a corrupt index the list would stay unordered and
   union_sorted would emit a wrong merge instead of self-healing;
   corrupt indexes already fail validation on open.
2. Parse params.filter_by once per request instead of per stage.
   Evidence: reaches_result_threshold (single_token.rs:173),
   format_matches (engine.rs:1767) and format_ungrouped
   (engine.rs:1793) each call parse_filter on the same string, 2-3x
   per request, while search already builds SearchConstraints { filter,
   .. } (engine.rs:631-637) that is not passed down. Expected effect:
   microseconds; only worth bundling with a PR that already touches
   that path. Hypothesis - not measured.
3. Do not touch the varint/decode loop. Variants A/B/C above were
   measured; none separated from noise. The remaining incidental
   per-varint overhead (call/ret, 2 checks, sret tag) is bounded by
   the stage-isolation measurement (decode ~4-5 percent of request;
   that overhead is a fraction of that).

Noted, out of scope for one PR: two hashmap_random_keys (two
RandomState seeds) per single-token request (BlockCache::default() at
engine.rs:83/660 and format_ungrouped moment_counts at
engine.rs:1795). A shared precomputed RandomState in the engine would
drop one getrandom; microsecond-level, hypothesis.

## 6. Alternatives considered and rejected

- #[inline] on read_varint: measured no-win (variant C); would also
  grow the text of 10 call sites.
- Unchecked varint reads (get_unchecked): violates the corrupt-index
  error contract; read_varint must distinguish truncated from overflow
  (Error::Corrupt), and the index is user-supplied data.
- SIMD/memchr-style decode: postings are varint streams, not
  fixed-width; no SIMD win without a format change, which is a
  publication project (cf. docs/experiments/search/schema-v6-
  positions.md), not a PR.
- Sort-based grouping instead of the slot table: the slot table
  already meets the minimum (0 hashes); a sort-based grouping adds
  O(n log n).
- Removing the try_stream_broad_single_grouped probe for single
  tokens: that is the documented answer path for broad queries
  (docs/architecture/raw-broad-single-index.md); not a candidate.
- LTO / codegen-units=1: the 16-CGU split is what kept read_varint out
  of line; a [profile.release] lto = "thin" build might fix it
  globally (hypothesis; not tested here because a profile change is
  outside the crate-scoped temporary-edit rule; it belongs in the
  autoresearch harness, autoresearch/program.md track rules).

## 7. Not verified

- Live p95 effect: no production network calls allowed; all timings
  are local, fixture-scale, interleaved-block only.
- The second RandomState cost is inference from the asm, not a
  measured delta.
- Whether thin-LTO would inline read_varint everywhere and whether
  that matters (hypothesis, see section 6).
- Runtime saving of the title-sort removal (codegen evidence only).

## 8. Reverted temporary diffs (exact record)

All three were applied, measured, and reverted; git status was
verified clean (only the temporary bench file) after each revert, and
restoration was done by copying saved copies of the original files.

Variant A, crates/youtube_raw_search/src/format.rs:

```diff
--- a/crates/youtube_raw_search/src/format.rs
+++ b/crates/youtube_raw_search/src/format.rs
@@ -93,6 +93,16 @@
 pub fn read_varint(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u32> {
+    // TEMP-CANDIDATE-E: fast path for end == bytes.len() (all current call
+    // sites), which avoids the per-call cursor<=end and end<=len re-checks.
+    if end == bytes.len() {
+        let first = bytes.get(*cursor).ok_or_else(|| {
+            Error::Corrupt("postings range extends beyond file".to_string())
+        })?;
+        if *first < 0x80 {
+            *cursor += 1;
+            return Ok(u32::from(*first));
+        }
+    }
     let available = bytes
         .get(*cursor..end)
```

Variant B, crates/youtube_raw_search/src/engine.rs
(decode_text_postings):

```diff
--- a/crates/youtube_raw_search/src/engine.rs
+++ b/crates/youtube_raw_search/src/engine.rs
@@ -1312,9 +1312,19 @@
         let mut matches = Vec::with_capacity(entry.text_frequency as usize);
         let mut cursor = 0_usize;
         let mut position_cursor = 0_usize;
         let mut document = 0_u32;
+        // TEMP-CANDIDATE-E: hoisted single-byte-varint fast path with one
+        // cursor bounds check per iteration instead of two checks per varint
+        // inside read_varint (cursor<=end and end<=len, both constant here).
+        let text_len = bytes.len();
+        while cursor < text_len {
+            let first = bytes[cursor];
+            if first < 0x80 && position_bytes.is_none() {
+                cursor += 1;
+            } else {
+                document =
+                    document.saturating_add(read_varint(bytes, &mut cursor, text_len)?);
+            }
             let frequency = read_varint(bytes, &mut cursor, bytes.len())?;
```

(The original first line of the loop body,
document = document.saturating_add(read_varint(bytes, &mut cursor,
bytes.len())?);, moved into the else branch shown above, and
`while cursor < text_len` replaced `while cursor < bytes.len()`.)

Variant C, crates/youtube_raw_search/src/format.rs:

```diff
--- a/crates/youtube_raw_search/src/format.rs
+++ b/crates/youtube_raw_search/src/format.rs
@@ -91,7 +91,11 @@
+// TEMP-CANDIDATE-E: allow cross-codegen-unit inlining; all 14 call sites pass
+// end == bytes.len(), so per-call inlining lets LLVM hoist the loop-invariant
+// end<=len check and fold cursor<=end into each loop's cursor<len condition.
+#[inline]
 pub fn read_varint(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<u32> {
     let available = bytes
```

Measurement harness (temporary; +154 lines appended to
crates/youtube_raw_search/src/engine/tests.rs as
#[test] fn temp_candidate_e_decode_bench, stashed then dropped): a
synthetic corpus builder (400 videos x 30 chunks, 48-token payloads
with periodic zebra) plus a channel file, build_index,
RawSearchEngine::open, best-of-N timing of matches_for_entry
(text,title and text profiles) and of engine.search with
group_by=video_id and filter_by=youtuber_handle:=@one, plus the
stage_exact_best_us / stage_union_best_us block comparing the decode
stage against the full request.
