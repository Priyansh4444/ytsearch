# Cold, warm, and concurrent deployment costs

Date: 2026-07-17. Currency: USD. Region: Fly `sjc`.

## Recommendation

Use the **practical economy** shape now: Cloudflare stays at the edge, the Rust
API may stop or suspend when idle, the indexer remains queue-woken, and one
Typesense node stays warm. A fully stopped Typesense node is suitable only for a
demo or private installation where the first user accepts a potentially long
wait while roughly 3.9 GB of index state is restored into memory.

For sustained multi-user traffic, first separate the JSONL queue owner from the
stateless search API. Then run two or more API Machines behind Fly Proxy. Scale
Typesense vertically for throughput, or use a three-node cluster when both read
scaling and one-node failure tolerance justify roughly $467/month.

No infrastructure was resized or deployed for this analysis.

## Price assumptions

Current official `sjc` prices used here:

| Resource | Hourly | 30-day month |
| --- | ---: | ---: |
| shared 1 CPU, 512 MB | $0.0053 | $3.81 |
| shared 4 CPU, 8 GB | $0.0709 | $51.02 |
| performance 2 CPU, 4 GB | $0.1027 | $73.92 |
| performance 2 CPU, 8 GB | $0.1358 | $97.78 |
| performance 4 CPU, 8 GB | $0.2053 | $147.85 |
| Fly volume | — | $0.15/GB |
| stopped Machine root filesystem | — | $0.15/GB of rootfs |
| snapshot storage | — | first 10 GB free, then $0.08/GB stored |
| Cloudflare Workers Free | — | $0; 100,000 requests/day |
| Cloudflare Workers Paid | — | $5 minimum; 10M requests/month included |

Fly bills running compute by the second. Stopped or suspended Machines do not
incur CPU/RAM charges, but their root filesystem, volumes, snapshots, and
network usage remain billable. Cloudflare static asset requests are free and
unlimited; Worker/Pages Function requests use the Workers allowance.

