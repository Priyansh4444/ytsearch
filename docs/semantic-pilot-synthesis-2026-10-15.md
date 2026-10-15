# Qdrant and EmbeddingGemma pilot decision — 2026-10-15

The implementation uses self-hosted Qdrant 1.19.2 and pinned EmbeddingGemma-300M
Q4 weights with float32 CPU inference. The default is a normalized 128-coordinate
prefix of the model's 768-coordinate output; 256 is an explicit comparison option.
There is no paid inference endpoint or turbopuffer dependency. A new model trained
from scratch is unnecessary before measuring this pretrained encoder on judged
transcript queries.

## Implementation comparison

Two isolated prototypes explored direct `ort` tensor assembly and FastEmbed's
local user-defined model interface. Provider usage limits interrupted both original
runs; the FastEmbed version was recovered and fully exercised. The path labels in
the review were X (recovered FastEmbed) and Y (direct ONNX Runtime).

| Criterion (5 is best) | X: FastEmbed | Y: direct ORT |
| --- | ---: | ---: |
| Small dependency/runtime surface | 4 | 5 |
| Complete retrieval and API path | 5 | 4 |
| Profile, input and resource guards | 5 | 4 |
| Retry and replacement safety | 5 | 3 |
| Maintainer clarity and reproducibility | 4 | 4 |

The recovered version is the base for the importer, profile and API contract.
Its receipt validates exact snapshot bytes and complete-video chunk counts before
writes, and reconciliation checks existing payloads and completed acknowledgements.
The final inference loader comes from the direct ORT design: a measured comparison
showed about 334 MiB steady RSS versus 636 MiB for the FastEmbed prototype, and
399 MiB startup peak versus about 842 MiB. These are local process measurements,
not a Fly memory allocation or full-corpus capacity claim. This measured saving
justifies the small explicit tensor boundary and removes FastEmbed from the build.

Grafts from the direct prototype: file-backed ORT graph loading, streaming asset
hashes, explicit tensor extraction, an inference wait timeout and disk-backed
Qdrant payloads. Parent review additionally requires disk-backed vectors and
checks both storage flags when opening a collection. The recovered version's
single admitted request and one-item channel remain; the larger alternative
queue was rejected.
Full-video hashes were rejected for identity because an unrelated late caption
edit would re-embed every passage; passage-local source hashes keep unaffected
passages reusable. No implementation uses Python for serving or inference.

The real-caption trial exposed a shared prototype assumption: source chunks can
overlap in time. The existing `youtube_ingest::Chunker` preserves YouTube overlaps.
The final importer accepts ordered starts, consumes each chunk once, and keeps the
minimum original start and maximum original end. It never invents nonoverlapping
timestamps. A test exercises this through the real chunker, including token-driven
splits and nested ranges. This changes the locked passage profile.

The fresh independent judge also chose X: footprint 3/4, complete path 5/3,
validation 4/3, reconciliation 5/2, clarity 4/3 (X/Y). It identified a creator-case
mismatch: existing API normalization preserves case while X required lowercase.
The final import/service boundary canonicalizes ASCII handles, and the optional
API adapter lowercases only semantic filters. Mixed-case tests cover all three.
The judge's other concerns were Y's missing completeness receipt and completed
write checks; those implementations were rejected. Its timeout/storage and file-backed inference grafts
are included. The first cross-provider judge failed a rate limit without review;
the successful fresh judge used the inherited provider. The first review was on prototypes. A fresh final review inspected the
synthesized ORT loader, importer, API, scripts and docs and found no blockers.
Final runtime verification below covers the synthesized artifact.

## Operational boundary

This is a local snapshot pilot. Its loopback service and explicit API opt-in let
operators evaluate search without changing lexical defaults, queue writers,
production index formats, Fly resources, or indexing costs. One local importer
serializes per collection. Replacements can briefly expose old and new points;
imports should finish before stable evaluations. Missing videos are retained.
The receipt attests export completeness, not truth: exporters must finish each
video before issuing it. Automatic source synchronization and production rollout
remain separate work.

Use [the playbook](playbooks/semantic-pilot.md) for verified setup and limits.

## Measurements and cost decision

The final debug build imported 151 source chunks from five complete cached videos
into 23 passages in 22.3 seconds. It used about 336 MiB steady model RSS and
402 MiB importer peak RSS. Query embeddings took 32–71 ms across six local probes;
these samples are not latency percentiles or Fly measurements. The three topic
queries placed their corresponding videos first: Next.js server actions, a
two-sided polygon, and long-running AI agents. Creator/video filters held,
including mixed-case creator spelling; an absent creator produced no hits.
An applause-only passage still scored highly from its title context, so this
small trial does not establish production relevance or no-answer quality.

A repeated import embedded zero passages. A deliberately shorter local fixture
removed obsolete passages; an unchanged part remained reusable. A truncated JSONL
prefix with its original receipt was rejected before writes. The API returned
semantic video cards and retained lexical routing for `I`, `fable on high` by
default, and explicit date sorts. The lexical transport in this local API test
was a fixture, not a production relevance comparison. A checked-in
[smoke test](../scripts/test-semantic-pilot.py) reproduces these lifecycle checks
with disposable synthetic data and real CPU inference. The two loaders produced
identical stored Float16 vector coordinates for the 16 compared final points.
See [machine-readable evidence](semantic-pilot-evidence-2026-10-15.json).

At 128 dimensions and Float16 storage, one raw vector is 256 bytes; one million
vectors alone occupy about 244 MiB. This excludes HNSW, text, metadata, WAL,
page cache and backups. The model graph and external weights total about 197 MB
(decimal); tokenizer/config assets add about 21.5 MB. Self-hosting avoids a
managed vector-service or per-token inference bill, but still consumes CPU,
storage and machine uptime. Initial embedding of the full catalog is the main
compute task. This five-video timing cannot price that backfill or establish a
minimum production Machine size. The 256 MiB API allocation cannot host this
measured model process; inference stays separate. No new Fly resource was created.

[EmbeddingGemma's model card](https://huggingface.co/google/embeddinggemma-300m)
specifies 300M parameters and supports 128-dimensional prefixes with normalization.
Float32 activations are required for this CPU graph; Float16 vector storage is a
separate Qdrant setting. This is a compact Gemma encoder, not the smallest encoder
of every family. If a tighter memory budget is required, benchmark the smaller
[MiniLM baseline](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2)
against the same judged queries before training a custom model. Existing model
quality, chunking and retrieval should be evaluated before spending on training.

## Verification

`bun run check:precommit` passed. The final full check passed 345 Rust tests,
122 frontend tests, 40 Worker tests and all 18 browser tests, then stopped at the
same four baseline Hawk findings in `youtube_index_store`: `publish`,
`delete_video`, `operation`, and `ADMIN_REQUEST_TIMEOUT`. Later checks were not
reached; the complete suite is not green. Focused import-failure tests cover safe
retry after partial writes, receipt truncation, profile mismatch and shrinking
videos. The actual Gemma/Qdrant/API chain and checked-in smoke test passed.
