# Raw generation schema deprecation

Status as of 2026-09-27. Each schema is removed from the reader only after no
production artifact or rollback path depends on it.

| Schema | Contents | Production role | Status |
| --- | --- | --- | --- |
| 5 | Postings, first positions, metadata | None | **Removed.** The reader rejects it at open. |
| 6 | Schema 5 plus complete positions and manifest checksums | The immutable base index (`/data/index`, 8,142,650 documents), opened only when a state directory has no active generation: fresh canary trials and recovery. Production serves its active schema-7 generation. | Supported until the base is rebuilt in a newer schema |
| 7 | Schema 6 plus bounded bigram sidecars | Every published generation (`raw-index-schema-7-v2`) | **Current** |
| 8 | Schema 7 plus materialized broad single-token answers (`broad-single.bin`) | None; never activated | **Deprecated.** Nothing builds it; the reader still opens it |
| 9 | Structural lazy-exact format | Not yet built | Target; see [readiness](../architecture/lazy-exact/readiness-2026-09-27.md) |

## Why schema 8 is deprecated

Every publication rebuilds the full generation, and schema 8 adds a
multi-gigabyte broad-answer phase to each rebuild. That phase never passed the
2 GiB reader's build or cold-open gates (see
[raw-schema8-validation.md](raw-schema8-validation.md)). Partitioned scans,
lazy posting checkpoints, and warm-before-swap publication brought the schema-7
reader under 200 ms on the canonical matrix without it, and schema 9 replaces
materialized answers with exact sets and block-bounded ranking.

## Removal steps that remain

- **Schema 8 read support** (`engine/broad_single.rs`, the schema-8 manifest and
  publication allowlists, and the legacy test fixtures): remove after the owner
  approves deleting the retained, never-activated schema-8 artifact on the
  stopped publication builder volume (`080d297f070d08`, job sequence 2152,
  2,738,580,172 bytes; see P00). Until then it is rollback evidence.
- **Schema 6 read support:** remove after the immutable base index in the
  reader image is rebuilt in a newer schema, so canary trials and recovery
  never open a schema-6 generation, and a reader release that opens it has
  been promoted.
