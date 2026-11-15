# Semantic search for ytsearch

> Historical snapshot from 2026-10-15. Recommendations, prices, production state
> and test results below describe that investigation, not the current release.
> See the [architecture](architecture.md) and [semantic pilot playbook](playbooks/semantic-pilot.md)
> for the implemented system and its operating procedure.

Research and production inspection: **2026-10-15**. Repository baseline: `d0e69c4`.

**Recommendation: retain Raw lexical search, add a semantic passage index, and evaluate turbopuffer first. Use Qdrant if owning the deployment is more valuable than avoiding database operations. Compare existing embedding models before training anything.** A domain reranker is the first custom model I would consider.

This is an investigation, not a performance claim about an implemented system. I inspected Fly with `flyctl`, logged into the running reader over SSH, checked public search behavior, inspected the Rust source, and researched primary vendor/model documentation. No machines were started, stopped, resized, or deployed; no index writes or paid embedding requests were made. The models and alternative engines below have **not** been benchmarked on this corpus.

## What production actually has

The active reader manifest, read over SSH at approximately 11:51 UTC, identifies schema 9, generation **10006**, with **9,405,380 chunks, 41,475 videos, and 69 creators**. Public `/health` and `/metrics` agreed on the chunk count. The generation's index files total **2,083,270,828 bytes**, approximately **1.94 GiB**.

| Surface | Observed state | Allocation / location |
| --- | --- | --- |
| Raw reader `d89303da029328` | Started; health endpoint successful | 4 shared vCPUs, 2,560 MiB, SJC; 20 GB volume |
| Public API `8e7204a7694d28` | Started; Fly health check passing | 1 shared vCPU, 256 MiB, SJC |
| Queue worker `8e426df7e305e8` | Stopped at snapshot | 2 shared vCPUs, 2,048 MiB, SJC |
| Publication builder `80e9345f6ee198` | Stopped at snapshot | 2 shared vCPUs, 2,048 MiB, ORD |
| Reader canary `e820611bde9518` | Stopped | 4 shared vCPUs, 2,560 MiB, SJC |

Reader process RSS was approximately **1.10 GiB**, including **586 MiB anonymous memory**, **540 MiB file-backed memory**, and about **89 MiB swapped process memory**. An earlier cgroup sample recorded **2.23 GiB charged memory** and about **95 MiB swap**, with zero recorded OOM events. Cgroup memory includes cache; it is not equivalent to process RSS or evidence that all memory is unreclaimable. Its reported peak was only a current sample because the kernel peak counter was not usable. The volume used **6.54 GB**, with **13.31 GB available**. These observations give no basis for placing a large model and ANN index on this reader without new capacity measurements.

The public creator directory summed to 41,399 indexed videos, below the generation's 41,475. Treat the generation manifest as the sizing authority; the directory is a separately observed API surface, not an atomic corpus snapshot. Fly's stored health-check output also contained an older chunk count. Neither discrepancy was used to infer lost data.

The checked-in [capacity measurements](capacity.md) describe about 41 searches/day in an earlier 30-day window. I did not remeasure current user traffic. That historical workload favors low operational overhead over maximum ANN throughput.

### Current retrieval path

- [Ingestion chunker](../crates/youtube_ingest/src/chunker.rs) groups caption segments into time windows; the CLI [defaults to 10 seconds](../crates/youtube_cli/src/args.rs). It retains previous/next text and timestamp boundaries.
- [TranscriptChunk](../crates/youtube_ingest/src/model.rs) has an optional embedding field, but the chunker sets it to `None`. This is a schema placeholder, not working vector search.
- [Raw](../crates/youtube_raw_search/src/engine.rs) uses inverted postings, BM25 components, coverage, positions, and other lexical ranking logic. [SearchParams and SearchBackend](../crates/youtube_search_service/src/lib.rs) contain no vector-query contract; `SearchBackend::search` is synchronous.
- The [public API](../crates/youtube_api/src/main.rs) groups timestamp hits into video cards and emits `mode: "lexical"`. [Frontend types](../src/types.ts) only accept that mode. There is no semantic toggle to enable.
- The API owns public sort policy, including newest-first handling for low-signal queries. The [existing immutable-generation and credential boundaries](architecture.md) remain valuable.

