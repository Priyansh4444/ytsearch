# Low-resource semantic search experiment (2026-07-18)

## Decision

Keep Typesense lexical search as the production default. The safest next
semantic experiment is a coarse-to-fine, exact-search sidecar with a small
static embedding model. Do **not** add one vector for every transcript moment
to the current Typesense machine.

The live API currently reports 26,385 videos and 7,129,139 transcript moments.
At that size, 256-dimensional `f32` moment vectors alone need about 6.8 GiB,
before document, graph, allocator, and process overhead. This is a poor fit for
the existing 8 GiB search machine.

Instead, divide each video into four temporal regions and average the region's
passage embeddings. This produces about 105,540 vectors:

| Representation | Vector count | Raw `f32`, 256d | Raw `int8`, 256d |
| --- | ---: | ---: | ---: |
| One vector per video | 26,385 | 25.8 MiB | 6.4 MiB |
| Four regions per video | 105,540 | 103.1 MiB | 25.8 MiB |
| Eight regions per video | 211,080 | 206.1 MiB | 51.5 MiB |
| Every transcript moment | 7,129,139 | 6.8 GiB | 1.7 GiB |

Four regions preserve coarse timestamp locality while remaining small enough
for a contiguous, in-process exact cosine scan. At this scale, avoiding HNSW
also avoids graph memory, build time, tuning, and recall loss.

## Local embedding measurements

These measurements used real cached transcript passages on the current local
development machine. They are implementation screening numbers, not a quality
evaluation. Process RSS includes Python and the inference runtime. First-load
time includes the initial unauthenticated model download and therefore is not a
packaged cold-start measurement.

| Model/runtime | Dimensions | Texts | Corpus throughput | Query p50 / p95 | Process RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| `minishlab/potion-base-8M`, Model2Vec | 256 | 10,000 | 45,059 texts/s | 0.032 / 0.063 ms | 168 MiB |
| `BAAI/bge-small-en-v1.5`, FastEmbed ONNX | 384 | 2,000 | 137 texts/s | 3.06 / 4.02 ms | 1,142 MiB |

On this workload, Model2Vec was roughly 329 times faster for corpus encoding,
96 times faster at query p50, and used about 973 MiB less process RSS. This
does **not** establish quality parity. Static embeddings discard much of word
order and context, which may hurt code, ambiguous terms, and phrase-sensitive
queries.

## Proposed experiment

1. Embed transcript passages with `potion-base-8M`, then average normalized
   embeddings into four chronological regions per video.
2. Store a contiguous matrix plus a compact mapping from row to video and time
   interval. Start with `f32`; test symmetric `int8` only after a recall check.
3. Exact-scan the region matrix for the semantic top candidates. No separate
   vector database or HNSW index is needed at this size.
4. Fuse semantic candidates with the existing Typesense ranking using reciprocal
   rank fusion (`k=60`). Keep lexical-only results as a fallback and preserve
   the current timestamp snippets.
5. Run it in shadow mode. Record only aggregate timing, candidate overlap, and
   resource measurements until a privacy-reviewed judged-query set exists.

## Intra-frame and inter-frame prototype

`youtube_semantic` now has an isolated multimodal exact-search prototype:

- `transcript` evidence represents what is said during an interval;
- `intra_frame` evidence represents the content of one selected frame;
- `inter_frame` evidence represents motion or change between two frames.

Each modality is searched independently and may use a different model and
embedding dimension. Results are fused only at the rank level. Evidence that
refers to the same scene must use the same `segment_id`, while retaining its
own timestamp interval. This avoids averaging incompatible text, image, and
temporal vector spaces.

Run the checked-in model-independent example:

```bash
cargo run -p youtube_semantic --example multimodal_search -- \
  crates/youtube_semantic/fixtures/multimodal-corpus.jsonl \
  crates/youtube_semantic/fixtures/multimodal-query.json
```

The corpus is JSONL with `id`, `segment_id`, `video_id`, timestamps, `kind`,
and a precomputed `embedding`. The query JSON accepts an embedding and weight
for any combination of `transcript`, `intra_frame`, and `inter_frame`.

For a real one-channel experiment, generate intra-frame vectors with a local
image-text encoder such as SigLIP 2. Generate inter-frame vectors with a model
trained for temporal video-text retrieval; subtracting two image embeddings is
an inexpensive diagnostic baseline, but it should not be described as motion
understanding. Select frames by scene changes, deduplicate them before
inference, and keep exact artifact revisions and licenses in the benchmark
report.

If four static vectors per video fail the judged relevance gate, test in this
order:

1. eight temporal regions with Model2Vec;
2. quantized BGE-small through ONNX, accepting its higher resource use only if
   the quality gain is material;
3. a Matryoshka-trained encoder at 128 or 256 dimensions;
4. Qdrant with scalar quantization and disk-backed payloads only after exact
   scanning becomes a measured bottleneck.

Qdrant documents 4x storage reduction for scalar `int8` quantization and larger
reductions for binary quantization, with accuracy trade-offs. Sentence
Transformers documents ONNX/OpenVINO and quantized CPU inference, but explicitly
recommends benchmarking the actual workload. Model2Vec's published repository
claims large speed and size advantages; the local measurements above are the
relevant evidence for this project, while retrieval quality remains unproven.

## Evaluation gate

Do not ship semantic ranking based on latency alone. Require at least 100
privacy-safe judged queries covering paraphrases, creator names, code terms,
short/low-signal queries, and phrase-sensitive searches. Compare lexical,
semantic, and fused rankings using nDCG@10, Recall@20, and failure slices. Also
measure packaged cold load, warm p50/p95/p99, peak RSS, index size, build time,
and concurrent-query throughput.

The experiment should be promoted only when fused search improves judged
relevance without regressing the existing low-signal and creator-filtered
checks, and when its warm and cold resource budgets fit the deployment target.

## Axiom status

On 2026-07-18, both rotated local Axiom keys authenticated and could enumerate
datasets, but Axiom rejected query execution with HTTP 403. The keys lack the
`query:read` action. No raw search queries were retrieved. After granting a
dataset-scoped query key that action, run:

```bash
scripts/axiom-agent.sh check
scripts/axiom-agent.sh report
```

The existing historical aggregate reports indicate that public tail latency is
larger than backend search time for sampled queries. A semantic sidecar should
therefore be judged primarily on relevance, memory, and concurrency, not on an
assumption that shaving a few milliseconds from retrieval will fix end-to-end
tail latency.

## Sources

- [Model2Vec repository](https://github.com/MinishLab/model2vec)
- [Sentence Transformers inference efficiency](https://www.sbert.net/docs/sentence_transformer/usage/efficiency.html)
- [Sentence Transformers embedding quantization](https://www.sbert.net/examples/sentence_transformer/applications/embedding-quantization/README.html)
- [Qdrant quantization](https://qdrant.tech/documentation/guides/quantization/)
- [Qdrant storage optimization](https://qdrant.tech/documentation/guides/optimize/)
- [Reciprocal rank fusion](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf)
