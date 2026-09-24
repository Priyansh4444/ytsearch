# Schema-9 directory wire format, version 1

This freezes the **directory metadata** format for the P03 prototype. It does
not choose the posting block codec or make the production reader use schema 9.
The directory is written after immutable data files. The reader must bind its
exact digest to an authenticated generation manifest before accepting it.

All integers are unsigned, fixed width, little endian. There is no native Rust
struct serialization. Offsets in this document are zero based from the start
of `directory.bin`; file lengths and data offsets are `u64`. The maximum
directory length, including its trailer, is 16 MiB. The minimum is 132 bytes.
No extension or trailing padding is accepted in version 1.

| Header offset | Width | Field |
| ---: | ---: | --- |
| 0 | 8 | ASCII `YTS9DIR` followed by NUL |
| 8 | 2 | storage schema, exactly 9 |
| 10 | 2 | directory version, exactly 1 |
| 12 | 2 | normalization version, nonzero |
| 14 | 2 | rank version, nonzero |
| 16 | 8 | generation sequence, nonzero |
| 24 | 8 | captured mutation sequence |
| 32 | 32 | corpus digest, nonzero BLAKE3 value |
| 64 | 8 | document count, at most `u32::MAX` |
| 72 | 8 | video count, at most `u32::MAX` |
| 80 | 8 | term count |
| 88 | 8 | configured bitmap budget in bytes |
| 96 | 2 | file record count, at most 11 |
| 98 | 2 | reserved, exactly zero |

The corpus digest is a **provisional opaque 32-byte identity** in this
prototype. Its production calculation is not frozen: a Raw generation depends
on both the base corpus and the captured mutation snapshot. The current
publication job records `snapshot_blake3`, target sequence, and compatibility
(`corpus_identity`, `base_chunks_blake3`, `base_channels_blake3`), while the
current generation manifest has no schema-9 directory binding. P03 integration
must define a canonical binary digest over those exact inputs and save it in
the trusted publication manifest. The manifest must bind snapshot identity,
mutation sequence, generation sequence, and directory digest together. The
directory parser only checks field shape; neither its trailer nor this field
authenticates a substituted directory or corpus.

Each record starts immediately after the 100-byte header. Records are sorted
by increasing file-kind ID. Every required kind appears exactly once. The only
optional kind is bitmaps (5). Duplicate, missing required, out-of-order, and
unknown IDs fail open. File names come from the kind table, never from decoded
path bytes. The ten required files have nonzero length, including for an empty
corpus: their own format headers remain present. The optional bitmaps file may
be absent or present with zero length.

| ID | File name | Family | Policy |
| ---: | --- | --- | --- |
| 1 | `terms.bin` | term dictionary | required |
| 2 | `postings.bin` | posting doc/frequency blocks | required |
| 3 | `skips.bin` | multilevel skip directory | required |
| 4 | `rank-bounds.bin` | conservative block rank metadata | required |
| 5 | `bitmaps.bin` | high-DF occurrence bitmaps | optional |
| 6 | `positions.bin` | position streams | required |
| 7 | `documents.bin` | document fast fields | required |
| 8 | `videos.bin` | video fast fields | required |
| 9 | `facets.bin` | facet membership | required |
| 10 | `payloads.bin` | document payload units | required |
| 11 | `channels.bin` | channels and external IDs | required |

| Record-relative offset | Width | Field |
| ---: | ---: | --- |
| 0 | 2 | file-kind ID |
| 2 | 2 | outer file codec, exactly 0 (`raw`) |
| 4 | 8 | exact file byte length |
| 12 | 4 | integrity-unit byte size, 2–8 MiB inclusive |
| 16 | 4 | integrity-unit count |
| 20 | `32 × count` | BLAKE3 digest of each consecutive raw file unit |

Codec 0 means the outer file is not compressed. Posting blocks and payload
units may carry their own codecs inside their file formats; this directory
does not select those codecs. A new outer codec requires a new directory
version. Unknown codec values fail open.

Unit count is exactly `ceil(file_length / unit_size)`; zero length has zero
units and digests. The final unit is hashed over its actual short length. A
file of exact unit size has one digest. The directory trailer is a 32-byte
BLAKE3 digest of every preceding byte, starting at offset 0. Decoding requires
exact consumption of all bytes before the trailer and uses checked arithmetic
for offsets, lengths, and digest allocation. The reader then compares every
declared file length with filesystem metadata before accessing data.

`Directory::decode` validates metadata and checksum. `Directory::decode_bound`
also requires the generation sequence, mutation sequence, corpus digest, and
directory digest supplied independently by a trusted generation manifest; a
validly checksummed substituted directory fails that check. It does **not**
verify data file digests. `VerifiedUnits` checks each touched 2–8 MiB data unit once
before returning its bytes; a caller must retain immutable backing files for
its entire lifetime. The production reader must also validate the trusted
manifest binding, open files without substitution races, and preserve them
through pinned sessions and rollback. Those integrations are still open.
