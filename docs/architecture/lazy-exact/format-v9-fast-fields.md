# Schema-9 fast-field layout, design revision 1

Status: P03 wire-format sketch. The schema-9 writer and reader do not yet use
these files. The implementation must pass the P03 structural and live gates
before this layout can be treated as a production format.

## Caller contract

A query obtains dense document IDs from its exact match set. `FastFields`
returns a document's dense video ID and transcript start without decoding its
external ID, payload, or unrelated numeric fields. The writer assigns IDs in
`(dense_video_id, t_start_ms, external_document_id_bytes)` order, so a scan of
ascending IDs yields the order required by the exact 20-second greedy moment
reducer. Query provenance determines whether a matched document contributes a
transcript moment; title expansion alone must not fabricate transcript hits.

A page hit resolves exact external UTF-8 IDs on demand. A video lookup reads
creator, kind, year, duration, publication time, and engagement values with
missingness preserved. Facet membership is separately indexed in `facets.bin`;
the video columns are the source for validation, numeric ranking, and exact
aggregations. Cold open reads only the bound directory and file metadata.

## Source and identity gates

Dense IDs are generation-local `u32`s. The writer must externally sort and
spool canonical chunks plus mutations before assigning them, then build
postings, facets, payload locators, and video ranges against those same IDs.
The existing schema-6 builder assigns IDs in input order and does not establish
this global order. The exact source `TranscriptChunk.id` survives in the
canonical archive; the schema-9 writer must prove that every source document
has one unique, stable external ID. Missing or ambiguous IDs fail publication.
The private source spool appends each validated JSON line to a scratch file and
uses a disk-backed SQLite unique key to reject duplicate external IDs. Its
ordered scan uses `(external_video_id, t_start_ms, external_document_id)` so
dense video and document IDs can be assigned without retaining the corpus in
memory. The spool is rejected after any append error. It does not yet resolve
per-video metadata conflicts or prove that a mutation snapshot is complete;
both remain publication gates. Scratch space and sort runtime must be measured
on the real corpus before this path can be used for production builds.

Canonical chunks carry optional `duration` in milliseconds and optional
`published_at_timestamp` in whole Unix **seconds**. Kind is derived from a
proven duration: `Short` for 0 through 60,000 ms, `Video` above 60,000 ms,
and `Unknown` when duration is absent. A known zero duration remains known.
The independent reference currently lacks `Unknown`; parity cannot be claimed
until that fixture contract is repaired. Year is derived from a known Unix
timestamp using UTC Gregorian rules. A missing timestamp implies missing year.
Schema-6 `IndexDocument` defaults views, likes, and comments to zero, erasing
absence. The schema-9 writer must read raw archive/mutation field presence;
defaulted zero is not evidence that the source provided a zero.

## Common wire rules

All fields are fixed-width little-endian; signed publication seconds use
two's-complement `i64`. There is no native Rust layout or implicit padding.
Offsets are `u64` absolute byte offsets from the start of their file. The
directory supplies exact file lengths and 2–8 MiB BLAKE3 integrity units.
Every multiplication, addition, range, and `u64` to `usize` conversion is
checked before borrowing bytes. Header and descriptor bytes are lazily read
through the shared unit verifier. Zero-row files retain their headers and
descriptors. Unknown versions, tags, codecs, flags, nonzero reserved bytes,
holes, overlaps, trailing bytes, and malformed UTF-8 fail closed.

Each file begins with a 64-byte header, followed by an exact ordered table of
32-byte section descriptors, zero padding to the next 8-byte boundary, and
sections in descriptor order with no gaps. The header is:

| Offset | Bytes | Field |
| ---: | ---: | --- |
| 0 | 8 | `YTS9DOC\0` or `YTS9VID\0` |
| 8 | 2 | storage schema `9` |
| 10 | 2 | file version `1` |
| 12 | 2 | header bytes `64` |
| 14 | 2 | descriptor bytes `32` |
| 16 | 8 | row count, equal to the bound directory count |
| 24 | 4 | descriptor count: documents `6`, videos `14` |
| 28 | 4 | zero |
| 32 | 8 | generation sequence, equal to bound directory |
| 40 | 8 | captured mutation sequence, equal to bound directory |
| 48 | 16 | zero |

