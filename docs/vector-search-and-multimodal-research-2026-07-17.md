# Vector search alternatives and local multimodal research

Date: 2026-07-17

## Decision

Do not replace Typesense lexical search. If semantic retrieval is tried again,
run **Qdrant as an isolated shadow candidate store** and keep Typesense as the
lexical source of truth. Start Qdrant with exact, creator-filtered search. Enable
HNSW only after a production-shaped corpus shows a useful latency benefit at an
acceptable measured recall.

For video frames, start with a separate **SigLIP 2 B/16 frame index plus OCR**.
Keep transcript, OCR, and frame embeddings as independently retrievable evidence
at timestamps and fuse ranked lists. Do not average all modalities into one
video vector, and do not begin with an 8B video-language model.

These are experiment recommendations, not a claim that semantic relevance has
been established. The earlier [Typesense semantic experiment](semantic-experiment-2026-07-13.md)
showed that a small MiniLM fixture could look plausible while generalizing
poorly. Engine latency cannot repair a weak embedding model or evaluation set.

## Reproducible local retrieval benchmark

The repository now contains an isolated benchmark:

```bash
scripts/benchmark-qdrant-vector.sh
```

It starts pinned `qdrant/qdrant:v1.15.4`, generates deterministic vectors,
creates a keyword payload index, waits for HNSW construction, measures exact and
approximate queries against identical inputs, prints client-observed percentiles
and recall@10, then deletes the container. It never contacts staging.

Recorded on this development machine:

```text
engine=qdrant vectors=20000 dimensions=384 queries=100 top_k=10 ingest_ms=4810
case=unfiltered exact_p50_us=3804 exact_p95_us=4581 ann_p50_us=3637 ann_p95_us=4271 recall_at_10=0.8050
case=creator-filtered exact_p50_us=1865 exact_p95_us=2605 ann_p50_us=1890 ann_p95_us=2617 recall_at_10=1.0000
container=ytsearch-qdrant-bench-139062 memory=414.5MiB / 30.74GiB cpu=0.32%
```

Interpretation:

- HNSW saved only about 0.17 ms at p50 and 0.31 ms at p95 unfiltered, while
  losing roughly 19.5% of the exact top ten with `hnsw_ef=64`.
- The creator filter reduced the candidate set from 20,000 to about 200. Exact
  search was already about 1.9 ms p50 and was marginally faster than ANN.
- The 414.5 MiB number includes Qdrant process/index overhead and an idle Docker
  snapshot after the run. It is not a minimum production allocation.
- Synthetic random vectors test index mechanics, not transcript relevance.
  Loopback HTTP also excludes public network latency, TLS, embedding inference,
  lexical retrieval, fusion, grouping, and response serialization.
- One machine/run is not a capacity claim. Run at 100k, 1M, and the actual
  embedding dimension before choosing index parameters.

Useful larger run:

```bash
VECTOR_COUNT=100000 QUERY_COUNT=250 scripts/benchmark-qdrant-vector.sh
```

The benchmark uses exact Qdrant results as ground truth. Every ANN configuration
must report recall along with latency; QPS without recall is not admissible.

## Alternatives considered

| System | Fit for ytsearch | Important tradeoff | Recommendation |
| --- | --- | --- | --- |
| Qdrant | Rust service, payload indexes, dense/sparse and multivector queries | Adds a second stateful service; HNSW needs recall tuning | First shadow experiment |
| pgvector | Excellent if Postgres is already the metadata source | ANN filtering is generally post-scan; iterative scans mitigate missing filtered results | Reconsider only if Postgres is adopted |
| Vespa | Strong filtered ANN, ranking expressions, tensors and multivectors | Largest operational and schema/ranking learning cost here | Best for a future unified large search tier, not now |
| LanceDB | Embedded/local workflow, columnar multimodal data and IVF/PQ family indexes | Less natural as the current public multi-replica search service | Good offline frame-research store |
| Milvus | Mature distributed vector system designed for scale | Operational surface is disproportionate to the current corpus/team | Reject for the present stage |
| OpenSearch | Hybrid search and existing search-operations ecosystem | JVM/service footprint and another complete lexical engine | Only if consolidating onto OpenSearch |

Why Qdrant is the most relevant first test:

- Its documentation describes payload indexes used during HNSW traversal and
  filterable vector indexes, which maps directly to `youtuber=@handle` queries.
- It supports sparse vectors, dense vectors, fusion and multivectors without
  forcing transcript and frame representations into one vector.
- It is a narrower operational addition than Vespa, Milvus, or OpenSearch.

Why pgvector is not the default despite its simplicity: the official pgvector
documentation explains that approximate-index filtering happens after the index
scan. Since 0.8.0, iterative scans can continue until enough filtered rows are
found, and partial indexes can help low-cardinality filters. That is workable,
but ytsearch does not otherwise need Postgres, and creator filtering is central
rather than incidental.

Primary references:

