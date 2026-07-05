# Search Backend Comparison

This compares the three realistic options for this project:

1. Self-hosted Meilisearch on Fly.io, currently used by staging.
2. Typesense Cloud, the managed Typesense baseline.
3. Self-hosted Typesense on Fly.io, added on this branch via `fly.typesense.toml`.

Sources checked on 2026-07-04:

- Meilisearch pricing: https://www.meilisearch.com/pricing
- Typesense Cloud pricing: https://cloud.typesense.org/pricing/calculator
- Typesense Cloud billing: https://cloud-help.typesense.org/article/billing-process
- Typesense Cloud HA: https://cloud-help.typesense.org/article/high-availability
- Typesense self-host install/config: https://typesense.org/docs/guide/install-typesense.html and https://typesense.org/docs/29.0/api/server-configuration.html
- Fly.io resource pricing: https://fly.io/docs/about/pricing/

## Recommendation

Use self-hosted Meilisearch on Fly for the staging path right now. It is already wired into the frontend, API, and CLI; the recent API changes made it fast enough for the current dataset; and it is simpler operationally than running a self-hosted Typesense cluster.

Use Typesense Cloud as the production fallback if Meilisearch quality or scale becomes a blocker. It gives real managed cluster operations, HA, node replacement, upgrades, and support without us building those pieces.

Use self-hosted Typesense on Fly only as a controlled benchmark branch. A single-node Typesense Fly app is useful to test raw engine behavior and keep the abstraction honest, but it is not production-grade HA by itself.

## Current Benchmarks

Benchmark shape:

- Query: `fable`
- YouTuber filter: `@t3dotgg`
- API path: `/api/search`
- Client scripts: `scripts/benchmark-search-backends.sh` and `scripts/benchmark-search-globalping.sh`
- Meilisearch API: `https://ytsearch-api-meili-staging.fly.dev`
- Existing Typesense Cloud-backed API: `https://ytsearch-api-pronsh-sandbox.fly.dev`
- Self-hosted Typesense API: `https://ytsearch-api-typesense-fly-staging.fly.dev`

Measured on 2026-07-05 after restarting the overloaded Meilisearch staging machine:

| Local metric, 5 runs | Meilisearch on Fly | Typesense Cloud | Typesense on Fly |
| --- | ---: | ---: | ---: |
| min | 123 ms | 163 ms | 156 ms |
| average | 420 ms | 226 ms | 197 ms |
| p50 | 150 ms | 190 ms | 181 ms |
| p95 | 1487 ms | 368 ms | 294 ms |
| max | 1487 ms | 368 ms | 294 ms |
| HTTP errors | 0 | 0 | 0 |

| Public probe | Meilisearch on Fly | Typesense Cloud | Typesense on Fly |
| --- | ---: | ---: | ---: |
| US, Buffalo | 295 ms | 209 ms | 189 ms |
| Great Britain, London | 401 ms | 343 ms | 403 ms |
| Japan, Tokyo | 344 ms | 490 ms | 296 ms |
| Australia, Sydney | 330 ms | 538 ms | 243 ms |
| Brazil, Sao Paulo | 439 ms | 621 ms | 487 ms |
| Public probe average | 362 ms | 440 ms | 324 ms |

Interpretation: self-hosted Typesense on Fly was fastest on average in this short local and public-probe run. Typesense Cloud remained consistent and fully managed. Meilisearch was fast after warmup, but the first local request was slow and the machine had just been restarted after `408 Request Timeout` responses during/after heavy indexing. Treat Meilisearch benchmark numbers as sensitive to indexing load on the current 1 GB staging VM.

Self-hosted Typesense on Fly is only partially indexed because the full indexing job was intentionally stopped. Re-run the full one-off update before treating self-host Typesense quality/counts as final.

## Pricing

| Option | Base cost model | Current likely staging shape | Cost risk |
| --- | --- | --- | --- |
| Meilisearch on Fly | Fly Machine + Fly Volume + bandwidth/snapshots. Fly lists volumes at `$0.15/GB/month`; running Machines are usage-based by CPU/RAM preset and region. | 1 shared CPU machine, 1 GB RAM, 3 GB volume in `sjc`. | Low and predictable for staging. Production HA requires more Machines/volumes and operational work. |
| Typesense Cloud | Dedicated cluster hourly fee by RAM/CPU/config, plus bandwidth, plus optional support. Docs say bandwidth is usually about `$0.09-$0.12/GB` depending on region. | Pick lowest RAM/vCPU that fits the indexed dataset, then benchmark. Typesense recommends 2-3x dataset-on-disk as RAM. | Higher minimum spend than a tiny Fly VM, but includes managed ops and makes HA straightforward. |
| Typesense on Fly | Fly Machine + Fly Volume + bandwidth/snapshots. Same Fly resource pricing as Meili. | `fly.typesense.toml`: 1 shared CPU, 1 GB RAM, 3 GB volume, single region `sjc`. | Cheap for staging, but production HA/backup/upgrade burden is ours. |

Meilisearch Cloud is not one of the three target deployments, but it is useful context: their public pricing page says Cloud starts at `$20/month`; the estimator example for 100K docs and 10K searches showed `$30/month` usage-based or `$23/month` resource-based for an XS instance with disk.

For the current cheapest comparison:

