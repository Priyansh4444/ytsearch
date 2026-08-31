# Raw inverted-index experiment

`youtube_raw_search` is a read-only search backend implemented without Tantivy,
Lucene, Typesense, or another search library. It exposes both the existing
Typesense-shaped route and an explicit raw route:

- `GET /collections/{collection}/documents/search`
- `GET /raw/collections/{collection}/documents/search`
- `GET /health`
- `GET /metrics`

The metrics response reports cumulative end-to-end `search_nanos` separately
from `queue_wait_nanos` and `backend_nanos`. This matters because the 2 GB
reader deliberately serializes uncancellable blocking searches: a slow posting
scan can make the next request wait even when that request's own engine work is
fast. The benchmark resource artifact records all three deltas.

## Representation

The immutable index contains:

| File               | Purpose                                                                    |
| ------------------ | -------------------------------------------------------------------------- |
| `lexicon.bin`      | Sorted UTF-8 terms and offsets, binary-searched through mmap               |
| `fuzzy.bin`        | Compact mmap trie for guarded prefix and Damerau-Levenshtein expansion     |
| `postings.bin`     | Delta-varint document/video IDs and term frequencies                       |
| `positions.bin`    | Compact per-term offsets and first-token-position delta-varint streams     |
| `positions.full.bin` | Cold remaining-position deltas for phrase/proximity reads                |
| `documents.meta`   | Fixed-width block, offset, length, video, and token counts                 |
| `documents.blocks` | Independent 64-document zstd frames                                        |
| `videos.json`      | Video metadata, engagement counters, and compact document membership lists |
| `channels.json`    | Existing channel response documents                                        |
| `manifest.json`    | Version and cross-file count invariants                                    |

Text is indexed once per transcript chunk. Titles are indexed once per video,
then expanded through that video's document list during query evaluation. This
removes the repeated-title postings present in a flat per-chunk schema.

Only query posting lists and selected compressed blocks are touched. The index
is not copied into a process-owned hash map at startup.

## Query pipeline

1. Tokenize and binary-search the memory-mapped lexicon.
2. Decode only the requested delta-varint posting lists.
3. Intersect strict terms and let exact/split results satisfy the five-result
   typo threshold before doing prefix or fuzzy work.
4. For a compact single token, try at most eight lexicon-proven splits. Each
   side must match the same chunk; a narrow plural alternative is allowed only
   inside this guarded path. Reject a pair from lexicon metadata before decoding
   when its field-specific posting work exceeds one million records, and skip
   positional refinement above 50,000 surviving candidates.
5. Traverse the compact trie for frequency-ranked prefix candidates only when
   exact coverage remains low.
6. Expand progressively to one and then two Damerau-Levenshtein edits using
   the active Typesense length gates. A final compact-token fallback may pair
   one exact split component with one one-edit component in the same chunk.
7. Keep every multi-token expansion conjunctive. If no result remains, drop
   suffix tokens one at a time, then prefix tokens, stopping at the longest
   filter-visible subset. Each ladder is capped at 20 candidates.
8. For broad two-plan conjunctions, merge mmap posting cursors directly instead
   of materializing multi-million-entry intermediate vectors. Coverage, prefix
   alternatives, title/text cross-field matches, and BM25 remain exact.
9. Rank by retained-token count, a packed exact-token mask, BM25/title score,
   a candidate-targeted 10-token proximity window, and early position. The mask
   compares exact-token count first, then earlier retained query tokens second
   while sharing the existing 8-byte hot candidate rather than allocating
   per-hit provenance.
10. Apply an explicit date, engagement, or bounded relevance-plus-engagement
   order when requested.
11. Group scored chunks through dense generation-local video slots and retain
    only the requested top moments.
12. Decompress and parse document blocks only for returned hits.

BM25-style text length normalization and a title boost provide the initial
rank. Engagement counters are stored once per video and use logarithmic corpus
normalization. The combined sort caps engagement at a 20% text-score boost;
pure views, likes, and comments sorts are separate explicit modes. See
[engagement-ranking.md](engagement-ranking.md) for the formula and measurements.

## Safety and corruption behavior

The only `unsafe` operation creates read-only memory maps. Its contract is that
an index is immutable after build and the service exposes no write endpoint.
Every typed read validates file bounds, header magic, version, record counts,
posting/position counts, monotonic position offsets, block lengths, and
document ranges before materialization. Schema v6 additionally verifies the
recorded byte length and BLAKE3 checksum of every generation file at open.

Missing numeric publication timestamps are derived from `published_at` through
the shared `youtube_time` parser during index construction.

Tests cover varint boundaries and malformed encodings, arbitrary input order,
grouping, filtering, relevance/date sorting, highlights, compact splitting,
plural recovery, prefixes, adjacent transpositions, two-edit correction,
typo-plus-split recovery, conjunctive token dropping in both directions,
filter-aware stop thresholds, exact-token priority, position ties, exact
quoted phrases including duplicate tokens, pagination,
field selection, wildcard/channel search, empty corpora, concurrency,
deterministic slicing, overwrite refusal, and corrupt/truncated files including
the fuzzy trie and position sidecar.

## Build and serve

```sh
cargo run --release -p youtube_raw_search -- build \
  .search-experiment/corpus/chunks.jsonl \
  .search-experiment/corpus/channels.jsonl \
  .search-experiment/raw-index

RAW_SEARCH_BIND=127.0.0.1:8120 \
cargo run --release -p youtube_raw_search -- serve \
  .search-experiment/raw-index
```

The builder refuses to overwrite a completed index directory. Build into a new
directory and switch the process only after `RawSearchEngine::open` validates
all files. The current binary reads schema v5 as a non-phrase rollback and
schema v6 for complete positions; quoted queries fail closed on schema v5.

For a full snapshot, keep the provider-neutral backup compressed and stream it
through a FIFO so the multi-gigabyte JSONL corpus is never duplicated on disk:

```sh
RAW_CHUNKS_ARCHIVE=.search-experiment/backups/<timestamp>/youtube_transcript_chunks.engagement.jsonl.gz \
  scripts/build-raw-generation-from-backup.sh \
  .search-experiment/backups/<timestamp> \
  .search-experiment/raw-index-<timestamp>
```

The wrapper verifies backup checksums, compares document/channel counts, and
records elapsed time and sampled peak RSS for the release binary. It only
builds a versioned candidate; activation remains a separate,
rollback-preserving operation after the search contract and resource gates
pass. A custom enriched chunks archive must have a sibling `.sha256` file and
is verified independently from the immutable source backup.

## Historical deployment record

The retired isolated deployment used
[archived experiment Fly configuration](https://github.com/Priyansh4444/ytsearch/blob/b00c51d/fly.raw-search-experiment.toml). Its
Worker, API app, and engine app were removed after the production gate, while
the experiment configuration and results remain in version control.

The first production promotion used a 4 GiB, two-performance-vCPU Raw engine.
That allocation and its direct Fly deployment commands are historical. Do not
use them as current deployment instructions.

Use [Deployment](../../deploy.md) for the current release procedure and [Raw
production cutover](../../operations/raw-production-cutover.md) for the dated
promotion evidence and conditional rollback path.

See [raw-validation.md](raw-validation.md) for the real-corpus results and
[typesense-compatibility.md](typesense-compatibility.md) for the Typesense 30.2
implementation audit and fuzzy-v2 comparison.