- [Qdrant search concepts and query API](https://qdrant.tech/documentation/search/)
- [Qdrant filtering and payload indexes](https://qdrant.tech/documentation/concepts/filtering/)
- [pgvector implementation and index guidance](https://github.com/pgvector/pgvector)
- [Vespa approximate nearest-neighbor/HNSW documentation](https://docs.vespa.ai/en/querying/approximate-nn-hnsw.html)
- [Milvus SIGMOD system paper](https://doi.org/10.1145/3448016.3457550)
- [HNSW paper](https://arxiv.org/abs/1603.09320)
- [LanceDB benchmark methodology](https://docs.lancedb.com/enterprise/benchmarks)

Vendor benchmarks are useful for feature discovery, not cross-system ranking:
hardware, datasets, filters, cache state, concurrency, quantization, recall and
client placement differ. A fair next comparison must use one checked-in corpus,
one query set, one ground-truth evaluator, fixed CPU/RAM limits and cold/warm
runs.

## Local embeddings for transcript text and video frames

### What each signal can retrieve

| Signal | Finds well | Misses | Initial local approach |
| --- | --- | --- | --- |
| Transcript text | Spoken concepts, names, explanations | Unspoken visuals, diagrams, code on screen | Keep lexical first; evaluate a stronger text embedding separately |
| OCR text | Slides, terminals, code, product UI | Objects/actions; stylized or tiny text | Scene/keyframe OCR into timestamped lexical fields |
| Frame image embedding | Objects, settings, visual concepts, broad UI appearance | Motion and event order; exact text | SigLIP 2 B/16 per selected frame |
| Short clip embedding | Actions and temporal relationships | Higher compute/storage cost | Research only after frame baseline |
| Audio embedding | Music, sounds, non-speech events | Spoken semantics already covered by transcript/ASR | Optional ImageBind experiment |

### Model shortlist

1. **SigLIP 2 B/16** is the recommended frame baseline. The paper reports
   improved image-text retrieval and localization over SigLIP at every released
   scale; the B model is 86M parameters. The official code/checkpoints are
   Apache-2.0/CC-BY as documented by the project. Test 224 or 256 resolution
   before larger variants. It is an image-text encoder, not a temporal model.
2. **OpenCLIP/CLIP** is the control. CLIP established zero-shot image-text
   retrieval and has exceptionally broad runtime support. A result is more
   credible if the newer model beats this simple baseline on ytsearch queries.
3. **ImageBind** is the next multimodal experiment when audio retrieval matters.
   Its paper aligns image, text, audio, depth, thermal and IMU in one space. Do
   not adopt solely because it supports more modalities; measure the actual
   sounds users search for and review checkpoint licensing separately.
4. **InternVideo2 small/distilled encoders** are a later temporal baseline. The
   paper unifies masked video modeling, cross-modal contrastive learning and
   next-token prediction; the project offers smaller distilled variants as well
   as much heavier models. The 8B chat models are inappropriate for first-stage
   bulk indexing.

Primary model research:

- [CLIP paper](https://arxiv.org/abs/2103.00020)
- [SigLIP paper](https://arxiv.org/abs/2303.15343)
- [SigLIP 2 paper](https://arxiv.org/abs/2502.14786) and [official checkpoints](https://github.com/google-research/big_vision/blob/main/big_vision/configs/proj/image_text/README_siglip2.md)
- [ImageBind paper](https://arxiv.org/abs/2305.05665)
- [InternVideo2 paper](https://arxiv.org/abs/2403.15377) and [official repository](https://github.com/OpenGVLab/InternVideo)

### Proposed frame pipeline

1. Reuse downloaded video only during indexing; never fetch video in a public
   search request.
2. Detect scene changes, then cap sampling to one representative frame per
   5–10 seconds. Also retain frames near transcript chunk boundaries. Fixed 1
   FPS sampling is too costly for long creator archives and highly redundant.
3. Run OCR on selected frames. Store normalized OCR text, video ID, timestamp,
   frame hash, model/version and confidence. Deduplicate near-identical frames
   with a perceptual hash before inference.
4. Batch frame embedding locally. Store one vector per frame, not an average per
   video. Quantization is a later measured optimization.
5. At query time retrieve lexical transcript, lexical OCR, semantic transcript,
   and visual-frame candidates independently. Fuse ranks with the existing RRF
   helper; return the evidence type and timestamp so relevance is explainable.
6. Collapse adjacent hits only after retrieval. A frame hit should link to its
   own timestamp even if it joins a transcript video card.

This layout supports queries such as “the purple architecture diagram,” “code
showing createSignal,” or “the segment with a race car” without weakening exact
spoken-term search.

## Evaluation gate

Create at least 100 judged queries before shipping any semantic or visual mode:

- 40 spoken-concept/paraphrase queries;
- 20 exact names/code terms where lexical retrieval must not regress;
- 20 visual-only or OCR queries;
- 10 low-signal queries such as `I`;
- 10 creator-filtered and cross-creator queries.

Record relevance at the timestamp level and report Recall@10, nDCG@10, MRR,
zero-result rate, p50/p95/p99 end-to-end latency, embedding latency, index bytes
per frame/chunk, and peak indexing memory. Compare lexical-only, each modality
alone, and fused retrieval. Use bootstrap confidence intervals or paired random
tests before calling a small metric difference an improvement.

Production gates:

- no statistically credible lexical regression on exact/code/name queries;
- visual/OCR queries improve over transcript-only retrieval;
- p95 added search latency stays within the API budget;
- all model and dataset licenses are recorded by exact artifact revision;
- shadow mode first, feature flag second, public default only after judgment.

## Next experiment, in order

1. Build the judged query set from privacy-safe/manual cases.
2. Run the Qdrant benchmark at the actual vector count and dimension with fixed
   container CPU/RAM limits; sweep exact and `hnsw_ef` values.
3. Index one representative channel with scene-selected frames, OCR and SigLIP 2
   B/16; do not ingest the full archive yet.
4. Measure frame extraction, OCR, embedding throughput and storage as well as
   retrieval quality.
5. Only then compare a temporal InternVideo2 encoder on the visual queries that
   the frame baseline misses.
