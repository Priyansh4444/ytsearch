# Cost Optimization Without Making Search Slow

Last audited: 2026-07-14. Prices vary by region and can change; verify the
linked provider pages before resizing or migrating.

Concrete scale-to-zero, practical economy, warm concurrent, and three-node HA
budgets are in
[cold-warm-concurrency-costs-2026-07-17.md](cold-warm-concurrency-costs-2026-07-17.md).

The Typesense-era RAM reduction and display-field migration records were retired
with the engine that needed them; recover them from git history
(`git log -- docs/typesense-memory-optimization.md`) if they are ever relevant.

## Current Production Shape

| Service | Current shape | Runtime policy | Main constraint |
| --- | --- | --- | --- |
| Cloudflare Worker | Static assets, API proxy, queue producer/consumer | Edge managed | Free-tier request limits |
| Fly API | shared CPU, 1 vCPU, 512 MB | Always on | Public API and JSONL queue owner |
| Fly Typesense | performance CPU, 4 vCPU, 8 GB | Always on | 3.9 GB resident keyword index |
| Fly indexer | shared CPU, 4 vCPU, 8 GB | Queue-woken; stops after five idle minutes | `yt-dlp` peak memory |
| Attached Fly volumes | 1 GB API + 10 GB Typesense + 10 GB indexer | Always billed | Local persistence |

Typesense is the dominant bill. Its documentation explains why: the entire
search index is held in memory, normally requiring roughly two to three times
the searchable source data. Fly performance CPUs receive a continuous CPU
quota; shared CPUs receive a small baseline with burst credit. This is why the
live shared-CPU Typesense experiment recovered too slowly even though it had
the same 8 GB RAM.

Sources: [Fly pricing](https://fly.io/docs/about/pricing/),
[Fly CPU quotas](https://fly.io/docs/machines/cpu-performance/), and
[Typesense system requirements](https://typesense.org/docs/guide/system-requirements.html).

## Do These First

### 1. Delete verified orphan volumes

The audit found three unattached legacy 10 GB indexer volumes:

- `vol_v8e6wk3g1l6kj15v`
- `vol_458zq611m0yppz14`
- `vol_4qlzgynglnz786gr`

Fly bills volumes even when unattached. At $0.15/GB-month, deleting these after
one final content/snapshot check saves about **$4.50/month**. This is deliberately
not automated because volume deletion is irreversible. The attached volumes
should remain.

### 2. Keep the indexer on demand

The current queue-wake design is correct. A stopped Fly Machine does not incur
running CPU/RAM charges, while its 10 GB cache volume costs about $1.50/month.
Do not replace this with an always-on worker. Keep one writer, five-minute idle
exit, and persistent cache; those choices avoid both duplicate work and repeated
YouTube downloads.

### 3. Keep the API separate

The API costs only a few dollars per month at shared-1x/512 MB. Combining it
with Typesense would save little while making API availability depend on a
large Typesense restart and giving both processes the same failure boundary.

### 4. Keep Cloudflare at the edge

Static assets, gzip/Brotli delivery, the API proxy, and queue wake messages are
good edge workloads. Cache safe GET responses for their existing short TTLs.
Do not cache indexing mutations or status responses. Monitor the current
[Workers](https://developers.cloudflare.com/workers/platform/pricing/) and
[static-assets](https://developers.cloudflare.com/workers/static-assets/billing-and-limitations/)
free-tier limits before traffic grows.

## Largest Safe Experiment: Fewer Dedicated Typesense CPUs

Memory cannot currently fall below 8 GB safely: steady resident memory is about
3.9 GB, and startup/indexing needs headroom. CPU and RAM should therefore be
tested independently.

1. Fork the Typesense volume.
2. Start an isolated `performance-2x` Machine with 8 GB RAM in `sjc`.
3. Replay the fixed query suite for warm p50/p95, cold recovery, and concurrent
   indexing/search latency.
4. Keep it only if warm p95 stays below 250 ms, cold readiness below 3 minutes,
   there are no 5xx responses, and corpus counts remain identical.
5. Destroy the candidate Machine and fork immediately after the test.

This has a much better chance than shared CPU because performance vCPUs retain
continuous scheduling. Never resize the only production Typesense Machine for
the experiment; the previous live resize caused avoidable downtime.

## Reduce Typesense Memory Before Reducing RAM

To reach a cheaper 4 GB machine without losing speed, first reduce indexed data:

1. Measure per-field index size on a volume fork.
2. Keep only `text` and `title` searchable. Metadata used only for display should
   remain stored but not indexed or faceted.
3. Test 8–10 second transcript chunks against the judged relevance suite. Fewer
   documents can substantially reduce repeated title/channel metadata and index
   overhead, but timestamp precision and recall are release gates.
4. Remove unused optional fields only through a versioned collection migration.
5. Promote using an alias after count, relevance, and latency checks; never
   rewrite the sole production collection in place.

## Alternative Hosting With the Largest Dollar Saving

A single 8 GB NVMe VPS can run Typesense, the API, and the on-demand indexer for
far less than a Fly performance Machine. Hetzner currently lists shared 8 GB
instances in the low tens of dollars per month depending on architecture and
location. See [current Hetzner pricing](https://www.hetzner.com/cloud/) and its
[June 2026 adjustment](https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/).

Trade-offs:

- You own OS patching, firewalling, monitoring, backups, and process recovery.
- One VPS is one failure domain unless a second node is added.
- Moving away from `sjc` changes latency for North American users.
- ARM can be cheaper, but every Rust image, Typesense image, `yt-dlp`, Deno, and
  FFmpeg path must be built and tested for arm64 first.

The sensible migration is a 7-day shadow server fed from a restored snapshot,
not an immediate DNS switch.

## Protobuf Decision

Do **not** replace the browser API JSON with protobuf now.

- The existing benchmark found the public response dominated by search/network
  time, and gzip already reduced a representative response by roughly 8x.
- Browsers would need a schema runtime and compatibility layer, increasing the
  JavaScript bundle and operational complexity.
- Cloudflare-to-Fly and Fly-to-Typesense traffic is HTTP JSON; changing only the
  public envelope does not reduce Typesense memory or compute.
- Queue wake messages are tiny, so protobuf cannot create meaningful savings.

Protobuf becomes worthwhile only for a future bulk binary pipeline—for example,
shipping large cached transcript batches between independent indexer and ingest
services. Keep the existing benchmark and require at least a 20% end-to-end cost
or throughput improvement before adopting it.

## Monthly Review Checklist

- Run `DETAILS=1 scripts/check-staging-machines.sh`.
- List attached and unattached volumes for every Fly app.
- Record Typesense RSS, disk usage, warm p50/p95, and cold readiness.
- Record indexer active hours rather than its theoretical monthly price.
- Check Cloudflare Worker and Queue usage against free-tier limits.
- Review outbound transfer and snapshot retention.
- Re-run the bundle and interaction performance budget.
