# Production search replay (2026-09-26 UTC)

This records how the reader performs on the searches people actually sent,
not only on the synthetic benchmark matrix. Query text is private: the export
and per-row results stay outside Git, and this note keeps aggregates plus
queries already named in `scripts/benchmark-search-cases.tsv`.

## Dataset

`scripts/axiom-export-search-replay.sh` exported every distinct Raw
`search_query` event from Axiom over 90 days as one row per
`(query, youtuber, sort, page)`: 278 rows covering 608 searches. Of these,
120 rows are creator-scoped. Sorts: 224 relevance, 41 newest, 7 views,
2 likes, 2 relevance-with-engagement, 1 oldest, 1 comments.

`scripts/replay-production-searches.sh` sends each row through the real
`youtube-api` binary, so creator normalization, page windows, lookahead
(`limit=21`), highlights, sorts, and multi-call pages match production. The
reader's cumulative `search_nanos` gives server-side time per request. The API
asks the reader for a 60-second response cache, so each measured pass either
restarts the reader or waits out the TTL.

The replay adds what the matrix lacks: real creator, sort, and page
combinations, and first-touch cost. Each row is searched once per pass, so a
fresh reader pays the page faults a repeated benchmark query hides; a fresh
local process took 185 ms for its first `fire`, then 5 ms.

## Findings

- Warm and uncached, the slowest real searches were dense common-word scans
  (`and the`, `the`, `in the`, `this is the`) and sentence queries whose
  phrase check walked every posting of `the`.
- Cold after a restart, the reader was far slower: the startup checksum read
  each file through the cache-evicting reader, so it finished with the whole
  index evicted and the first queries faulted from the Fly volume. A cold
  `fire` took 9,930 ms on the canary.
- Transcript blocks are read as scattered ~5 KB zstd blocks, but default mmap
  read-around paged in far more (327 MB for one 143-block query on a THP host),
  evicting the files the scans need.

## Results

Server-side milliseconds from the reader's metrics. Canary runs use the exact
isolated 2 GiB, two-shared-vCPU Machine and the immutable 8,142,650-document
base; production's active generation holds 8,422,419 documents.

| Build | Where | Pass | p50 | p99 | Max | Over 200 ms |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| `9f63a76` | local, 2 cores | uncached | 6 | 138 | 205 | 1 |
| `5e3a33b` | canary | cold, just started | 15 | 839 | 9,930 | 13 |
| `5e3a33b` | canary | warm, uncached | 9 | 136 | 222 | 1 |
| `95cb6cd` | canary | cold, just started | 14 | 167 | 261 | 2 |
| `95cb6cd` | canary | warm, uncached | 9 | 158 | 235 | 1 |

On the synthetic matrix at `95cb6cd`, every sentence probe stayed below 110 ms
on the canary (`the problem is that` fell from 234 ms at `9f63a76`), but
`and the` still took 228–238 ms and `the` 170–204 ms. Those dense scans are the
remaining gap to 200 ms.

A local warm ruler (75 slowest production and sentence queries, production
request shape, median of five, two pinned cores) fell from 3,220 ms summed at
`9f63a76` to 1,719 ms at `5e3a33b`, with identical response digests. With four
cores instead of two, `0c53dbd` cut the worst query (`and the`) from 146 ms to
89 ms locally.

## Changes behind the numbers

- `61a12ce`: one position walk serves both proximity and ordered-phrase
  evidence.
- `bd08039`: partition planning moved off the open path (21 s on the cold
  production volume).
- `5e3a33b`: lazy posting checkpoints every 64 postings let phrase cursors seek
  instead of decoding whole lists; bounded to 64 MiB.
- `95cb6cd`: the startup checksum keeps the six scanned files cached, a
  background prefetch warms every opened generation, and `documents.blocks` is
  advised random.
- `0c53dbd`: broad scans use one partition per available CPU, up to eight.

Rejected, within host noise or worse on an instruction-count ruler: a BM25
lookup table and 128-posting block decoding. Fat LTO measured 3.5% fewer
cycles and is not adopted.

## Scaling

Schema 6 broad scans are linear in the postings of the query's words. At the
current corpus, two shared vCPUs leave `and the` about 15% over 200 ms; a
corpus twice as large would roughly double it. Extra vCPUs divide the scan
(`0c53dbd`), and the scanned files (~730 MB) must stay resident, so a doubled
corpus also needs more than 2 GiB. Sublinear cost needs the schema-9 skip,
bitmap, and block-max structures in `docs/architecture/lazy-exact/design.md`.
