# Staging Observability

Use these scripts when you need to answer:

- Which Fly machines are running?
- Which backend has a channel indexed?
- Is a requested channel queued?
- Which backend is faster from local and public probes?

For the seven-machine staging map (queue API plus one indexer per backend), read [staging-architecture.md](staging-architecture.md).

## Machine Report

```bash
scripts/check-staging-machines.sh
```

For a deeper self-hosted Typesense check, run:

```bash
scripts/check-typesense-fly.sh
```

That script checks the Fly machines, `/health`, `/debug`, `/metrics.json`, the API wrapper health, indexed channel counts, and a Typesense-only benchmark.

Latest Typesense Fly runtime check:

```text
health: ok=true
debug: version=30.2 state=1
memory_used_mb=454 memory_total_mb=962 typesense_resident_mb=274
disk_used_gb=0.76 disk_total_gb=2.87
indexed @t3dotgg: videos=1080 chunks=271033
```

This is a historical pre-retirement sample, not the current topology:

```text
app	machine	state	region	size	image	volume
ytsearch-api-typesense-fly-staging	1855eddf654298	started	sjc	shared/1/512	registry.fly.io	queue volume
ytsearch-typesense-staging	8147deb94937e8	started	sjc	shared/2/4096	docker-hub-mirror.fly.io	Typesense data volume
ytsearch-indexer-staging	781e414be41208	started	sjc	performance/4/8192	registry.fly.io	transcript cache
```

The script prints `summary started_machines=3 apps_with_machines=3 expected_staging_machines=3` when healthy.

Verify the one Typesense worker invariant:

```bash
scripts/check-queue-indexers.sh
```

## Channel Status

```bash
scripts/check-indexed-channel-status.sh @vsauce @thePrimagen
```

The script checks each active backend's `/api/youtubers` data, runs a small scoped search, and, when `YTSEARCH_INDEX_QUEUE_TOKEN` is available, checks the private queue.

Current result from July 6, 2026:

```text
backend	handle	indexed	videos	chunks	public_videos	latest_video	search_http	search_videos	first_result
meilisearch	@vsauce	no	0	0			200	0
meilisearch	@thePrimagen	no	0	0			200	0
typesense-fly	@vsauce	no	0	0			200	0
typesense-fly	@thePrimagen	no	0	0			200	0

queued_requests
id	handle	status	target_backend	requested_at	note
1783302653-meilisearch-vsauce	@vsauce	queued	meilisearch	1783302653
1783302653-typesense-fly-vsauce	@vsauce	queued	typesense-fly	1783302653
```

Interpretation:

- `@vsauce` is queued, but no active backend has indexed searchable data for it yet.
- `@thePrimagen` is not indexed and was not present in the queue at the time of this check.
- This historical sample predates Meilisearch retirement. Current staging has one
  started worker for `typesense-fly`; retired-Meili workers must not be started.

## Axiom Search Timing

Both active API apps emit a `search_query` event to Axiom when `AXIOM_API_KEY`/`AXIOM_TOKEN` and `AXIOM_DATASET` are configured.

Important fields:

| Field | Meaning |
| --- | --- |
| `backend` | `meilisearch` or `typesense`. |
| `duration_ms` | Total API handler time. |
| `backend_search_ms` | Total time spent waiting on search backend calls. |
| `first_backend_search_ms` | First backend search call latency. |
| `result_processing_ms` | Rust-side result parsing/grouping/shaping time. |
| `backend_search_calls` | Number of backend search requests needed for one API response. |
| `backend_raw_hits` | Count of ungrouped backend hits returned. |
| `backend_grouped_hits` | Count of backend groups returned by grouped search. |
| `response_body_bytes` | Serialized JSON response size. |
| `returned_videos` / `returned_hits` | Final frontend-visible result sizes. |
| `client_country` / `cf_ray` / `user_agent` | Non-secret request metadata useful for regional/device debugging. |

Useful Axiom questions:

```text
search_query
| summarize avg(duration_ms), p95(duration_ms), avg(backend_search_ms), p95(backend_search_ms) by backend
```

```text
search_query
| summarize avg(duration_ms), p95(duration_ms) by backend, client_country
```

```text
search_query
| where backend_search_calls > 1
| project _time, backend, query, youtuber, backend_search_calls, backend_search_ms, duration_ms
```

Use `scripts/axiom-search-report.sh` for a redacted aggregate report. The
ingestion token needs Axiom `query:read` in addition to write access; a
write-only token correctly receives HTTP 403 and cannot be used for analysis.

## Local Benchmark

Active backend benchmark:

```bash
RESULTS_FILE=/tmp/ytsearch-local-benchmark-$(date +%Y%m%d).log \
QUERY=react \
YOUTUBER=@t3dotgg \
RUNS=10 \
BACKENDS="meilisearch typesense-fly" \
scripts/benchmark-search-backends.sh
```

Current local result from July 6, 2026:

```text
backend	count	errors	min_ms	avg_ms	p50_ms	p95_ms	max_ms
meilisearch	10	0	106	140	118	333	333
typesense-fly	10	0	481	863	680	1407	1407
```

## Public Multi-Location Benchmark

Globalping benchmark:

```bash
GLOBALPING_COUNTRIES="US GB JP AU BR" \
QUERY=react \
YOUTUBER=@t3dotgg \
BACKENDS="meilisearch typesense-fly" \
scripts/benchmark-search-globalping.sh
```

Current raw result from July 6, 2026:

```text
backend	city	country	continent	network	status	http	dns_ms	tcp_ms	tls_ms	ttfb_ms	total_ms	raw
meilisearch	Buffalo	US	NA	HostPapa	finished	200	38	4	20	143	213
meilisearch	London	GB	EU	OVH	finished	200	177	7	16	233	433
meilisearch	Tokyo	JP	AS	xTom	finished	200	112	1	10	246	370
meilisearch	Sydney	AU	OC	Oracle	finished	200	15	1	15	346	378
meilisearch	Sao Paulo	BR	SA	Oracle	finished	200	120	1	24	197	356
typesense-fly	Buffalo	US	NA	HostPapa	finished	200	27	3	25	1914	1971
typesense-fly	London	GB	EU	OVH	finished	200	131	12	28	1713	1885
typesense-fly	Tokyo	JP	AS	xTom	finished	200	113	0	10	1603	1727
typesense-fly	Sydney	AU	OC	Oracle	finished	200	9	1	15	2299	2325
typesense-fly	Sao Paulo	BR	SA	Oracle	finished	200	56	2	17	2239	2315
```

Summary:

```text
backend	count	min_total_ms	avg_total_ms	max_total_ms
meilisearch	5	213	350	433
typesense-fly	5	1727	2045	2325
```

Historical interpretation: before the Typesense capacity upgrade, Meilisearch
was the better default for this sample. This is not a current deployment
recommendation; see `docs/performance-investigation-2026-07-11.md` for the
reversal, controls, and retirement decision.

After optimizing Typesense Fly grouping and response fields:

```text
local:
meilisearch	10	0	181	269	190	531	531
typesense-fly	10	0	242	376	330	627	627

global:
meilisearch	5	349	543	885
typesense-fly	5	835	1162	1495
```
