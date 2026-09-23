# Schema-9 structural directory, first draft

The generation will contain one small `directory.bin` plus immutable data
files. The directory will be written only after every data file is complete and
verified. The reader will open and authenticate the directory, compare every
listed file length with filesystem metadata, and defer payload checksums until
a byte range is used. The directory itself has a trailing BLAKE3 of all bytes
before that digest. It is capped at 16 MiB so cold open cannot read arbitrary
payload data through metadata.

All fixed integers are little endian. The directory header contains an eight
byte magic, schema and directory versions, normalization/rank versions,
generation sequence, captured mutation sequence, a corpus identity digest,
document/video/term counts, the configured bitmap byte budget, and the number
of file records. A record contains a fixed file-kind ID, codec ID, exact byte
length, integrity-unit size, unit count, and one BLAKE3 digest per unit. Records
appear in increasing file-kind order; duplicate or unknown IDs fail open.
Data file names come from the shared file-kind table, never from decoded path
bytes. Files larger than 4 GiB use `u64` byte lengths and offsets.

The integrity-unit size is 2–8 MiB. Its count must equal ceiling(file length /
unit size), with a short final unit. An empty optional data file has zero
units and zero digests. Every declared byte range is checked for overflow and
file bounds before the verifier touches bytes. The footer digest authenticates
the digest directory; the exact generation path and immutable-file lifetime
must also be bound by publication and reader ownership. A digest alone does not
identify a file when its metadata can be substituted.

This is a design draft, not a frozen codec. P01 has not selected the posting
codec or bitmap threshold. Before implementing the directory parser, P03 must
freeze file-kind IDs, header and record byte widths, codec rejection rules,
required versus optional file policy, and the corpus identity calculation.
The implementation must test malformed/truncated/extended directory bytes,
file substitution, wrong lengths, duplicate IDs, and every short-unit edge.
