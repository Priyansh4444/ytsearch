# Search Backend Comparison

This project compares two active search backends:

1. Self-hosted Typesense on Fly.io, used by the primary staging frontend.
2. Meilisearch, retained only as a source-level legacy crate and historical documentation.

The managed Typesense service is no longer part of this repo's active staging or benchmark setup.

## Source Notes

- Fly autostop/autostart starts and stops existing Machines based on traffic; stopped or suspended Machines do not bill CPU/RAM. See `https://fly.io/docs/launch/autostop-autostart/`.
- Fly autostop/autostart never creates new Machines; it only starts/stops Machines that already exist. See `https://fly.io/docs/reference/fly-proxy-autostop-autostart/`.
- Fly volumes are billed while provisioned, even when an attached Machine is stopped. See `https://fly.io/docs/about/pricing/`.
- Typesense stores the whole search index in memory and raw data on disk. For keyword search, Typesense recommends RAM around 2x-3x the searchable dataset size. See `https://typesense.org/docs/guide/system-requirements.html`.
- Typesense exposes `/health`, `/debug`, `/metrics.json`, and `/stats.json` for runtime checks. See `https://typesense.org/docs/30.2/api/cluster-operations.html`.

## Decision Status

Typesense is now the active backend and `head.ytsearch-755.pages.dev` is its primary frontend. Meilisearch is deprecated and its Fly engine, API machine, queue worker, volume, and indexed data were destroyed after the preservation commit documented in `docs/legacy/meilisearch-retirement.md`.

The final idle resource snapshot was Meilisearch about 2.8 GB RSS versus Typesense about 1.9 GB RSS on identical `shared-cpu-2x` / 4 GB Fly Machines, with negligible CPU for both. Removing Meilisearch eliminates roughly $22.22/month of warm engine compute plus its volume and API/indexer overhead.

## Fly Runtime Shape

The self-hosted Typesense engine should stay warm. Typesense stores the search index in memory and keeps raw data on disk, so scaling the engine to zero saves compute but adds cold-start/load risk to every first search. The API wrapper can still behave like a serverless service: it has no persistent data, can autostop when idle, and can autostart on traffic.

Current intended shape:

| Component | App | Runtime behavior |
| --- | --- | --- |
| Typesense engine | `ytsearch-typesense-staging` | One warm machine, persistent volume, no autostop. |
| Typesense API wrapper | `ytsearch-api-typesense-fly-staging` | Autostop/autostart, zero minimum machines. |

This follows Fly's model: autostop/autostart starts and stops existing Machines based on requests, and stopped/suspended Machines do not bill CPU/RAM. It also follows Typesense's model: the search index is in memory, with raw data on disk, so keeping the engine warm is the correct latency tradeoff for search.

## Historical Benchmarks

The following results are historical observations, not a current decision record. They predate the corpus-stability guard and must not be used to choose a primary backend.

Measured on July 6, 2026 with:

- Query: `react`
- YouTuber filter: `@t3dotgg`
- API path: `/api/search`
- Scripts: `scripts/benchmark-search-backends.sh` and `scripts/benchmark-search-globalping.sh`
- Meilisearch API: `https://ytsearch-api-meili-staging.fly.dev`
- Self-hosted Typesense API: `https://ytsearch-api-typesense-fly-staging.fly.dev`

Local benchmark:

| Backend | Runs | Errors | Min | Average | P50 | P95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Meilisearch on Fly | 10 | 0 | 106 ms | 140 ms | 118 ms | 333 ms | 333 ms |
| Typesense on Fly | 10 | 0 | 481 ms | 863 ms | 680 ms | 1407 ms | 1407 ms |

Public Globalping probes:

| Location | Meilisearch on Fly | Typesense on Fly |
| --- | ---: | ---: |
| US, Buffalo | 213 ms | 1971 ms |
| Great Britain, London | 433 ms | 1885 ms |
| Japan, Tokyo | 370 ms | 1727 ms |
| Australia, Sydney | 378 ms | 2325 ms |
| Brazil, Sao Paulo | 356 ms | 2315 ms |
| Average | 350 ms | 2045 ms |

Interpretation: Meilisearch is the better default for the current live staging setup. Typesense Fly is healthy, but it spends much more time before first byte for this query.

Follow-up check after keeping the Typesense engine warm:

| Backend | Runs | Errors | Min | Average | P50 | P95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Meilisearch on Fly | 5 | 0 | 187 ms | 388 ms | 322 ms | 869 ms | 869 ms |
| Typesense on Fly | 5 | 0 | 1634 ms | 1922 ms | 1975 ms | 2161 ms | 2161 ms |

Conclusion: the current evidence still does not support making Typesense Fly the main backend. It is healthy and operationally valid, but Meilisearch is faster for the measured `react` / `@t3dotgg` path.