Sources: [Fly resource pricing](https://fly.io/docs/about/pricing/),
[Fly autostop/autostart](https://fly.io/docs/launch/autostop-autostart/), and
[Cloudflare Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/).
Prices vary over time and invoice details such as egress and stored snapshot
bytes are not predictable from repository configuration.

## Current live shape

The read-only machine inventory on 2026-07-17 showed:

| Component | Machine | State |
| --- | --- | --- |
| API and JSONL queue owner | shared 1 CPU, 512 MB | running |
| Typesense | performance 4 CPU, 8 GB | running |
| Indexer | shared 4 CPU, 8 GB | stopped, queue-woken |

Provisioned volumes are 1 GB for the API queue, 10 GB for Typesense, and 10 GB
for the indexer cache: 21 GB or $3.15/month.

The predictable current floor is therefore about:

```text
Typesense       147.85
API               3.81
volumes            3.15
Cloudflare      0 or 5.00
-----------------------
base total    154.81 or 159.81 / month
```

Add indexer running time at $0.0709/hour, stopped rootfs, snapshots beyond the
free 10 GB, egress, Axiom, and any taxes. For example, 20 indexer hours add
about $1.42; running that Machine continuously would add $51.02 and is not
recommended.

## Option A: true scale-to-zero

```text
Cloudflare Pages/Worker
        |
        v
stopped API ---- first request starts it
        |
        v
stopped Typesense ---- loads the in-memory index from its volume

queue request -> starts API/queue owner -> wakes one indexer when approved
```

Configuration concept for both HTTP services:

```toml
[http_service]
  auto_stop_machines = "stop" # test "suspend" separately
  auto_start_machines = true
  min_machines_running = 0
```

Idle cost is approximately $3.15 for the three volumes, plus stopped rootfs and
snapshot storage: usually a low-single-digit monthly bill before Cloudflare.
Running cost is usage based:

```text
monthly compute = Typesense active hours * $0.2053
                + API active hours       * $0.0053
                + indexer active hours   * $0.0709
```

Example with each search service active for 30 hours and the indexer active for
10 hours: $6.16 + $0.16 + $0.71 + $3.15 volumes = about **$10.18**, before
rootfs, snapshots, egress, observability, and an optional $5 Workers plan.

The catch is user experience. Stopping Typesense discards its in-memory state.
It must boot and reload/rebuild the resident index before health checks pass.
The current cost audit observed about 3.9 GB resident memory and already uses an
8 GB Machine for recovery headroom. The cold request can exceed normal HTTP
timeouts and a burst of users cannot make that single recovery faster.

Required product behavior for this option:

- Cloudflare returns `202 Search is waking` with a retry token instead of
  holding an ordinary search request indefinitely.
- A readiness endpoint distinguishes `starting`, `loading index`, and `ready`.
- Only one wake operation is issued; concurrent cold requests wait on the same
  readiness state rather than starting duplicate databases.
- Cloudflare serves cached recent searches during recovery where safe.
- Measure `stop` and `suspend`. Suspend resumes faster because it preserves
  memory state, but clock/network behavior must be tested and billing savings
  are the same as stopped compute.

This option is cheapest but is not recommended for a public interactive search
default.

## Option B: practical economy

Keep one Typesense node warm and allow only stateless HTTP capacity and the
indexer to sleep:

```text
Cloudflare -> API (0 or 1 warm; one stopped spare) -> warm Typesense
                    |
                    +-> dedicated queue owner -> on-demand indexer
```

Two possible search-node prices require isolated testing:

| Search node | API policy | Approximate base/month | Risk |
| --- | --- | ---: | --- |
| performance 4 CPU / 8 GB | API scales to zero | $151.00 + rootfs/CF | Current search capacity; only API cold starts |
| performance 2 CPU / 8 GB | API scales to zero | $100.93 + rootfs/CF | Saves about $50/month, but must pass cold recovery, indexing, and concurrent search tests |

The totals include $3.15 of current volumes but exclude active indexer hours.
The 2-CPU candidate is an experiment, not a safe resize recommendation. The
existing cost plan requires a forked volume and p95, recovery, error, and corpus
parity gates before promotion.

This is the best default balance for the current corpus: normal users never wait
for the large index to load, while infrequent API/indexing compute can stop.

## Option C: warm concurrent service

Before horizontally scaling the API, remove its local JSONL queue responsibility:

```text
                         +-> stateless API 1 --+
Cloudflare/Fly Proxy ----+-> stateless API 2 --+--> Typesense tier
                         +-> stopped API spare +

index submissions -> one dedicated queue API/volume -> one indexer writer
```

The queue volume is local and single-writer. Multiple current API replicas
cannot safely share it, and Fly volumes are not a shared network filesystem.
Either split search and queue processes into separate apps or replace JSONL
with a concurrency-safe managed store. Keep one queue consumer until that
storage model changes.

A modest warm shape would be:

- two shared-1x/512 MB stateless API Machines: $7.62/month;
- one performance-4x/8 GB Typesense node: $147.85/month;
- one shared-1x/512 MB queue owner: $3.81/month;
- current 21 GB volumes: $3.15/month;
- Cloudflare Paid: $5/month;
- stopped indexer plus active hours.

Base: about **$167.43/month**, plus rootfs, snapshots, egress, observability, and
indexer hours. This improves API availability and concurrent request handling,
but Typesense remains a single failure domain and throughput ceiling.

Suggested Fly API policy after load testing:

```toml
[http_service]
  auto_stop_machines = "suspend"
  auto_start_machines = true
  min_machines_running = 2

  [http_service.concurrency]
    type = "requests"
    soft_limit = 20 # placeholder; derive from a load test
    hard_limit = 40 # placeholder; derive from latency/error saturation
```

Fly uses the soft limit to route requests away from a busy Machine and to start
existing stopped Machines. It never creates new Machines automatically; create
the desired maximum number of spares beforehand or operate a metrics-based
autoscaler. Do not copy the placeholder limits into production without tests.

## Option D: expensive warm and highly available

For a service that tolerates one Typesense node failure and scales reads:

- three performance-4x/8 GB Typesense nodes: $443.55/month;
- two shared-1x/512 MB API nodes: $7.62/month;
- one shared-1x/512 MB queue owner: $3.81/month;
- three 10 GB Typesense volumes plus 1 GB queue and 10 GB cache: $6.15/month;
- Cloudflare Paid: $5/month.

Base: about **$466.13/month**, plus active indexer time, rootfs, snapshots,
egress, and observability.

Typesense uses Raft and needs three nodes to tolerate one failure. The complete
dataset is replicated to each node; reads can be served independently, while
writes are forwarded to the leader. This is real HA, not merely three unrelated
copies. See [Typesense high availability](https://typesense.org/docs/guide/high-availability.html).

Place nodes across failure domains only after measuring added replication and
user latency. A three-node cluster is not cost-effective merely to handle a
small burst; vertical scaling or one warm node plus caching is simpler.

## Concurrent-user behavior

The Rust/Axum API is asynchronous and can have multiple searches in flight.
The practical limits are Typesense CPU, response processing, payload size,
network connections, and the current default per-client rate limit of 120
searches/minute—not a one-user-at-a-time lock.

However, current in-process rate-limit state is local to each API replica. With
two API Machines, the effective limit can vary as requests are balanced. Move
abuse protection to Cloudflare or a shared coordination store before relying on
it as a strict global policy.

Concurrency must be capacity-tested, not inferred from CPU count. Run the fixed
query mix at 1, 5, 10, 25, and 50 concurrent users, with unique and repeated
queries, and record:

- request rate and completed searches/second;
- p50/p95/p99 end-to-end and backend latency;
- 429, 5xx, timeout, and connection errors;
- Typesense CPU/RSS and API CPU/RSS;
- returned video/moment counts and ranking fixture correctness;
- cold-start queue time separately from ready-service latency.

Promotion gates should include zero 5xx/timeouts at the target concurrency,
stable result counts, p95 below the chosen product budget, and at least 30%
headroom before the observed saturation point. The current single-request
benchmarks cannot honestly establish a concurrent-user capacity number.

## Optional semantic and frame-search cost

Semantic/frame retrieval is still experimental and is intentionally excluded
from every base total above.

Qdrant's planning estimate for in-memory vectors is:

```text
vectors * dimensions * 4 bytes * 1.5
```

At one million 384-dimensional vectors that is about 2.15 GiB before substantial
payloads and operational headroom. A first warm candidate would therefore be at
least performance-2x/4 GB ($73.92/month), with performance-4x/8 GB
($147.85/month) safer for indexing/headroom. Frame-level indexing can create
many more points than transcript chunks, so scene selection and measured point
counts are essential. See [Qdrant capacity planning](https://qdrant.tech/documentation/capacity-planning/).

Local embedding during offline indexing adds no hosting bill when run on owned
hardware, but it adds electricity and operator time. A warm query-embedding
service would add another CPU or accelerator service. Fly's GPU offering is
documented as deprecated after 2026-08-01, so it should not anchor a new cost
plan. Keep local CPU embedding or choose a separately priced inference provider
only after the judged retrieval experiment proves value.

## Decision thresholds

- Choose true cold only for demos/private installs that accept an explicit
  waking screen.
- Choose practical economy for current public traffic.
- Add stateless API replicas when a concurrency test shows API saturation or
  availability requires it; split the queue first.
- Add a three-node Typesense cluster when downtime/read throughput is worth an
  additional roughly $299/month over the current single-node search tier.
- Add Qdrant/model serving only after semantic or visual relevance clears the
  evaluation gate; its likely warm floor adds roughly $74–$185/month depending
  on vector and embedding-service memory.
