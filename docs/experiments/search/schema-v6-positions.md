# Raw schema-v6 complete-position storage

Measured 2026-08-08. This slice adds complete token positions without moving
cold phrase data onto the ordinary-query path.

## Representation and invariants

Schema v6 keeps the schema-v5 `positions.bin` stream unchanged: each posting
has one delta-varint first position, and each term/field has one range in the
offset table. The new `positions.full.bin` uses the same term/field ranges but
stores only the remaining positive position deltas. Posting frequency supplies
the number of values, so the cold file needs neither a per-posting count nor a
per-posting offset.

The builder retains only term frequencies, first positions, and linked repeat
nodes for the current document. It serializes each completed list into a
temporary disk spool, then replays that spool while writing sorted postings.
This avoids a corpus-wide `Vec<Vec<u32>>`. A single position scratch vector and
a single 256 KiB checksum buffer are reused. The spool fails closed above 4
GiB, and a single field/term fails above 65,535 occurrences or encoded bytes.

Every immutable generation file is recorded in `manifest.json` with its exact
byte length and BLAKE3 digest. Schema-v6 open verifies all nine files before
mapping and serving them. Tests cover truncated hot and cold positions,
postings, document blocks, malformed codec data, and a same-length bit flip.
There remains a named time-of-check/time-of-map window until atomic generation
activation lands; the serving contract therefore still requires immutable
generation directories.

## Measurement contract

- Hardware: Intel Core Ultra 7 255H, 16 online cores, Linux 7.1.3 x86-64.
- Build: Cargo `--release`, one-shot local build.
- Corpus: 47,860 Veritasium/Vsauce transcript chunks from 787 videos.
- Query workload: nine frozen non-phrase queries, 900 sequential warm HTTP
  searches per variant, run in alternating schema-v5/schema-v6 order.
- Correctness: byte-identical responses for all nine queries.
- Acceptance: schema-v6 non-phrase p95 no more than 10% above schema v5.
- Rollback: the same binary continues to open schema-v5 generations; deleting
  the schema-v6 candidate restores the prior representation and responses.

## Results

| Measurement | Schema v5 | Schema v6 | Change |
| --- | ---: | ---: | ---: |
| Index bytes | 11,911,860 | 12,261,425 | +349,565 (+2.94%) |
| One-shot build | 305 ms | 269 ms | -36 ms |
| Warm HTTP median | 1.863 ms | 1.486 ms | -20.2% |
| Warm HTTP p95 | 4.636 ms | 3.199 ms | -31.0% |
| Warm HTTP mean | 2.100 ms | 1.678 ms | -20.1% |
| Observed steady RSS | 25.32 MB | 27.12 MB | +1.80 MB |

The schema-v6 sidecars were 1,139,445 bytes for `positions.bin` and 348,321
bytes for `positions.full.bin`. The artifact increase is therefore small on
this corpus and the p95 gate passes. The latency improvement is not attributed
to storing more positions: ordinary queries never read the cold mapping. It is
an accompanying code-path improvement from removing a redundant token-counting
prepass in query token deduplication. Two internal-mean pairs were
1.585/1.444 ms for v5 and 1.249/1.217 ms for v6.

RSS and startup observations are secondary and noisy. Five readiness samples
were 6/24/8/24/24 ms for v5 and 22/27/24/25/24 ms for v6; millisecond polling
is too coarse to call a difference. The observed v6 RSS includes checksum work
and mapped offset pages, so the 1.8 MB delta is recorded rather than presented
as a stable regression estimate.

## Correctness and rollback record

The correctness invariant is that the hot first-position stream and cold
remaining-position stream contain exactly one aligned list per posting, and
that reconstructing a list yields strictly increasing positions whose count
equals posting frequency. Unit tests reconstruct repeated real writer output,
and ordinary schema-v5/schema-v6 searches return identical JSON.

Reject or roll back this representation if non-phrase p95 exceeds the 10% gate,
response parity changes, generation verification fails, or production-shaped
build memory violates the future 512 MiB compaction gate. Phrase and proximity
ranking are intentionally not claimed by this slice; the reader and relevance
oracle must pass separately before those TODO items close.
