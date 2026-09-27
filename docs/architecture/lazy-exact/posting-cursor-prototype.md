# Schema-9 posting cursor prototype

Status: internal P02 structure. This is not connected to a writer, generation,
or production reader. P01 has not selected the final posting codec. P03 must
freeze the on-disk descriptor and lazy validation contract before publication.

Each provisional block holds at most eight strictly increasing document IDs and
nonzero frequencies. Its first ID is stored as a little-endian `u32`; later IDs
use canonical delta varints. Frequencies use canonical unsigned varints.
Positions are deliberately separate. Directory levels group 8, 64, 512, and
4096 postings. Each checkpoint holds the first and last document ID, first
ordinal, and byte offset for its group. A seek chooses the first group whose
last ID reaches the target at each level, then decodes at most one base block.
`current` observes the current posting, `advance` seeks without consuming or
rewinding, and `next` consumes it.

The constructor rejects duplicate/decreasing IDs, out-of-universe IDs, and
zero frequencies. The decoder rejects truncated and noncanonical varints,
bad block lengths, overflow, and mismatched checkpoint bounds. Tests enumerate
the complete encoded sequence, cross every skip boundary, seek to `u32::MAX`,
exercise malformed bytes, and compare 1000 random seeks against a sorted-vector
oracle. Decode counters distinguish blocks visited from decoded posting IDs.

This prototype reports retained encoded bytes separately from retained vector
capacity, including all four skip directories. It does not reserve that memory
under the session budget or include the temporary base directory held during
construction, allocator metadata, or the list header. It also does not validate a
borrowed on-disk directory lazily. The 2 GiB and 200 ms gates require P01 codec
comparison on the retained corpus, P03 file-format integration, and P10 reader
measurements before this cursor can be considered production-ready.
