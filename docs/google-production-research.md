# Google Production Practices Applied to ytsearch

This document extracts applicable practices from Google's public Search, SRE,
research, and cost-optimization material. It does not attempt to copy Google's
warehouse-scale architecture into a single-node search service.

## Practices to adopt

1. **Separate discovery, indexing, and serving.** Google describes Search as
   crawling, indexing, then serving. ytsearch should keep queue discovery and
   caption extraction outside the public API, with Typesense as the serving
   index. The current three-service separation follows this principle.
2. **Optimize the tail.** Google research emphasizes that p95/p99 behavior can
   dominate interactive experience even when averages look healthy. Promotion
   gates therefore use per-case p95 plus confidence bounds, not average alone.
3. **Degrade optional work before core search.** Under overload, expensive
   grouped totals, semantic reranking, prewarming, and analytics should have
   deadlines and be skippable. Exact lexical search and creator filtering are
   the minimum viable response.
4. **Bound queues and retries.** Retries use exponential backoff with jitter and
   must not multiply overload. The single-writer index queue stays durable, but
   invalid rows cannot block later work.
5. **Test cold state and overload.** Warm-cache latency is insufficient. API
   machine experiments include cold starts, and future load tests must find the
   nonlinear saturation point without targeting production.
6. **Use caches as latency improvements, not survival requirements.** Search
   must remain correct on a cache miss. Production gzip/cache headers are kept
   because measured transfer and p95 improved, not because the backend depends
   on them for correctness.

## Cost model

Every proposed feature should record these quantities before rollout:

| Feature | Incremental storage | Per-index CPU/network | Per-query CPU/network | Failure fallback |
| --- | --- | --- | --- | --- |
| Semantic vectors | dimensions x 4 bytes x chunks plus HNSW overhead | model inference and vector import | query embedding and ANN/fusion | lexical search |
| Cross-encoder reranking | model/image/cache bytes | none or model warmup | candidates x model inference | fused lexical/vector order |
| Full category totals | schema field plus backfill | one collection migration | extra filters/count calls | current-page counts |
| Search prewarming | none | none | additional speculative queries | submit normally |
| Longer result pages | none | none | larger Typesense and response work | pagination/show-more |

Cost experiments use ephemeral infrastructure with a spending cap. Google
Cloud's architecture guidance recommends workload-specific profiling,
autoscaling limits, ephemeral experimental resources, and defined spending
limits. For this Fly deployment, that translates to explicit TTL cleanup,
`MAX_COST_USD`, fixed replay workloads, and cost per successful workload unit.

## Promotion rule

Run baseline and candidate at least three times per frozen case. An experiment
is invalid on HTTP errors, result-count drift, unstable corpus counts, or failed
correctness fixtures. A candidate is kept only when its confidence-adjusted
mean and p95 improve for every required case. Run:

```bash
scripts/compare-autoresearch-search.sh baseline.log candidate.log
```
