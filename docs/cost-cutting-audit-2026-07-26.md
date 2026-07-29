# ytsearch Cost-Cutting Audit

Date: 2026-07-26

This audit prioritizes savings that preserve the current Typesense search
behavior and North American latency. Prices are before tax and should be
confirmed against the actual Fly and Cloudflare billing dashboards.

## Current provisioned inventory

The live inventory contains:

- One running Fly `performance-4x` Typesense Machine with 8 GB RAM.
- One stopped cold-benchmark `performance-4x` Machine.
- One always-warm `shared-cpu-1x` API Machine with 512 MB RAM.
- One stopped on-demand `shared-cpu-4x` indexer with 8 GB RAM.
- Six Fly volumes totaling 61 GB.
- Three unattached 10 GB indexer volumes.
- One 10 GB cold-benchmark volume attached to the stopped benchmark Machine.

Fly charges $0.15/GB-month for provisioned volumes even when they are
unattached or their Machines are stopped. The four non-production 10 GB
volumes therefore represent approximately **$6/month** before snapshot
storage. Do not delete them until their contents and restore value have been
verified.

Sources:

- <https://fly.io/docs/about/pricing/>
- <https://fly.io/docs/about/cost-management/>

## Recommended order

### 1. Finish scheduled availability

The production SJC `performance-4x`/8 GB price is approximately $147.85/month.
Running from 11:45 AM through midnight uses roughly 51% of full-month compute,
reducing that component to approximately **$75/month**. Indexing that continues
after midnight will increase the actual number slightly.

Expected saving: **about $72/month**.

### 2. Remove verified unused Fly resources

After confirming they contain no unique transcript cache or benchmark artifact:

- Remove the three unattached 10 GB indexer volumes.
- Remove the stopped cold benchmark Machine and its 10 GB volume, or retain it
  only until the scheduled-start rollout has completed.

Expected volume saving: **about $6/month**, plus stopped-Machine rootfs and
snapshot storage. Fly charges stopped Machines $0.15 per GB of rootfs per
month, so the exact extra saving depends on image size.

### 3. Benchmark cheaper CPU shapes without reducing RAM

The index currently needs RAM more than it needs four dedicated CPUs. Current
SJC list prices for 8 GB configurations are approximately:

| Fly shape | Always warm | 12.25-hour schedule | Change from scheduled `performance-4x` |
| --- | ---: | ---: | ---: |
| `performance-4x` | $147.85 | $75.46 | baseline |
| `performance-2x` | $97.78 | $49.88 | save about $25.58 |
| `performance-1x` | $72.75 | $37.12 | save about $38.34 |
| `shared-cpu-4x` | $51.02 | $26.03 | save about $49.43 |

The shared shape is not automatically safe. Fly gives shared CPUs a 6.25%
baseline quota per vCPU and allows bursting; sustained indexing or a burst of
concurrent searches can exhaust that balance and increase latency. Test the
stopped benchmark node with the frozen relevance and concurrency suite before
changing production.

Pass thresholds remain:

- API-path warm p95 below 250 ms.
- No 5xx responses during bounded concurrent search/import.
- Cold readiness below three minutes.
- No relevance regression.

Sources:

- <https://fly.io/docs/about/pricing/>
- <https://fly.io/docs/machines/cpu-performance/>

### 4. Buy Fly reservation blocks only after sizing stabilizes

Fly reservation blocks discount eligible compute by 40% in exchange for annual
prepayment. Performance blocks cost $144/year and provide $20 of credit each
month. Credits do not roll over.

If scheduled `performance-4x` usage settles near $75/month, three blocks would
cost an effective $36/month and cover $60 of usage; the remaining approximate
$15 stays on demand. That produces a rough **$24/month additional saving**.

Do this only after at least a full billing cycle establishes actual scheduled
runtime. A CPU-shape change may alter which reservation class is useful.

Source: <https://fly.io/docs/about/pricing/#machine-reservation-blocks>

### 5. Reduce snapshot duplication

The serving Typesense and API queue volumes currently retain 30 daily Fly
snapshots while the design also calls for provider-neutral R2 and B2 backups.
Fly snapshot storage costs $0.08/GB-month after the first 10 GB, based on
incremental stored data.