- Typesense Cloud: use the actual current cluster cost, `$55/month`.
- Meilisearch on Fly: cheapest useful staging shape is one 1 GB engine VM plus one 512 MB API VM plus a 3 GB volume. Fly volume storage is `$0.15/GB/month`; compute is usage-based by Machine size/region. This should be materially cheaper than `$55/month`, but production HA would add another engine/API shape and operational burden.
- Typesense on Fly: cheapest useful staging shape is one 1 GB Typesense VM plus one 512 MB API VM plus a 3 GB volume. Cost shape is similar to Meilisearch on Fly. The tradeoff is RAM pressure: Typesense generally wants RAM proportional to indexed data, while Meilisearch leans more comfortably on disk for this workload.

## Cluster Management

| Area | Meilisearch on Fly | Typesense Cloud | Typesense on Fly |
| --- | --- | --- | --- |
| Ownership | We run the engine and Fly app. | Typesense runs the cluster. | We run the engine and Fly app. |
| Deployments | `scripts/fly-deploy-meili.sh`. | Cloud dashboard/API. | `scripts/fly-deploy-typesense.sh`. |
| Persistence | Fly volume mounted at the engine data directory. | Managed by Typesense Cloud. | Fly volume mounted at `/typesense-data`. |
| Backups | Fly volume snapshots; app-level export still recommended before schema rewipes. | Managed platform behavior; still keep app-level export/import path. | Fly volume snapshots; app-level export required for safe schema experiments. |
| HA | Not configured in current staging. Would need multiple nodes, replication strategy, and failover design. | Built-in option: 3-7 nodes across physically/geographically separate datacenters. Cloud docs say a cluster with `N` nodes tolerates `floor((N - 1) / 2)` node failures. | Not configured. Single Fly volume is region-local; true Typesense HA needs a multi-node Typesense cluster and careful Fly networking/placement. |
| Scaling | Resize Fly VM/volume, or design multi-node. | Change RAM/CPU, enable HA, add Search Delivery Network. Cloud docs recommend enabling HA for production. | Resize Fly VM/volume manually. Multi-node is possible but no longer simple. |
| Upgrades | Change image version and deploy; verify data compatibility. | Managed, including safer rolling behavior when HA is enabled. | Change image version and deploy; we own rollback and data compatibility. |
| Secrets | Fly secrets for server/admin keys; frontend only receives API URL, not master keys. | Cloud API keys in Fly/Cloudflare secrets. | Dedicated `TYPESENSE_SELFHOST_API_KEY` for server startup, then separate app env values for API clients. |

## Code Architecture

The Rust CLI now treats search as a backend-selected abstraction instead of a Typesense-specific config path:

- `--backend meilisearch` uses `MEILI_HTTP_ADDR` and `MEILI_MASTER_KEY` for admin indexing.
- `--backend typesense` uses `TYPESENSE_API_URL` and `TYPESENSE_ADMIN_API_KEY`.
- `--mode update` is the default for heavy creator indexing. It keeps existing chunks and skips already-indexed videos.
- `--mode rewipe` intentionally deletes existing chunks first, which is the right mode for schema/chunking/timeframe changes.
- `index-youtubers` reads a plain text list and applies the same backend/mode defaults to every creator.
- Three Cloudflare Pages previews can be deployed from the same frontend build, each with a different backend API URL.

This keeps Meilisearch, Typesense Cloud, and self-hosted Typesense hot-swappable at the CLI/API configuration layer while still allowing engine-specific clients underneath.

## Commands

Meilisearch update:

```bash
cargo run --release -p youtube_cli -- index-youtuber @t3dotgg --backend meilisearch --mode update --jobs 8
```

Meilisearch batch update:

```bash
cargo run --release -p youtube_cli -- index-youtubers scripts/indexed-channels.txt --backend meilisearch --mode update --jobs 8
```

Meilisearch schema/chunking rewipe:

```bash
cargo run --release -p youtube_cli -- index-youtuber @t3dotgg --backend meilisearch --mode rewipe --jobs 8
```

Deploy self-hosted Typesense on Fly:

```bash
export TYPESENSE_SELFHOST_API_KEY="$(openssl rand -hex 32)"
scripts/fly-deploy-typesense.sh
```

Deploy the API in front of self-hosted Typesense:

```bash
scripts/fly-set-typesense-selfhost-api-secrets.sh
scripts/fly-deploy-typesense-api.sh
```

Deploy frontend variants:

```bash
scripts/cloudflare-pages-deploy-backends.sh
```

Point the CLI at self-hosted Typesense:

```bash
export YTSEARCH_SEARCH_BACKEND=typesense
export TYPESENSE_API_URL=https://ytsearch-typesense-staging.fly.dev
export TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
export TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
cargo run --release -p youtube_cli -- index-youtubers scripts/indexed-channels.txt --backend typesense --mode update --jobs 8
```

## Open Production Work

- Add a restricted Meilisearch search key instead of using the master key for API reads.
- Rotate any local keys that were pasted into terminals or shared logs.
- Add app-level export/import commands before large `rewipe` jobs.
- Benchmark self-hosted Typesense on Fly after indexing the same channel set.
- Decide whether production needs managed HA. If yes, Typesense Cloud is much cleaner than rolling our own cluster on Fly.