After optimizing the Typesense API path to use Typesense grouping, field-limited responses, and server-side cache:

| Backend | Runs | Errors | Min | Average | P50 | P95 | Max |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Meilisearch on Fly | 10 | 0 | 181 ms | 269 ms | 190 ms | 531 ms | 531 ms |
| Typesense on Fly | 10 | 0 | 242 ms | 376 ms | 330 ms | 627 ms | 627 ms |

Public Globalping probes after the same optimization:

| Backend | Probes | Min | Average | Max |
| --- | ---: | ---: | ---: | ---: |
| Meilisearch on Fly | 5 | 349 ms | 543 ms | 885 ms |
| Typesense on Fly | 5 | 835 ms | 1162 ms | 1495 ms |

Updated conclusion: Typesense Fly is now close locally, but Meilisearch remains the safer global default for this measured workload.

## Cost Shape

| Option | Base cost model | Current staging shape | Cost risk |
| --- | --- | --- | --- |
| Meilisearch on Fly | Fly Machine + Fly Volume + bandwidth/snapshots. | `shared-cpu-2x`, 4 GB engine VM, 10 GB volume in `sjc`. | About $22.22/month for the warm engine compute, plus volume/transfer. |
| Typesense on Fly | Fly Machine + Fly Volume + bandwidth/snapshots. | `shared-cpu-2x`, 4 GB engine VM, 3 GB volume in `sjc`. | About $22.22/month for the warm engine compute, plus volume/transfer. RAM must track its in-memory index. |

## Cluster Management

| Area | Meilisearch on Fly | Typesense on Fly |
| --- | --- | --- |
| Ownership | We run the engine and Fly app. | We run the engine and Fly app. |
| Deployments | `scripts/fly-deploy-meili.sh`. | `scripts/fly-deploy-typesense.sh`. |
| Persistence | Fly volume mounted at the engine data directory. | Fly volume mounted at `/typesense-data`. |
| Backups | Fly volume snapshots; app-level export recommended before schema rewipes. | Fly volume snapshots; app-level export recommended before schema rewipes. |
| HA | Not configured in current staging. | Not configured in current staging. |
| Scaling | Resize Fly VM/volume, then revisit multi-node design if needed. | Resize Fly VM/volume; true HA requires a multi-node Typesense cluster. |
| Secrets | Fly secrets for server/admin keys; frontend only receives API URL. | Dedicated self-host key in Fly secrets; frontend only receives API URL. |

## Commands

Active all-backend update:

```bash
scripts/index-all-backends-update.sh scripts/indexed-channels.txt
```

Meilisearch-only update:

```bash
BACKENDS="meilisearch" scripts/index-all-backends-update.sh scripts/indexed-channels.txt
```

Self-hosted Typesense-only update:

```bash
BACKENDS="typesense-fly" scripts/index-all-backends-update.sh scripts/indexed-channels.txt
```

Deploy frontend variants:

```bash
scripts/cloudflare-pages-deploy-backends.sh
```

Benchmark active backends:

```bash
QUERY=react YOUTUBER=@t3dotgg RUNS=10 scripts/benchmark-search-backends.sh
GLOBALPING_COUNTRIES="US GB JP AU BR" QUERY=react YOUTUBER=@t3dotgg scripts/benchmark-search-globalping.sh
```

The local benchmark summary now includes `total_videos_range`, `total_moments_range`, and `corpus_stable`. A `corpus_stable=no` row means writes were still draining: retain the raw timings for diagnosis but do not compare the backends or switch the primary frontend.

## Serialization Experiment

The experimental `youtube_serialization_bench` crate compares the current
transcript-chunk shape encoded with `serde_json` and a generated `prost`
protobuf schema:

```bash
cargo run -p youtube_serialization_bench --release -- 10000 10
```

On the July 11 sample, protobuf was about 53.4% of JSON size, with roughly
2.6x faster encoding and 2.1x faster decoding. This is a wire-format benchmark,
not a recommendation to change the public API: the schema currently carries
`extra` as JSON and has no compatibility/versioning policy. Typesense's native
HTTP JSON API remains the active integration; protobuf should only be adopted
after a versioned export/import contract and cross-language fixtures exist.

Check machine and indexed-channel status:

```bash
scripts/check-staging-machines.sh
scripts/check-typesense-fly.sh
scripts/check-indexed-channel-status.sh @vsauce @thePrimagen
```

Use `scripts/check-typesense-fly.sh` before claiming Typesense Fly is better. It confirms:

- the engine and API machines are alive or startable;
- Typesense `/health` has no resource error;
- `/metrics.json` has enough RAM/disk headroom;
- the API wrapper returns indexed channel counts;
- the benchmark beats the current Meili numbers for the same query/filter.
