# Schema-9 lazy integrity unit prototype

`schema9::verified_units` checks a 2–8 MiB BLAKE3 unit only when a caller first
reads bytes intersecting it. It rejects invalid unit sizes, incorrect digest
counts, overflowing or out-of-file ranges, and digest mismatches. The short
final unit hashes its actual length. A zero-length file has no digests and is
valid. Concurrent readers share a `OnceLock` per unit, so a success or
corruption failure is computed once and then retained.

This is an internal P03 primitive. The caller must keep the backing file
immutable for the verifier's lifetime. It currently accepts a borrowed byte
slice and an already validated digest vector; there is no footer parser,
generation identity, mmap lifetime management, writer, or production reader
integration yet. A whole-file checksum is not performed at open. The current
tests cover untouched units, overlapping reads, a short final unit, concurrent
corruption, invalid metadata/ranges, and empty files. The cold-open physical
read and 2 GiB reader gates remain open.
