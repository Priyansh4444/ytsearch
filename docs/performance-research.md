# Performance and Retrieval Research Review

Last reviewed: 2026-07-17

## Executive decision

There is no defensible universal “fastest search implementation.” Performance
depends on corpus size, update rate, query distribution, hardware, network path,
and the relevance definition. The goal for ytsearch is therefore the best
measured quality/latency/cost frontier on a frozen transcript corpus and judged
query set.

The current production direction remains sound:

- keep Typesense lexical retrieval as the default and exact-match safety net;
- preserve title and neighboring transcript context;
- add semantic retrieval only as a measured recall layer;
- fuse independent rankings before grouping by video;
- keep public JSON with compression;
- optimize local CPU work, but prioritize backend and network tail latency;
- reject changes that lack stable counts, relevance judgments, and repeated
  latency distributions.

This review kept one implementation change: relevance sort keys are now computed
once per item instead of repeatedly inside sort comparisons. A five-run release
microbenchmark measured a **1.84x–2.37x speedup** for the isolated ranking sort,
with identical output order. This does not imply an equivalent end-to-end gain.

## Fresh baseline

The benchmark used commit `143af8f`, the existing four fixed search cases, 10
public requests per case, gzip, and five serialization runs of 10,000 chunks ×
10 rounds. Corpus counts were stable and every request succeeded.

| Search case | Backend first-result average | End-to-end p50 | End-to-end p95 |
| --- | ---: | ---: | ---: |
| low-signal `I` | 16 ms | 187 ms | 780 ms |
| `linux exploit` | 8 ms | 197 ms | 253 ms |
| `fable on high` | 20 ms | 141 ms | 393 ms |
| `battery gate incident` | 7 ms | 167 ms | 643 ms |

The gap between 7–20 ms backend time and 253–780 ms public p95 means network,
queueing, cache state, and tail effects dominate these samples. Replacing JSON
or adding a neural reranker cannot honestly be presented as the first latency
fix.

Serialization remained consistent in size: protobuf used 365.6 bytes per chunk
and JSON 684.3 bytes, a 0.534 ratio. Across five runs, JSON encode/decode ranged
7.07–8.89/13.44–14.79 ms per round; protobuf ranged
3.03–3.94/5.63–7.84 ms. This supports protobuf for a future bulk internal
transport, not for the browser API where compatibility and network compression
matter more.

## What the research says

### Start with a strong lexical baseline