### Live search spot checks

These were single requests, not latency percentiles or a relevance evaluation:

| Query / scope | Observation |
| --- | --- |
| `I`, Theo | First three videos had publication dates October 14, 13, and 12. Newest-first behavior worked. |
| `fable on high`, Theo | “A proper guide to Fable 5” ranked second, with a snippet discussing high reasoning. |
| `avoiding unnecessary abstractions`, Theo | Three videos; first hit discussed avoiding unnecessary type checks. Other hits discussed conversions and blocking. This looks like word overlap rather than the intended software-design concept. |
| `why do developers make simple software complicated`, Theo | Six videos; first result was “Why is OpenAI buying a VS Code fork???”. Its returned moment discussed winning developers. |
| Same natural-language query, global | Eighteen videos; first result was a Next.js/Prisma talk with a to-do-app snippet. |

The latter examples motivate semantic retrieval. They do not prove a specific embedding model would improve results, or that a relevant answer necessarily exists in the corpus. Responses and sanitized runtime evidence are summarized in [the evidence file](semantic-search-evidence-2026-10-15.json).

## Separate the three decisions

An **embedding model** maps queries and passages into vectors. A **vector engine** retrieves nearby vectors under filters. A **reranker** reads the query and a small set of candidate passages to improve their order. Buying turbopuffer addresses retrieval infrastructure and optionally embedding integration; training a model addresses representation or ranking quality. They are complementary choices.

