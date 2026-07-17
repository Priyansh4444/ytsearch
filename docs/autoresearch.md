# Autoresearch for ytsearch

Karpathy's autoresearch fixes the evaluation workload and wall-clock budget,
allows one constrained implementation surface to change, logs every result,
and retains only improvements. ytsearch applies the same discipline with
separate tracks because latency, relevance, indexing reliability, and cost are
not interchangeable.

The executable protocol is in [`../autoresearch/program.md`](../autoresearch/program.md).

## 2026-07-12 baseline

The baseline ran all repository tests, a 10,000-chunk/10-round serialization
microbenchmark, and four live search cases with five repetitions each.

| Search case | Average | p95 | Errors | Stable counts |
| --- | ---: | ---: | ---: | --- |
| `fable on high`, `@t3dotgg` | 103 ms | 121 ms | 0 | yes |
| `battery gate incident` | 282 ms | 359 ms | 0 | yes |
| `linux exploit` | 122 ms | 168 ms | 0 | yes |
| `I`, `@t3dotgg` | 141 ms | 185 ms | 0 | yes |

The representative protobuf payload was 365.6 bytes versus 684.3 bytes for
JSON, a 0.534 size ratio. In the recorded baseline, protobuf encode/decode was
2.90/5.24 ms per round and JSON was 6.26/5.91 ms. A separate warm run produced
materially different codec timings, so repeat distributions are required.

This proves protobuf is useful for compact internal batch transport when both
ends already use the schema. It does not prove that replacing the public JSON
API improves end-to-end search: browser compatibility, compression, Typesense
query time, API processing, and network latency dominate that decision.

## Experiment queue

1. **Search response work:** measure gzip on/off, response bytes, API processing,
   and p95 for the frozen query cases. Keep JSON for browsers.
2. **Typesense query plan:** use the same collection snapshot to test grouped
   counts, facet counts, and cached queries independently. Ranking fixtures and
   result cardinality are promotion gates.
3. **API machine size:** deploy the API image to an isolated app on shared and
   performance CPU classes. Replay the same requests, including cold starts,
   and compare p95 and cost per 10,000 requests.
4. **Indexer machine size:** replay a fixed cached transcript fixture with no
   import on candidate CPU/RAM classes. Compare successful videos/minute and
   peak memory. Do not use live YouTube responses as the primary benchmark.
5. **Internal protobuf:** benchmark a real chunk batch through encode, transfer,
   decode, and Typesense JSONL conversion. Keep only if total CPU or transfer
   cost improves enough to pay for schema/versioning complexity.
6. **Warmness:** compare always-on, stop/autostart, and suspend/autostart API
   candidates. Background indexers must never use proxy-driven autostop because
   Fly can stop them after the triggering request closes.

## Kept experiment: browser gzip

On 2026-07-15, 20 alternating production request pairs for `linux exploit` were
run against the same corpus. Identity responses averaged 90,461 bytes, 479 ms
overall, 338 ms p50, and 980 ms p95. Gzip responses averaged 17,351 bytes,
263 ms overall, 212 ms p50, and 566 ms p95. Both modes returned 131 videos and
583 moments with zero errors and stable counts. Gzip cut transfer by 80.8% and
was faster at every summarized latency measure, so it remains enabled.

## Fly isolation contract

An infrastructure experiment must use a new app name and, when stateful, a new
volume seeded from a fixed fixture. Candidate Machines use `auto_destroy` where
possible, are disconnected from production routing, and have an explicit
cleanup trap. The runner must require `ALLOW_FLY_EXPERIMENT=1`, `MAX_COST_USD`,
and `TTL_MINUTES`; absence of any value is a hard failure.

Do not clone the active indexer as an experiment: it would inherit queue
credentials and could race the persistent worker. Do not attach another
Typesense process to its active volume.