[BEIR](https://arxiv.org/abs/2104.08663) evaluated lexical, sparse, dense,
late-interaction, and reranking systems across 18 heterogeneous datasets. BM25
was a robust baseline; late interaction and reranking were strongest on average
but had high computational cost. The important production lesson is not that one
family always wins, but that quality and cost must be compared out of domain.

For ytsearch, exact names, code identifiers, quotations, and creator filters are
high-value lexical cases. Typesense therefore remains the primary retriever and
fallback. Semantic-only promotion would need to beat the held-out ytsearch
judgments without regressing those exact-match classes.

### Transcript passages need context

Honda and Akiba’s spoken-document retrieval work found that indexing neighboring
context improved retrieval from lecture transcripts
([LREC 2010](https://aclanthology.org/L10-1319/)). The
[DAPR benchmark](https://arxiv.org/abs/2305.13915) later attributed 53.5% of
analyzed state-of-the-art passage-retrieval errors to missing document context
and found benefits from contextualized passage representations such as prepended
titles.

ytsearch already stores `prev_text` and `next_text`, returns them for readable
snippets, and searches a separately weighted title field. It should not blindly
index all neighboring text into every chunk: that increases postings and can
reduce timestamp precision. The correct experiment is to compare 5-, 8-, and
10-second windows plus title/context variants against judged transcript moments.

Research also cautions against assuming sophisticated segmentation always wins.
Tiedemann and Mur reported competitive simple segmentation strategies for
passage retrieval ([COLING workshop 2008](https://aclanthology.org/W08-1803/)).
Fixed time windows are therefore a valid baseline, not a proven optimum.

### Hybrid retrieval should preserve independent signals

The original Reciprocal Rank Fusion work found that a simple rank-based fusion
method consistently outperformed individual systems in its TREC experiments
([Cormack, Clarke, and Büttcher, SIGIR 2009](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf)).
The `k=60` starting point in ytsearch follows that work. Fusion must happen at
chunk level before video grouping; otherwise many adjacent hits from one video
can distort candidate diversity.

[SPLADE v2](https://arxiv.org/abs/2109.10086) shows that learned sparse
representations can combine lexical-style inverted indexes with learned term
expansion, while improving effectiveness substantially on its reported tasks.
It is a future candidate for paraphrase recall when GPU/model operations are
acceptable, but it adds training/inference and expanded postings. It should be
compared with the existing lexical baseline and a small dense model, not adopted
from public benchmark scores alone.

### Dense and late-interaction retrieval are quality/cost choices

[ColBERTv2](https://arxiv.org/abs/2112.01488) improves late interaction with
residual compression, reporting a 6–10x reduction in its representation
footprint. [PLAID](https://arxiv.org/abs/2205.09707) adds centroid pruning and
reports up to 7x GPU and 45x CPU latency improvements over vanilla ColBERTv2.
Those results make late interaction a credible high-quality candidate, but not a
free upgrade for this small staging system: model serving, token-level storage,
warmup, and operational complexity must be included in cost and p95.

If dense retrieval is tested, HNSW is the appropriate baseline ANN family.
The original [HNSW paper](https://arxiv.org/abs/1603.09320) describes its
hierarchical proximity graph and logarithmic scaling behavior. Recall must be
measured as `ef`/graph parameters change; ANN speed without held-out recall is
not a valid improvement.

[Matryoshka Representation Learning](https://arxiv.org/abs/2205.13147) supports
coarse-to-fine embeddings that can be truncated for different resource budgets.
This justifies the existing proposed 128/256/768-dimension experiment, but only
for models explicitly trained for Matryoshka truncation. Arbitrarily truncating
another model is not equivalent.

### Public leaderboards are screening tools, not promotion gates

[MTEB](https://aclanthology.org/2023.eacl-main.148/) evaluated 33 models across
58 datasets and found that no single embedding method dominated all tasks.
BEIR likewise emphasizes heterogeneous, out-of-domain evaluation. YouTube
transcripts contain disfluencies, repeated speech, code tokens, creator names,
and timestamp-local relevance that generic leaderboards do not represent.

The project therefore needs a versioned, human-judged test set before promoting
SPLADE, dense embeddings, ColBERT, or a cross-encoder. Public scores may shortlist
candidates; they cannot select the production model.

### Query engines should prune work without changing rank

Block-Max WAND research describes dynamic pruning that avoids scoring postings
which cannot reach the top-k while preserving effectiveness
([Macdonald and Tonellotto, ICTIR 2017](https://eprints.gla.ac.uk/145572/)).
This is primarily an engine-level decision. ytsearch should let Typesense own
posting-list execution rather than reproduce an inverted index inside the API.
API-side work should remain bounded to candidate grouping, deterministic
reranking, and response construction.

## Implementation audit

| Surface | Current choice | Research-aligned decision | Next proof required |
| --- | --- | --- | --- |
| First-stage retrieval | Typesense lexical | Keep | judged MRR/nDCG/Recall plus p95 |
| Passage unit | fixed time windows | Keep as baseline | 5/8/10-second isolated reindex |
| Context | title + adjacent snippet fields | Keep | test indexed context variants |
| Hybrid fusion | weighted RRF, `k=60` | Keep experimental | tune only on train split, verify held-out |
| Semantic ANN | Typesense/HNSW candidate | Do not promote yet | recall/latency/memory frontier |
| Late interaction | none | Research candidate only | compare ColBERTv2/PLAID on same judgments |
| Neural reranker | none | Optional top-20 experiment | p95 budget and exact-match guardrail |
| Public encoding | gzip JSON | Keep | browser end-to-end evidence to replace |
| API reranking | deterministic local pass | Keep optimized | watch `result_processing_ms` |
| Exact video totals | separate distinct-count query | Measure | compare omission/approximation/exact UX value |

## Kept CPU optimization

The relevance path previously called `to_ascii_lowercase()` and recomputed term
coverage from sort comparators, so an item could be normalized many times during
`O(n log n)` sorting. It now uses cached sort keys:

- each hit relevance key is calculated once;
- each video relevance key is calculated once;
- the existing timestamp, match-count, publication-date, and title tie-breaks
  remain unchanged;
- existing relevance fixtures and an explicit old/new order assertion protect
  behavior.

Reproduce the isolated comparison:

```bash
for run in 1 2 3 4 5; do
  cargo test --release -q -p youtube_api \
    cached_relevance_keys_avoid_repeated_normalization -- \
    --ignored --nocapture
done
```

Recorded speedups were 2.37x, 2.10x, 2.12x, 1.98x, and 1.84x. This benchmark
uses 4,096 synthetic hits to make CPU differences visible. Production requests
normally rerank far fewer candidates, so monitor `result_processing_ms`; do not
translate the microbenchmark directly into an end-to-end claim.

## Highest-value experiment order

1. **Build judgments first.** Create at least 100 queries covering exact terms,
   code identifiers, paraphrases, vague concepts, stopwords, low-signal queries,
   creator filters, and near-duplicate moments. Split tuning and held-out sets.
2. **Remove avoidable request work.** On a corpus snapshot, compare the current
   exact distinct-video count query with response-only counts and a cached or
   approximate count. Keep exact totals only if their UX value justifies p95.
3. **Chunking frontier.** Reindex isolated 5/8/10-second collections. Measure
   nDCG@10, MRR, Recall@10, index bytes, import throughput, and timestamp error.
4. **Hybrid baseline.** Compare lexical, a small dense model, and RRF fusion with
   equal candidate depths. Record embedding latency separately.
5. **Dimensionality.** For a Matryoshka-trained model, compare 128/256/768
   dimensions on identical HNSW settings, then tune ANN parameters.
6. **Rerank only top candidates.** Test a cross-encoder on the top 20 after
   retrieval. Reject it if held-out gains do not pay for p95 and warm memory.
7. **Late interaction last.** Evaluate ColBERTv2/PLAID only if simpler hybrid
   retrieval fails the recall target.

## Promotion gates

A candidate is kept only when all of these are true:

- same corpus fingerprint and query set;
- zero errors and stable result cardinality;
- no exact-match, low-signal, or creator-filter regression;
- held-out nDCG@10/MRR/Recall@10 improves beyond repeated-run noise;
- warm and cold p50/p95 are recorded independently;
- `backend_search_ms`, `result_processing_ms`, total duration, response bytes,
  index bytes, peak RSS, and import throughput are recorded;
- at least five alternating baseline/candidate runs, more for public networks;
- operational cost, fallback behavior, and rollback steps are documented.

“Faster” without relevance parity is a quality regression. “More relevant”
without bounded p95 and cost is not production-ready. The winning configuration
must sit on the measured Pareto frontier for this corpus and workload.

