# Raw inverted-index experiment

`youtube_raw_search` is a read-only search backend implemented without Tantivy,
Lucene, Typesense, or another search library. It exposes both the existing
Typesense-shaped route and an explicit raw route:

- `GET /collections/{collection}/documents/search`
- `GET /raw/collections/{collection}/documents/search`
- `GET /health`
- `GET /metrics`

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
   inside this guarded path.
5. Traverse the compact trie for frequency-ranked prefix candidates only when
   exact coverage remains low.
6. Expand progressively to one and then two Damerau-Levenshtein edits using
   the active Typesense length gates. A final compact-token fallback may pair
   one exact split component with one one-edit component in the same chunk.
7. Keep every multi-token expansion conjunctive. If no result remains, drop
   suffix tokens one at a time, then prefix tokens, stopping at the longest
   filter-visible subset. Each ladder is capped at 20 candidates.
8. Rank by retained-token count, a packed exact-token mask, BM25/title score,
   and first-token position. The mask compares exact-token count first and
   earlier retained query tokens second while sharing the existing 8-byte hot
   candidate rather than allocating per-hit provenance.
9. Apply an explicit date, engagement, or bounded relevance-plus-engagement
   order when requested.
10. Group scored chunks by video and retain only the requested top moments.
11. Decompress and parse document blocks only for returned hits.

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

## Deployment and rollback

The isolated Fly app uses [fly.raw-search-experiment.toml](../../../fly.raw-search-experiment.toml),
one shared CPU, 256 MB RAM, authentication, zero warm machines, and suspend on
idle. Schema v5 is deployed at <https://ytsearch-raw-experiment.fly.dev>; the
frontend/API facade is <https://search.experimental.pronsh.dev>.
The Raw API deployment sets `YTSEARCH_PRESERVE_BACKEND_RELEVANCE_ORDER=true`
so the compatibility layer does not replace the engine's exact-token order;
the Typesense default remains unchanged.

```sh
fly deploy --config fly.raw-search-experiment.toml --ha=false
fly status --config fly.raw-search-experiment.toml
```

Experimental rollback keeps the schema-v4 binary/index generation together;
production rollback remains leaving `TYPESENSE_API_URL` pointed at Typesense.
Do not resize or delete production Typesense for this experiment.

See [raw-validation.md](raw-validation.md) for the real-corpus results and
[typesense-compatibility.md](typesense-compatibility.md) for the Typesense 30.2
implementation audit and fuzzy-v2 comparison.