Each descriptor has tag `u16` at offset 0, raw codec `u16=0` at 2, element
width `u32` at 4 (zero for a blob), absolute section offset `u64` at 8,
section length `u64` at 16, and zero reserved `u64` at 24. Fixed-array length
is exactly `row_count × width`. The final section ends at the declared file
length. The complete descriptor table is under 512 bytes.

### `documents.bin`

| Tag | Section | Width |
| ---: | --- | ---: |
| 1 | spine `(video_id:u32, t_start_ms:u32)` | 8 per document |
| 2 | indexed text token count | 4 per document |
| 3 | field flags: bit 0 transcript indexed, bit 1 video title indexed; others zero | 1 per document |
| 4 | external document ID blob offset | 8 per document |
| 5 | external document ID UTF-8 byte length | 4 per document |
| 6 | exact external document ID bytes | blob |

ID spans exactly partition the blob in document order. Every ID is nonempty,
valid UTF-8, and unique generation-wide. `payloads.bin` locates payloads by
dense document ID, so this file does not carry a redundant payload offset.
Document `video_id` must be in range and all video ranges must partition the
document IDs. Within each video, `(t_start_ms, external_id_bytes)` is sorted.

### `videos.bin`

| Tag | Section | Width |
| ---: | --- | ---: |
| 1 | dense creator ID | 4 per video |
| 2 | kind: `0=Unknown`, `1=Short`, `2=Video` | 1 per video |
| 3 | UTC Gregorian year, `i32`, zero when absent | 4 per video |
| 4 | duration milliseconds, `u32`, zero when absent | 4 per video |
| 5 | publication Unix seconds, `i64`, zero when absent | 8 per video |
| 6–8 | views, likes, comments, `u64` each | 8 each per video |
| 9 | missing mask: bits 0 kind, 1 year, 2 duration, 3 publication, 4–6 counters; bit 7 zero | 1 per video |
| 10 | external video ID blob offset | 8 per video |
| 11 | external video ID UTF-8 byte length | 4 per video |
| 12 | exact external video ID bytes | blob |
| 13 | first dense document ID | 4 per video |
| 14 | number of documents in contiguous range | 4 per video |

The missing mask is authoritative; a missing numeric value must be zero and a
present zero remains distinct. Kind must agree with proven duration, and the
kind missing bit equals the duration missing bit. Year must agree with present
publication seconds, or both are absent. Creator IDs resolve in `channels.bin`.
External video IDs are nonempty, valid UTF-8, and unique. Video document ranges
are adjacent, disjoint, and cover all documents. Zero-document videos require
an explicit catalog/query policy before publication; no implicit inclusion.

## Reader and writer seam

`schema9/fast_fields.rs` owns pure header/descriptor/row decoders and a
`FastFields<'g>` view over `OpenedGeneration`. Its first useful operation is
`document_spine(DocId) -> (VideoId, u32)`; it reads only tag 1 through the
shared verifier. Later accessors decode video columns and exact IDs lazily.
They return typed `Option` values and never expose unchecked mmap slices.
Sequential scans batch adjacent spine entries inside verified units.

The writer streams disk-backed columns from sorted source runs and performs a
full audit of global uniqueness, ordering, video ranges, source provenance,
field presence, cross-file payload references, and derived kind/year before
the P03 directory finalizer runs. The finalizer's per-unit hashes do not prove
these structural relations. A failed build remains unpublished. Query-specific
answers never appear in a structural file.

For active generation 2950's 8,422,419 documents, the 8-byte spine is about
64.3 MiB; a 40-byte fixed row would require about 321 MiB for the same broad
scan. These are disk/mapped bytes, not measured RSS or query latency. The
exact 2 GiB reader gate still requires cold/warm physical I/O, bytes hashed,
RSS, complete count/Tier-0/page-one timing, and the 300-query differential.

## Synthesis

The fixed-row candidate favored one-step point lookups; the bounded-column
candidate favored a small
video/time scan spine. The independent cross-review selected bounded columns
because the broad exact-summary gate is materially more plausible at 64 MiB
than at 321 MiB. This sketch grafts fixed rows' checked address discipline,
typed missingness, and explicit external-ID/source audit. It drops the
column candidate's redundant payload locator and corrects its publication
unit from milliseconds to seconds. It derives kind from duration instead of
assuming an unavailable source registry. Fixed rows remain a fallback if real
canary traces show descriptor/column access costs dominate; no performance
claim is made before that measurement.
