# Raw mutable full-corpus publication

Date: 2026-08-09

## Corpus and method

- Base documents: 8,142,650 transcript chunks.
- Base index: schema-v6 Raw generation, 1,444,907,196 bytes.
- Source: the checksummed engagement JSONL gzip production archive.
- Mutation: durable deletion of one existing video, followed by explicit publication.
- Validation: query returned one video before publication and zero after atomic activation.
- Recovery: a second deletion was published after restarting from generation 1.

The server continued serving the old generation throughout each build. Control-plane chunk counts
reflected the durable deletion immediately; public search changed only after the verified manifest
was atomically activated.

## Results

| Measurement | Result |
| --- | ---: |
| First full publication wall time | approximately 70 seconds |
| Second full publication wall time | approximately 80 seconds |
| Peak process RSS | 2,576,420 KiB (2.46 GiB) |
| Peak mutable-state disk usage with one prior generation | approximately 8.7 GiB |
| Post-publication RSS before allocator reclamation | approximately 1.13 GiB |
| Post-publication RSS after `malloc_trim(0)` | 141,772 KiB (138.45 MiB) |
| Activated generation documents | 8,142,175 after both test deletions |
| Publication errors | 0 |

The second run retained one rollback generation while building the next candidate. This is the
relevant peak-disk scenario for production, and it supports a 20 GB mutable volume with more than
2x measured headroom.

## Decision

- Keep query generations immutable and memory-mapped.
- Keep mutations in the SQLite WAL until an explicit batch publication.
- Publish once per single-channel queue job and once per full multi-channel command.
- Retain one previous generation for rollback.
- Reclaim glibc builder arenas after every publication.
- Provision the existing half-sized 4 GB Raw machine; measured publication peak is below 2.5 GiB.

This is a correctness-first compaction design, not a claim of instant per-document visibility.
Durability is immediate, control-plane visibility is immediate, and public visibility is atomic at
the batch boundary.
