# Engagement-aware Raw ranking

Measured 2026-08-01 on the deployed four-channel Raw experiment. This phase
adds explicit views, likes, comments, and a bounded relevance-plus-engagement
sort without changing the production Typesense backend.

## Product contract

The experimental frontend exposes seven intentionally distinct choices:

| UI label              | API value              | Raw sort                                       |
| --------------------- | ---------------------- | ---------------------------------------------- |
| Relevant              | `relevance`            | Text relevance                                 |
| Relevant + engagement | `relevance_engagement` | Text relevance with a bounded engagement boost |
| Most viewed           | `views_desc`           | `view_count:desc`                              |
| Most liked            | `likes_desc`           | `like_count:desc`                              |
| Most discussed        | `comments_desc`        | `comment_count:desc`                           |
| Newest                | `published_desc`       | `published_at_timestamp:desc`                  |
| Oldest                | `published_asc`        | `published_at_timestamp:asc`                   |

Production rejects the four engagement values. The experimental API enables
them with `YTSEARCH_ENGAGEMENT_SORTS=true`, so a deployment cannot silently
claim to sort by data its backend does not have.

## Data and index representation

`scripts/enrich-search-engagement.ts` collected current public statistics for
all 2,743 distinct video IDs through the YouTube Data API, in 55 requests of at
most 50 IDs. It enriched all 420,720 transcript documents; no video was
missing. The API key is required through the environment and is never written
to output or logs.

The enrichment command accepts plain JSONL or `.jsonl.gz` input and output. A
full-snapshot migration uses gzip-to-gzip streaming, retaining only distinct
video IDs and their three counters in memory rather than materializing the
multi-gigabyte corpus. Its JSON report includes elapsed time and sampled peak
process RSS:

```sh
bun scripts/enrich-search-engagement.ts \
  youtube_transcript_chunks.jsonl.gz \
  youtube_transcript_chunks.engagement.jsonl.gz
sha256sum youtube_transcript_chunks.engagement.jsonl.gz \
  > youtube_transcript_chunks.engagement.jsonl.gz.sha256
```

The builder consolidates each counter once per video, using the maximum seen
across its chunks. The three `u64` counters live in video metadata rather than
postings or per-document metadata. Search hits receive the counters only when
the selected documents are materialized.

| Measurement |        Fuzzy v2 | Engagement v3 |            Change |
| ----------- | --------------: | ------------: | ----------------: |
| Documents   |         420,720 |       420,720 |                 0 |
| Videos      |           2,743 |         2,743 |                 0 |
| Terms       |          59,596 |        59,596 |                 0 |
| Index bytes |      72,441,536 |    73,018,300 | +576,764 (+0.80%) |
| Fresh build | 4,335 ms median |      4,615 ms |             +6.5% |

The original four-channel source repeated fetched counters on every chunk. The
full-snapshot path writes counters only on the first record for each video,
copies all other JSON lines byte-for-byte, and records the number of metric
records in its report. The builder takes the per-video maximum and the reader
injects those counters into every returned hit, so ranking and response values
remain identical without millions of redundant fields or serializations.

## Bounded combined score

Each counter uses logarithmic normalization against its corpus maximum:

```text
metric(value, maximum) = ln(1 + value) / ln(1 + maximum)
engagement = 0.50 * views + 0.35 * likes + 0.15 * comments
combined = relevance + max(relevance, 1) * 0.20 * engagement
```

Log normalization prevents one viral outlier from flattening the rest of the
corpus. The weighted engagement value is in `[0, 1]`, so engagement can add at
most 20% to a text score. It can break close relevance calls, but it cannot
turn an unrelated popular video into a textual match. The three pure metric
sorts remain available when popularity itself is the user's intent.

## Deployed matched benchmark

Both rows use the same Fly app, one shared CPU in `sjc`, nine queries, five
warmups per query, and 30 measured requests per query. External timing includes
TLS, machine routing, and network noise; internal time and process counters are
the better regression signals.

| Measurement               | Fuzzy v2 baseline | Engagement v3 |
| ------------------------- | ----------------: | ------------: |
| Searches                  |               270 |           270 |
| External p50              |         108.00 ms |      96.55 ms |
| External p95              |         134.92 ms |     120.60 ms |
| External mean             |         110.15 ms |      98.03 ms |
| Internal mean search work |          2.010 ms |      2.082 ms |
| CPU ticks                 |                67 |            70 |
| RSS before                |          32.93 MB |      33.04 MB |
| RSS after                 |          33.09 MB |      33.14 MB |

The external improvement is not attributed to the ranking change because the
internal cost is 3.6% higher and the workload crosses the public network. The
meaningful result is that CPU, internal time, and RSS stayed effectively flat
while the index added 0.8% on disk and every returned hit gained three counts.

## Isolation and live topology

- Raw engine: <https://ytsearch-raw-experiment.fly.dev/health>
- Raw-compatible API: <https://ytsearch-api-raw-experiment.fly.dev/api/health>
- Experimental frontend: <https://search.experimental.pronsh.dev>

The browser never receives the Raw search key. Its same-origin Worker proxies
search reads to the Raw-compatible API. Index requests, approval, status, and
activity routes are sent to the existing production API and wake queue, so the
experiment does not fork indexing state. The production Typesense route and
schedule are unchanged.

The search-only API image is 29 MB and omits the CLI, Deno, `yt-dlp`, FFmpeg,
and queue indexer tools. Both experimental Fly services keep zero warm machines
and suspend when idle.

`scripts/cloudflare-worker-experimental-deploy.sh` always builds with the Raw
feature flag, enforces the browser bundle budgets, deploys the isolated Worker
config, and smoke-tests availability plus descending view order.

## Verification

- 30 Raw release tests pass across three suites.
- `cargo clippy --release -p youtube_raw_search -- -D warnings` passes.
- 26 current API tests pass, including sort gating, numeric parsing, count
  propagation, and backend-order preservation.
- The deployed contract passes typo recovery, creator scope, date order,
  engagement order, pagination, channel search, aliases, and authentication.
- The experimental browser build is 23,715 bytes of initial gzip JavaScript,
  below the 30,000-byte budget.

Decision: keep the engagement modes on the isolated Raw hostname. Do not add
the counters or controls to production until a larger judged relevance set and
full-snapshot index pass the existing promotion gates.
