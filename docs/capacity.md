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
  reader sustains 0.25 cores once the burst balance saved while idle is spent.
  Real queries cost about 17.5 ms of reader CPU each (load test below), so that
  is about 14 searches per second.
- The Worker's 60-second edge cache answers repeated searches without the
  reader, but it does not merge identical searches that arrive together: 100
  simultaneous users sharing 10 queries got 14 cache hits.
- The API allows 120 searches a minute per client IP. Users behind one IP (an
  office or school NAT) share that budget.
- The API (0.25 cores and 75 MB at 119 req/s) is not a bottleneck.

### Production load test (2026-10-10 and 2026-10-13)

100 simulated users replaying random queries from `query-log.txt` with
[`scripts/perf/web-load.ts`](../scripts/perf/web-load.ts) (see the
[load-testing playbook](playbooks/load-testing.md)). Latency is what the client
saw; reader figures are from Fly metrics.

| Run | Rate | OK | p50 | p95 | p99 | Reader |
| --- | --- | --- | --- | --- | --- | --- |
| Website, 100 searches at once | 1 burst | 100/100 | 774 ms | 1.23 s | 1.26 s | — |
| Website, 100 users, 20–40 s think time, 2 min | 3.2 req/s | 245/387; 142 were 429 (one shared IP) | 145 ms | 204 ms | 325 ms | 0.2 cores |
| API direct, per-user IPs, 5–15 s think time, 3 min | 10 req/s | 1,813/1,815 (2 × 502) | 38 ms | 74 ms | 153 ms | 0.2 cores |
| API direct, per-user IPs, no think time, 60 s | 119 req/s | 7,199/7,203 (4 × 502) | 701 ms | 1.33 s | 5.7 s | 2.1 of 4 cores, +140 MB, no throttling |

- At 119 req/s the reader used only the two cores its concurrency limit of 2
  allows, and server time matched client time: requests waited in the reader's
  queue, not on CPU. The burst balance fell about 4% a minute, so that rate
  holds for about 20 minutes before the 0.25-core baseline applies.
- The rare 502s were not load: every one was a logged query with an unclosed
  quote (`"cold start problem`), which Raw rejects; #102 closes the quote.

Order of changes when traffic grows: move the reader to performance vCPUs,
which removes the baseline quota; then raise the concurrency limit toward 4.
The limit stays at 2 until then: 4 adds only about 14% throughput (123 vs 108
req/s locally), takes CPU from publication activation, which already uses most
of its timeout, and changing it restarts the reader.

## Corpus growth: the real limits

Each limit grows linearly with the index (about 28 MB per average creator):

| Limit | Now | Becomes a problem near |
| --- | --- | --- |
| Reader page cache: the index should stay resident beside about 0.5 GB of process memory, and activation briefly holds two generations | 1.7 GB in 2.5 GiB | about 1.5x the corpus; raise reader memory with the index |
| Publication rebuilds everything, so its time and builder memory scale with the corpus; on shared vCPUs the 6.25% baseline throttles it once the burst balance is spent | 71 minutes on the queue worker | already: new videos take over an hour to become searchable, and a 10x corpus would take many hours |
| Volume: active, rollback and staging generations plus the WAL (the base index is in the image) | 6.1 GB of 20 GB | about 3x the corpus |
| Cold start checksums every generation file before serving | up to 193 s of downtime per restart | already the longest outage a release causes |
| Publication disk: the build needs `RAW_PUBLICATION_MIN_FREE_BYTES` (8 GB) free and peaks at about 3x its 2.0 GB output plus the 1.16 GB mutation snapshot, about 7 GB | queue worker: 9.0 GB free on its 10 GB volume, of which the unpruned `transcript-cache` (about 21 KB per video) takes 0.85 GB (2026-10-10) | about 48k more videos, when the cache pushes free space under 8 GB and publication refuses to start; the 10 GB builder volume is empty and passes |
| Activation request: warm-up is capped at 20 s, but opening and swapping take the rest of the 30 s admin timeout (`ADMIN_REQUEST_TIMEOUT`) | 25.6 s for generation 9737 (2026-10-10); 23.0 s to the active pointer for 9894 (2026-10-12) | soon; a timeout leaves the job unactivated until the worker's next hourly retry, which skips the rebuild |

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
