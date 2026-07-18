# Typesense memory optimization

The transcript collection stores about 6.89 million chunks. Before this change,
all document fields were indexed in memory, including duplicated previous/next
transcript context and display-only metadata. The live baseline on 2026-07-18
was 3,950 MB resident and 3,693 MB allocated on an 8 GB Machine.

Typesense recommends indexing only fields used for search, filtering, faceting,
grouping, or sorting. Unindexed values remain stored on disk and are still
returned with search results. The lean schema retains in-memory indexes for:

- `text` and `title` search;
- creator and video filters/grouping;
- `duration` filtering;
- numeric publication sorting.

Display-only context, thumbnails, chapters, timestamp values, and redundant
date/identity fields use `index:false`. This particularly avoids building full
token indexes for `prev_text` and `next_text`, which duplicate neighboring
transcript content.

After all writers are stopped, apply the guarded migration:

```sh
ALLOW_SCHEMA_MEMORY_OPTIMIZATION=1 scripts/optimize-typesense-memory.sh
```

The script refuses to run alongside a local index process, changes one field at
a time, checks health between fields, and prints before/after resident memory.
Reads remain available during Typesense schema changes, but writes block until
each field finishes.

## Measured result and the larger lever

A controlled production alteration of `prev_text` reduced resident memory from
3,950 MB to 3,902 MB: 48 MB, or about 1.2%. `channel_id` produced no measurable
change. The remaining display-only changes are still correct schema hygiene,
but should not be presented as a significant RAM reduction.

The dominant opportunity is document count. Every transcript chunk repeats a
video's title and filter metadata, and searchable title tokens currently get a
posting for every chunk. A measurement over 498 real cached transcripts, using
the same grouping algorithm as `youtube_ingest::Chunker`, produced:

| Window | Chunks | Change from 10s | Mean words/chunk |
| ---: | ---: | ---: | ---: |
| 10s | 145,254 | baseline | 19 |
| 15s | 84,310 | -42.0% | 33 |
| 20s | 59,538 | -59.0% | 46 |
| 30s | 37,614 | -74.1% | 73 |

A 20-second window is the best next candidate: it removes about 59% of chunk
documents in this corpus while keeping snippets compact enough for the current
UI. This is a document-count estimate, not a claim of 59% lower RAM: Typesense
must be benchmarked on an isolated collection to measure actual postings,
latency, and restart peak.

Do not change the default for incremental updates. Mixing chunk sizes would
leave existing videos unchanged and make behavior inconsistent. Validate a
20-second collection with the frozen low-signal, stopword, creator-filter, and
representative title/transcript query set; then perform an explicitly approved
rewipe if relevance, latency, and memory all pass. A separate one-document-per-
video title collection is a further option if repeated title postings remain a
large share of memory.

Do not resize the Fly Machine based only on steady-state resident memory. Keep
at least 15% headroom and measure restart-time peak memory, recovery duration,
HTTP 503 exposure, write behavior, and the frozen search benchmark first. A
single-node restart rebuilds in-memory indexes and temporarily returns 503, so
the schema change should be measured before selecting a smaller RAM tier.

References:

- [Typesense system requirements](https://typesense.org/docs/guide/system-requirements.html)
- [Production schema guidance](https://typesense.org/docs/guide/running-in-production.html)
- [Collection schema alteration](https://typesense.org/docs/30.2/api/collections.html#update-or-alter-a-collection)