Keep lexical retrieval for names, error strings, quotes, and literal product terms. Add semantic candidates for paraphrases and concepts. A reranker can improve ordering but cannot recover a passage absent from every candidate list. This two-stage distinction is explained in the [Sentence Transformers retrieval guide](https://www.sbert.net/examples/sentence_transformer/applications/retrieve_rerank/README.html).

## Engine options

The fit assessments below are my judgments for this project, not vendor benchmark results.

| Option | Why it fits | Cost / operational tradeoff | Decision |
| --- | --- | --- | --- |
| **turbopuffer** | Managed vector retrieval with filters; supports BM25 and hybrid fusion if later useful. Rust API integration can use existing `reqwest`/`serde`. | Recurring usage bill, external network hop, cold-cache tail latency. Entry minimum is currently $16/month; this is a floor, not a full-corpus quote. | **First managed pilot.** Preserve Raw and use turbopuffer for semantic passages initially. |
| **Qdrant** | Rust engine and official Rust client; filter-aware retrieval and configurable vector memory placement/quantization. | Self-hosting adds another volume, backups, upgrades, recovery, and capacity planning. Managed Qdrant is an alternative deployment of the same candidate. | **First self-hosted choice.** Run separately from the Raw reader. |
| **LanceDB** | Rust integration, local/object-storage-oriented data, ANN indexes, filtering, and hybrid search. | Requires ownership of indexing, compaction, serving, and persistence behavior in the chosen deployment. Embedded convenience does not remove these tasks. | Strong third option if file-based datasets and offline evaluation become central. |
| **Cloudflare Vectorize** | Already adjacent to the Worker; low dimension-based storage price; HTTP API allows Rust integration. | Top-k limits constrain candidate overfetch; passage text must still be hydrated. Changes also cross the Worker/Fly boundary. | Serious low-cost challenger. Pilot if 100 IDs per vector query provide enough video diversity. |
| **Postgres + pgvector** | SQL filters, joins, transactions, exact search and ANN. | There is no existing Postgres deployment to reuse. ANN plus selective creator filtering needs tuning and recall checks. | Choose if Postgres is also becoming the canonical application database. |
| **Raw + USearch** | Keep control of Rust serving; USearch supplies ANN, persistence/viewing, quantization, and Rust filter predicates. | We must build metadata lookup, update/delete safety, snapshots, versioning, compaction, and recovery around the ANN library. | Best route if we explicitly want our own search infrastructure. Do not implement HNSW from scratch. |

**turbopuffer details.** It uses object storage with an NVMe cache. Its published cold and warm figures describe its own workloads, not ours; low historical traffic makes cold performance especially relevant. Start by measuring an Oregon region from SJC. Its native embedding integration now accepts source text and handles query/document embedding, but changing a model requires re-ingestion. Bring-your-own vectors remains possible. Keep one corpus namespace with creator/video filter attributes initially, because global search must work without querying 69 namespaces. Native model availability must be checked in the chosen region. Sources: [architecture](https://turbopuffer.com/docs/architecture), [regions](https://turbopuffer.com/docs/regions), [hybrid retrieval](https://turbopuffer.com/docs/hybrid), [native embeddings](https://turbopuffer.com/docs/embedding).

**Qdrant details.** Index `creator_id` and `video_id` as payload fields. Preserve global search as well as creator-filtered search: do not copy a tenant-only setup that disables the global HNSW graph. Benchmark quantized candidates against full-vector exact neighbors; compression and filtering can each affect recall. For a reduced passage corpus, a separate 4–8 GiB test machine is a reasonable experimental starting point, **not a measured production requirement**. Sources: [multitenancy](https://qdrant.tech/documentation/tutorials/multiple-partitions/), [memory tiers](https://qdrant.tech/documentation/ops-configuration/memory-tiers/), [official Rust client](https://github.com/qdrant/rust-client).

**LanceDB details.** It supports multiple IVF/quantization index choices and full-vector refinement. Filtering can happen before retrieval; hybrid searches can apply the same filter to both branches. It deserves a pilot rather than an assumption that embedded means small or slow. Sources: [vector search](https://docs.lancedb.com/search/vector-search), [filtering](https://docs.lancedb.com/search/filtering), [hybrid search](https://docs.lancedb.com/search/hybrid-search).

**Other viable families.** [Pinecone](https://docs.pinecone.io/guides/search/hybrid-search) provides managed hybrid patterns; [Weaviate](https://docs.weaviate.io/weaviate/search/hybrid) combines BM25F and vectors; [Milvus](https://milvus.io/docs/overview.md) is another vector platform; [Vespa](https://docs.vespa.ai/en/querying/nearest-neighbor-search.html) supports vector retrieval and configurable ranking. I would revisit these if a unified search platform or much larger scale becomes the goal; none establishes enough project-specific advantage to justify four more pilots now. [Elasticsearch](https://www.elastic.co/docs/solutions/search/vector/knn) and [OpenSearch](https://docs.opensearch.org/latest/vector-search/) also belong on that longer list when an existing deployment can be reused, which is not the present architecture. [sqlite-vec](https://github.com/asg017/sqlite-vec) is worth considering for tiny local experiments, but I would require full-scale retrieval measurements before choosing it for millions of passages. [USearch's actual Rust API](https://github.com/unum-cloud/usearch) makes a more focused custom ANN experiment possible.

## The highest-value change: semantic passages

Do not assume that the timestamp display unit must also be the embedding unit. Ten seconds is useful for a YouTube jump link but often cuts through a thought.

Retain the current chunks. Derive additional passages from consecutive caption segments, testing **30, 60, and 90 seconds**, with sentence-aware boundaries, a tokenizer-based maximum, and modest overlap. For embedding, test a short title/chapter prefix plus passage text. Avoid repeating a full title in every tiny chunk. Preserve technical spelling; do not rely on an LLM to rewrite the entire corpus.

Each semantic record should carry a stable ID, creator/video identity, start/end times, source chunk references, transcript revision, passage-builder version, embedding model revision, dimension, and text hash. Include the visible source text or a durable lookup location. At retrieval time, resolve it to original timestamped captions. If a semantic-only result has no literal query words, it must still hydrate successfully and show the passage; it must not depend on another lexical match. Choose a more precise caption timestamp only when there is evidence for it, and never manufacture highlighted words.

There are cheaper or more elaborate retrieval designs worth separating from the passage recommendation:

| Design | Assessment for this project |
| --- | --- |
| One vector per existing chunk, with neighboring text as context | Most direct mapping to current IDs; useful pilot baseline. Still creates 9.4M vectors and repeatedly embeds overlapping context. |
| One title/summary vector per video, then search inside selected videos | Only about 41k first-stage records, but a single vector can miss a brief topic in a long video. Compare as an auxiliary candidate source; do not make it the only gateway to passage retrieval. |
| Lexical retrieval plus a reranker, without a vector database | Cheapest way to test whether the immediate problem is ordering. Cannot recover paraphrases with no lexical candidate. Include this baseline before attributing every gain to vectors. |
| Query expansion, aliases, or an LLM rewrite into lexical queries | Can help caption spelling and vocabulary mismatch. Test optional expansion against the original query; generated terms can drift, and an LLM adds latency. No reason to require an LLM call for every search initially. |
| Contextual/late chunking and multi-vector retrieval | Potential later experiments for long context and topic boundaries. Require compatible model/runtime support and larger evaluation/serving budgets. Start with explicit passages whose timestamps and revisions are easy to audit. |

These are design hypotheses to evaluate, not measured winners.

I sampled **500 of 608 local cached transcripts**, selected with random seed 42, and applied the current chunker's time-window grouping rule without overlap:

| Window | Records in sample | Ratio to 10-second count | Illustrative full-corpus projection |
| --- | ---: | ---: | ---: |
| 10 seconds | 323,131 | 1.000 | 9.405M observed chunks |
| 60 seconds | 39,435 | 0.122 | 1.148M passages |
| 90 seconds | 25,903 | 0.080 | 0.754M passages |

**These projections are not corpus measurements.** The local cache is biased toward long videos, and overlap/sentence boundaries increase the counts. They show why passage sizing should precede a database purchase. Embedding fewer records reduces vector storage and title repetition; it does not remove the need to process the transcript's words.

### Storage and money

For all **9,405,380** current chunks, vectors alone would occupy:

| Dimensions | float32 | float16 | int8 |
| --- | ---: | ---: | ---: |
| 256 | 8.97 GiB | 4.48 GiB | 2.24 GiB |
| 384 | 13.45 GiB | 6.73 GiB | 3.36 GiB |
| 768 | 26.91 GiB | 13.45 GiB | 6.73 GiB |
| 1,024 | 35.88 GiB | 17.94 GiB | 8.97 GiB |

Formula: `records × dimensions × bytes_per_dimension / 2^30`. These are payload sizes, not total RAM or disk requirements: add ANN structures, original vectors retained for rescoring, source text, metadata, WAL, staging, backups, and rollback copies. Quantization needs quality verification. Dimension reduction should use a model designed for it; arbitrary truncation is not a general substitute.

At the sample-projected 60-second count, 256-dimensional float32 payload is approximately **1.10 GiB**. That is much more manageable, but it still does not establish that it belongs on the existing 2.5 GiB reader.

- **turbopuffer:** [current pricing](https://turbopuffer.com/pricing) lists a **$16/month Launch minimum** and $256/month Scale minimum. Usage, writes, embeddings, and query accounting determine the actual bill. The pricing FAQ bills queried data using at least 1.28 GB per namespace/query, or the namespace size when greater, with size-tier discounts. A creator filter is not a promise of a proportionately smaller bill. The live calculator did not expose usable unit-rate values during inspection, so I am not presenting an invented monthly quote. Price the pilot with its returned billing usage and actual namespace bytes.
- **Vectorize:** at **1M vectors × 256 dimensions**, published paid-plan rates imply approximately **$0.12/month storage** after its 10M stored-dimension allowance. A full load followed by queries has additional queried-dimension charges; a 1M-vector load plus 10,000 queries is approximately **$2.09** after the 50M queried-dimension allowance. Workers subscription and embedding inference are additional, and existing account usage may consume allowances. At **9.405M × 384**, storage is approximately **$1.80/month**, with about **$35.66** queried-dimension cost for that full load plus 10,000 queries. These are calculations from published formulas, not a complete service quote. [Pricing](https://developers.cloudflare.com/vectorize/platform/pricing/).
- **Embedding backfill:** as a planning scenario, **250M–600M input tokens** costs **$5–$12** at $0.02/M, **$15–$36** at $0.06/M, or **$30–$72** at $0.12/M, before credits, discounts, retries, and storage. This token range is an assumption, not tokenizer-measured production data. Overlap and title prefixes matter. [Voyage pricing](https://docs.voyageai.com/docs/pricing) provides those three rates for its current lite/standard/large family. Native integration billing should be checked separately rather than assuming direct-provider credits apply.
- **Self-hosting:** quote an additional machine, volume, snapshots, and temporary backfill compute after the pilot records actual peak use. No new Fly machine was provisioned or priced as a verified configuration in this investigation.

Vectorize currently permits **20M vectors/index**, **1,536 dimensions**, and top-k **100 IDs without values/metadata** or **50 with them**. It therefore fits the corpus's count today, but 100 IDs can collapse into few unique videos. Test that constraint before choosing it. [Limits](https://developers.cloudflare.com/vectorize/platform/limits/).

## Models to compare, and how Rust fits

| Candidate | Role in evaluation | Practical concern |
| --- | --- | --- |
| **Voyage 4 lite / 4** | Managed quality/latency baseline; test 256 and 512 dimensions. | External embedding call, token cost, region and tail latency. |
| **Qwen3-Embedding-0.6B** | Open-weight candidate for domain adaptation; test compact output dimensions. | Model weights and activations need a separate resource budget. CPU latency must be measured. |
| **EmbeddingGemma 300M** | Smaller local candidate; test 256/512/768 outputs. | Validate task prefixes, export/runtime parity, and Gemma license/access terms. |
| **BGE-small-en-v1.5** | Cheap English CPU baseline with 384 dimensions. | Older baseline; limited input window and possibly lower domain quality. |
| **BGE-M3 / learned sparse retrieval** | Challenger if dense-only semantic misses technical terms. | More computation or index machinery; Raw already provides a strong lexical branch. |

Voyage's [model documentation](https://docs.voyageai.com/docs/embeddings) supports reduced dimensions and distinguishes document/query input. Qwen's [model card](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B) lists Apache-2.0, 0.6B parameters, instruction-aware queries, and output dimensions up to 1,024. EmbeddingGemma's [model card](https://huggingface.co/google/embeddinggemma-300m) lists a 2,048-token context and Matryoshka output sizes. BGE's [model card](https://huggingface.co/BAAI/bge-small-en-v1.5) is the baseline reference. Generic leaderboard scores are not a substitute for noisy-caption retrieval judgments.

There is no requirement to build a Python production service. [FastEmbed-rs](https://github.com/Anush008/fastembed-rs) currently lists BGE, EmbeddingGemma, rerankers, and Qwen3 through its Candle feature. It uses ONNX Runtime for its main inference path. Pin released crate versions and model revisions; README support is not proof that an untested export works on our Fly image. Custom models can use an appropriate ONNX export through `ort`/FastEmbed or a verified [Candle](https://github.com/huggingface/candle) implementation. Match the reference tokenizer, pooling, prompts, normalization, dimensions, and numerical outputs before evaluating retrieval.

Keep expensive inference outside the 256 MiB public API process and outside Raw's search permit pool. For the first managed pilot, typed asynchronous HTTP calls from Rust are sufficient. For local inference, use a separate bounded worker/service or dedicated blocking execution with explicit concurrency limits; do not block Tokio's request executor or an entire shared search semaphore. Cache query embeddings by normalized query plus model/prompt revision.

## Should we train our own model?

**Fine-tuning is plausible; training from scratch is the wrong starting investment.** Millions of transcript passages are not millions of labeled query–answer pairs. A new base encoder would add pretraining, tokenizer, GPU, multilingual, and retrieval-supervision work before demonstrating any advantage over existing models.

I would consider custom learning in this order:

1. **Domain reranker or small ranking model.** Learn which retrieved moments actually answer a query. Combine semantic relevance with exact-term and title evidence without allowing video length or popularity to dominate. This can improve ordering without re-embedding the corpus.
2. **Fine-tune an existing encoder.** Build query/positive-passage pairs with hard negatives mined from lexical and dense retrieval. A starting experiment could use 10k–50k carefully checked or teacher-generated pairs, with a separate human-labeled test set. Those counts are an experiment design, not an assurance of sufficient training data.
3. **Distill a stronger teacher into a compact local encoder.** This is useful if the managed baseline wins quality but its latency or dependency is unacceptable.
4. **Learned sparse expansion or late interaction.** Test if dense plus Raw still fails on technical vocabulary. Multi-vector/token representations cost substantially more storage and serving work; defer until measured failures justify them.
5. **Pretrain from scratch.** Revisit only with a demonstrated, persistent quality gap and a dedicated ML budget.

Training can happen offline in Python/PyTorch/Sentence Transformers and inference can remain Rust. [Sentence Transformers' training documentation](https://www.sbert.net/docs/sentence_transformer/training_overview.html) supports pair/triplet supervision, ranking losses, and held-out evaluation. Use human-checked positives and guard against false negatives: neighboring moments can both answer a query. Split by video and include held-out creators and a temporal split so near-identical captions do not leak between train and test. Synthetic queries alone can reward the teacher's phrasing rather than real user needs.

## Integration and acceptance plan

The following is a proposed implementation sequence, not work already performed:

1. **Freeze evaluation evidence.** Capture a generation-pinned transcript snapshot, candidate IDs, exact text, and query judgments. Start with 200–300 human-judged queries across several creator/topic types. Include exact quotes, names, code terms, paraphrases, noisy captions, absent answers, creator filters, within-video search, and the two regression queries above.
2. **Settle chunking and models cheaply.** On the same representative subset, compare lexical-only, lexical plus reranking, dense-only, lexical+dense fusion, and fusion plus reranking. Use exact vector search on that subset to isolate model/chunking quality from ANN approximation. Then compare ANN engines using the same vectors, filters, and recall target.
3. **Pilot storage.** Start with turbopuffer; run Qdrant against identical exported vectors if self-hosting matters. Bring-your-own vectors makes this comparison fair. Native turbopuffer embeddings are a separate convenience experiment. Add Vectorize if its top-k cap survives the candidate-diversity check. Do not ingest the whole corpus into several vendors before this gate.
4. **Add a versioned semantic data path.** Derive passages from a consistent transcript revision. Use an idempotent outbox/job stream with its own cursor; do not let a second worker consume the authoritative JSONL queue. `update` embeds new/changed content while preserving unchanged work. A `rewipe` or deletion must retire all old semantic passages for the affected video/creator, including records whose sequence IDs changed. Retries need per-video revision ordering so a late job cannot resurrect deleted content.
5. **Integrate retrieval.** At relevance search, run Raw and filtered dense retrieval concurrently with deadlines. A starting budget is approximately 100 lexical and 100 dense candidates, tuned for unique-video coverage. Raw currently groups before pagination, so obtain enough per-video moments intentionally. Normalize both branches to comparable source moments/passages before fusion; do not combine one passage score with eight duplicate chunk votes. Use rank fusion initially, then optionally rerank 30–50 deduplicated passages.
6. **Preserve the response contract deliberately.** Group and diversify by video, retain multiple useful timestamps, and hydrate vector-only matches by ID/revision. Add explicit retrieval provenance and `lexical`/`hybrid`/`semantic` result modes. Semantic top-k does not establish an exact total video/moment count: mark totals approximate and define a stable bounded result window or cursor. Cache keys must include filters, sort, corpus/semantic revision, model, and retrieval mode. Preserve API ownership of sorts; keep low-signal newest-first and exhaustive chronological/engagement sorts on their existing lexical path initially, so ANN top-k does not silently change sort semantics.
7. **Shadow before enabling.** Log model/index version, dense coverage, candidate diversity, embedding/retrieval/rerank durations, cost, fallback rate, and judged outcome. A vector timeout should return lexical results and report the actual mode. Read-only credentials stay on the search side; embedding/index-write credentials stay with the ingestion worker. Do not add secrets to browser variables. Changing model versions requires a separate namespace/index and an atomic read switch after complete coverage checks.

Proposed acceptance gates: at least **10% relative nDCG@10 improvement on the semantic-intent slice**, no material loss on exact-term/quote queries, **zero filter violations**, and explicit agreement that the timestamp supports the result. Measure passage Recall@100, unique-video Recall@20, MRR, no-answer behavior, and ANN recall against exact neighbors separately. Add load, cold-cache, repeated-query, selective-creator, and within-video tests. An initial target is **API p95 under 500 ms for warm hybrid search**, with cold latency reported separately; this is a target, not a prediction. No-traffic synthetic benchmark scores alone should not decide the system.

If this pilot wins, the initial production direction is **Raw + turbopuffer passages + the best measured existing encoder**, with a reranker enabled only when its relevance improvement pays for its latency. If managed cost or cold latency fails the gate, use the same model/passages with **Qdrant**. Build **Raw + USearch** only if maintaining retrieval infrastructure is an explicit project goal. Train a domain model when the evaluation identifies a model-specific gap that chunking, fusion, or reranking does not resolve.

## Reproduction and limits

The audit used `flyctl machine list` for each of the three apps, the existing [resource snapshot script](../scripts/fly-machine-resource-snapshot.sh), and `flyctl ssh console --app ytsearch-raw-production --machine d89303da029328` for bounded reads of `/proc`, the active-generation pointer, and the active generation manifest. Machine configuration was projected to resource fields; credentials and secret files were not inspected. Public GETs covered `/health`, `/metrics`, `/api/youtubers`, `/api/index-status`, and the five searches described above.

The local cache grouping experiment used the existing chunker's rule: add a valid segment while `segment.end_ms - group.start_ms <= window_ms`, otherwise start another group. It did not tokenize with any embedding model, evaluate embeddings, build ANN indexes, train, or load-test production. The evidence snapshot is descriptive and time-bound. Production can change independently while the report is being written.

Artifact verification: `bun run check:docs` and `bun run check:precommit` passed. `bun run check` passed its Rust tests and completed all 18 browser tests (the lazy-chunk test passed on retry), then failed the existing public-surface gate with four findings: `RawIndexAdminClient::publish`, `delete_video`, and `operation` are unreachable public methods, and `ADMIN_REQUEST_TIMEOUT` has unnecessarily broad visibility. The affected [HTTP client](../crates/youtube_index_store/src/http.rs) and [library](../crates/youtube_index_store/src/lib.rs) match the baseline commit; no Rust code was changed for this report. Later full-suite steps were not reached. The interactive report summary was also checked at desktop/mobile widths and its dimension/encoding controls verified.

throughput checkpoint: n/a, read-only investigation