Once both object-storage restores pass:

- Keep 5–7 days of Fly snapshots for fast recovery instead of 30.
- Preserve the longer daily/weekly/monthly policy in R2 and B2.
- Keep R2 Standard within its 10 GB-month free tier when possible.
- Use B2 for the second copy; its first 10 GB is free and additional storage is
  $0.00695/GB-month.

This saving is probably small today but grows with corpus churn.

Sources:

- <https://fly.io/docs/volumes/snapshots/>
- <https://developers.cloudflare.com/r2/pricing/>
- <https://www.backblaze.com/cloud-storage/pricing>

### 6. Check whether Cloudflare Paid is still necessary

Cloudflare Queues became available on Workers Free in 2026. The free account
allowances include:

- 100,000 Worker requests per day.
- 10,000 Queue operations per day.
- Static asset requests are free and unlimited.

The five-minute scheduler adds only 288 Cron invocations daily. If all account
traffic remains inside the free limits and no other project needs Paid-only
features, downgrading could save the **$5/month** Workers Paid minimum. Queue
retention would fall to 24 hours, but ytsearch uses the Fly JSONL queue as its
authoritative request store, so the Cloudflare queue is only a wake signal.

Confirm account-wide usage first; the limit is shared with other Workers.

Sources:

- <https://developers.cloudflare.com/workers/platform/pricing/>
- <https://developers.cloudflare.com/queues/platform/pricing/>

### 7. Keep the API warm unless its role is redesigned

The 512 MB API costs approximately $3.32/month. Stopping it would break
off-hours index requests because it owns the JSONL queue volume. Moving queue
ownership to D1 or a Durable Object could make the Rust API stateless and
auto-stoppable, but the maximum gross saving is only about $3/month.

Do not perform this migration solely for cost. It becomes worthwhile if API
horizontal scaling or queue reliability is independently required.

### 8. Cache only demonstrably repeated searches

Typesense already uses a 60-second internal query cache. A short Cloudflare
cache for anonymous GET searches can reduce CPU and Fly egress when queries
repeat, but transcript searches may have low cache reuse. Measure hit ratio
before adding complexity, and never cache admin or index-request endpoints.

North American Fly egress is $0.02/GB. This is unlikely to be a major line item
at current traffic.

## Trusted-provider alternatives

If moving away from Fly becomes desirable:

- AWS Lightsail offers 8 GB RAM, 2 vCPUs, 160 GB SSD, and 5 TB transfer for
  **$44/month** with public IPv4. It is a familiar provider and much cheaper
  than the current always-warm Fly node, but Lightsail is burstable and does
  not provide Fly-style stop/start billing savings.
- DigitalOcean Basic offers 8 GB RAM, 4 shared vCPUs, 160 GB SSD, and 5 TB
  transfer for **$48/month**. It is simple and popular, but a powered-off
  Droplet continues to reserve billable resources.
- Hetzner's June 2026 US CPX31 price is **$73.49/month**, so it is no longer the
  obvious value choice for this US-oriented workload.

AWS Lightsail is the strongest popular-provider alternative if predictable
always-on pricing matters more than scheduled compute billing. Within Fly,
scheduled `shared-cpu-4x` is potentially cheaper, but only if its benchmark
passes.

Sources:

- <https://docs.aws.amazon.com/lightsail/latest/userguide/amazon-lightsail-bundles.html>
- <https://www.digitalocean.com/pricing/droplets>
- <https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/>

## Realistic totals

| Configuration | Approximate predictable monthly floor |
| --- | ---: |
| Current always-warm search node + API + all volumes | about $160 before indexing and snapshots |
| Scheduled current CPU + API + required volumes | about $82–85 |
| Scheduled `performance-2x` + cleanup | about $56–60 |
| Scheduled `shared-cpu-4x` + cleanup | about $32–36 |
| Scheduled current CPU + reservation blocks + cleanup | about $57–61 |

The best low-risk target is scheduled `performance-2x` after benchmarking,
unused-volume cleanup, and shorter Fly snapshot retention after restore drills.
The aggressive target is scheduled `shared-cpu-4x`, but it must earn promotion
through latency and concurrency tests.
