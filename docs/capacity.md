# Capacity

What limits growth, measured on 2026-10-07 unless dated otherwise. Update the
numbers when you remeasure; projections assume linear growth from them.

## Current load

| Quantity | Value | Source |
| --- | --- | --- |
| Searches | 1,223 in 30 days (about 41 a day); API p50 29 ms, p95 269 ms | Axiom `search_query` events |
| Corpus | 9.37M documents, 69 creators | production `out_of`, `/api/youtubers` |
| Index size | 1.7 GB for 8.14M documents: about 210 B per document, 52 KB per video, 28 MB per creator | local full index |
| Reader | 4 shared vCPUs, 2.5 GiB; RSS about 430 MB, index memory-mapped | `fly.raw-search.toml`, [reader playbook](playbooks/reader-performance.md) |
| Publication | rebuilds the whole generation: 71 minutes for 9.37M documents on the queue worker's 2 shared vCPUs (2026-10-07; 70–80 s on a dedicated builder in the schema-6 era) | queue worker logs |
| Cold start | up to 193 s to checksum and open a generation (2 GB canary, 2026-08-31) | [release gate](operations/raw-publication-release.md) |

## Search traffic: ample headroom, two ceilings

- The reader runs `RAW_SEARCH_MAX_CONCURRENT_SEARCHES` backend searches at
  once (production: 2; the code default is 1, set after a broad query ran a
  2 GB reader out of memory before searches used bounded per-video slots). On
  the full index with 4 cores and 4 parallel clients (2026-10-08): 61 req/s at
  1, 108 at 2, 123 at 4, with peak anonymous memory of 154, 218 and 273 MB; a
  broad search adds about 40 MB. Beyond the limit requests queue.
- Shared Fly vCPUs guarantee 6.25% of a core each
  ([Fly CPU performance](https://fly.io/docs/machines/cpu-performance/)), so the
  reader sustains about 250 ms of CPU per second: roughly 1.8 searches per second
  once the burst balance saved while idle is spent.
- The Worker's 60-second edge cache answers repeated searches without the
  reader, and the API (0.2 ms of processing per search) is not a bottleneck.

Order of changes when traffic grows: raise the concurrency limit toward 4
(the 2.5 GiB budget has room); then move the reader to performance vCPUs, which
removes the baseline quota.

## Corpus growth: the real limits

Each limit grows linearly with the index (about 28 MB per average creator):

| Limit | Now | Becomes a problem near |
| --- | --- | --- |
| Reader page cache: the index should stay resident beside about 0.5 GB of process memory, and activation briefly holds two generations | 1.7 GB in 2.5 GiB | about 1.5x the corpus; raise reader memory with the index |
| Publication rebuilds everything, so its time and builder memory scale with the corpus; on shared vCPUs the 6.25% baseline throttles it once the burst balance is spent | 71 minutes on the queue worker | already: new videos take over an hour to become searchable, and a 10x corpus would take many hours |
| Volume: active, rollback and staging generations plus the WAL (the base index is in the image) | 6.1 GB of 20 GB | about 3x the corpus |
| Cold start checksums every generation file before serving | up to 193 s of downtime per restart | already the longest outage a release causes |

The architectural fixes, in order of value:

1. Incremental publication: publish a small delta generation for new videos and
   merge into the base periodically, so publishing costs scale with the change,
   not the corpus.
2. Verify generation checksums once at publication and trust the verified
   marker at start, so a restart does not rescan every file.
3. A second, read-only reader replica serving the same immutable generations, so
   a restart or release has no downtime.

## Indexing throughput

Adding creators is limited by YouTube, not by the code: one queue worker with
`JOBS=1`, paced caption fetches, and bot challenges that stop a pass (see
[indexing operations](indexing-operations-guide.md#fetch-failures)). The indexer
overlay upgrades `yt-dlp` on every build, so rebuild it when YouTube breaks
fetching.

## What stays cheap

The Cloudflare Worker and static frontend scale with Cloudflare. The JSONL
request queue rewrites its file per change but is capped at 10,000 rows.
