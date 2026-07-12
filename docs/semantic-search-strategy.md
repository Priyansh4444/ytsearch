# Semantic Search Strategy

## Decision

Add semantic retrieval as a measured recall layer, not as a replacement for
Typesense lexical search. Exact names, identifiers, quotations, and timestamps
remain lexical strengths. Paraphrases and vague remembered concepts are the
semantic target.

The initial production candidate is Typesense-native hybrid search using a
separately generated embedding field. This avoids a second database and already
supports keyword/vector rank fusion. Do not enable `rerank_hybrid_matches` by
default: Typesense computes missing scores for every candidate when enabled,
which costs additional query work.

## Candidate pipeline

1. Build embedding input from the video title, chapter, and transcript chunk.
   Preserve the raw searchable fields separately.
2. Embed documents asynchronously during indexing. Cache by a versioned hash of
   model name, model revision, task prefix, dimensions, and normalized input.
3. Embed queries with the model's query-specific instruction where required.
4. Retrieve lexical and semantic candidates independently, initially 50 each.
5. Fuse with reciprocal-rank fusion. Start with equal weights and `k=60`.
6. Group by video only after chunk-level fusion so one video's many adjacent
   chunks cannot crowd out the candidate set.
7. Optionally rerank the top 20 fused chunks with a cross-encoder after latency
   and relevance are measured. Return the top grouped videos and moments.
8. Fall back to lexical results on embedding timeout, model failure, dimension
   mismatch, or Typesense vector errors.

## Model experiments

Start with local, open-weight English models because transcript text currently
appears predominantly English and local inference makes indexing cost bounded.

| Candidate | Dimensions | Why test it |
| --- | ---: | --- |
| `nomic-ai/nomic-embed-text-v1.5` | 128, 256, 768 | Matryoshka dimensions allow a direct storage/recall experiment from one model. Requires `search_document:` and `search_query:` prefixes. |
| `BAAI/bge-small-en-v1.5` | 384 | Small 33.4M-parameter baseline with ONNX support and a permissive license. |

Do not select from public MTEB scores alone. YouTube transcript chunks contain
speech disfluencies, adjacent context, code terms, creator names, and repeated
phrasing that generic embedding benchmarks do not model.

## Evaluation set

Create a versioned TSV/JSONL set with at least these query classes:

- exact title/name/code identifier where lexical must not regress;
- paraphrase of a known transcript moment;
- vague concept remembered without the video's wording;
- stopword-heavy natural language query;
- one-character/low-signal query, which should remain lexical/newest;
- creator-filtered query;
- adversarial near-duplicate moments from the same video.

Each query records relevant chunk IDs and acceptable video IDs. Split judged
queries into a tuning set and a held-out promotion set. Record MRR, Recall@10,
nDCG@10, p50/p95 latency, embedding latency, index bytes, and peak memory.

Promotion requires:

- no regression on exact-match and low-signal fixtures;
- improved held-out MRR or Recall@10 beyond repeated-run noise;
- stable result counts and zero query errors;
- p95 within the agreed interactive latency budget;
- a complete lexical fallback;
- embedding model and schema versions persisted with every vector.

## Cost experiment

Nomic's published Matryoshka dimensions provide the first bounded experiment.
At float32, raw vector storage is 512 bytes at 128 dimensions, 1,024 bytes at
256, and 3,072 bytes at 768 before index overhead. Embed the same fixed corpus
once at 768 dimensions, normalize after truncation as specified by the model,
then build isolated 128/256/768 Typesense collections. Compare held-out recall,
HNSW memory, import time, and query p95. A smaller vector is kept only if its
measured quality/cost frontier dominates the larger alternative.

## Rollout

1. Land the judged dataset and offline runner.
2. Benchmark candidate models locally without importing production data.
3. Seed isolated Typesense collections from a fixed transcript snapshot.
4. Shadow semantic queries from sampled, privacy-safe search events without
   changing user-visible ranking.
5. Review metrics, then expose hybrid search behind a server-side flag.
6. Ramp gradually with lexical fallback and monitor click/moment feedback.
