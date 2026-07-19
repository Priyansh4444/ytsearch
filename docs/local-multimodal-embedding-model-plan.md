# Local multimodal embedding model plan

Status: initial discussion draft  
Scope: experimental `youtube_semantic` subcrate  
Owner review: requested

## Purpose

Build a local-first embedding pipeline that can retrieve:

- spoken concepts from transcripts;
- visible content within a frame (`intra_frame`);
- actions or changes across frames (`inter_frame`);
- combined evidence at an explainable video timestamp.

The experiment must remain optional. It must not increase production API memory,
indexing cost, or deployment complexity until relevance and resource gates pass.
Typesense lexical search remains the source of truth and fallback.

## Questions for the owner

Please write comments directly below each question. Short answers are enough;
uncertainty is useful and can be recorded as “test both.”

### 1. Product behavior

1. What should a user be able to type that fails today? Please give 10–20 real
   example queries, including a few visual-only and motion/action queries.

   **Owner comments:**

2. Should multimodal search be a visible mode, silently improve normal search,
   or remain an internal experiment initially?

   **Owner comments:**

3. Should results point to the nearest frame, the beginning of a detected
   scene, or the best transcript timestamp near the visual match?

   **Owner comments:**

4. Is cross-creator search essential for the first version, or may the first
   useful version search one selected creator/channel at a time?

   **Owner comments:**

### 2. Content and permissions

1. Which channels or videos may be downloaded locally for frame extraction?
   Is this limited to owned/approved creators, public videos, or the entire
   indexed catalog?

   **Owner comments:**

2. May extracted frames be retained, or must they be deleted after embeddings,
   OCR, hashes, and timestamps are produced?

   **Owner comments:**

3. May thumbnails or representative frames be returned to users, or should the
   product only deep-link to YouTube at the matching timestamp?

   **Owner comments:**

4. Are private, members-only, age-restricted, unlisted, or cookie-authenticated
   videos explicitly out of scope?

   **Owner comments:**

### 3. Local hardware and runtime

1. What is the weakest machine that must perform indexing? Include OS, CPU,
   RAM, GPU model/VRAM if present, and available disk.

   **Owner comments:**

2. Must query embedding run on CPU, or may it require a local GPU? Should Apple
   Silicon, Linux CUDA, and CPU-only Linux all be supported?

   **Owner comments:**

3. What are acceptable limits for:

   - initial model download;
   - peak indexing RAM;
   - disk used per 100 hours of video;
   - time to index one hour of video;
   - warm query latency;
   - cold-start latency?

   **Owner comments:**

4. Is an always-warm semantic process acceptable, or must it be able to load on
   demand and shut down when idle?

   **Owner comments:**

### 4. Quality and model constraints

1. Is English-only acceptable initially? Which languages must eventually work?

   **Owner comments:**

2. How important are code, terminal text, diagrams, UI screenshots, faces,
   objects, gestures, and longer actions? Rank these if possible.

   **Owner comments:**

3. Are permissive open-source code and weights mandatory? Is an Apache-2.0 or
   MIT runtime with separately licensed model weights acceptable?

   **Owner comments:**

4. Is any network inference allowed after model installation, or must both
   indexing and queries work fully offline?

   **Owner comments:**

5. What quality trade-off is acceptable for a model that uses substantially
   less RAM? For example, would a 5% relevance loss be acceptable for a 4x
   memory reduction?

   **Owner comments:**

### 5. Operations and cost

1. Where should embeddings live during the experiment: local files, an embedded
   database, the existing Typesense service, or a separate vector service?

   **Owner comments:**

2. Must indexing be resumable after interruption, and should model-version
   changes trigger a full rebuild or permit two versions side by side?

   **Owner comments:**

3. How many concurrent users and semantic requests should the first deployed
   version support?

   **Owner comments:**

4. What monthly cost ceiling would make a deployed semantic service worthwhile?

   **Owner comments:**

## Initial technical proposal

This is a starting hypothesis, not a final model selection.

### Retrieval channels

| Channel | Initial representation | Why it is separate |
| --- | --- | --- |
| Transcript | Model2Vec retrieval model or quantized small transformer | Cheapest conceptual recall; lexical search remains available for exact text |
| OCR | OCR text indexed lexically and optionally embedded with transcript text | Exact code and UI text should not depend only on image embeddings |
| Intra-frame | SigLIP 2 B/16 image embedding | Aligns a text query with visible content in a single selected frame |
| Inter-frame | Small temporal video-text encoder, tested against a frame-difference baseline | Actions and state changes cannot reliably be inferred from one frame |

Each channel retrieves candidates independently. The system fuses ranks using a
shared `(video_id, segment_id)` rather than averaging incompatible vectors.
Every returned match keeps its evidence kind and timestamp interval.

### Proposed local indexing pipeline

1. Use an already permitted local video file; never download video in a public
   search request.
2. Detect scene boundaries and sample a representative frame. Add a conservative
   maximum interval so long static scenes are still represented.
3. Deduplicate frames with a perceptual hash before running OCR or embeddings.
4. Run OCR and store normalized text with confidence and bounding-box metadata.
5. Batch intra-frame inference locally.
6. Form short overlapping clips around scene boundaries and meaningful visual
   changes. Batch inter-frame/clip inference separately.
7. Align frames, clips, OCR, and transcript chunks into stable scene IDs.
8. Write an immutable manifest containing video hash, timestamps, model artifact
   revision, preprocessing configuration, dimensions, and embeddings.
9. Resume from the manifest after interruption. Never recompute unchanged video
   and model/configuration combinations.
10. Import the manifest into the experimental exact-search store and produce a
    benchmark report before any production wiring.

### Storage strategy

Start with contiguous, normalized `f32` matrices, one file per modality, plus a
compact metadata table. Use exact cosine search for the bounded one-channel
experiment. This keeps results deterministic and provides ground truth for any
later approximate index.

Only test `int8` after measuring recall against `f32`. Only add HNSW or a vector
service when an actual corpus demonstrates that exact scan misses the latency or
concurrency budget. Do not place one vector per transcript moment on the current
8 GiB Typesense machine.

### Suggested first experiment

- Corpus: one approved channel, capped at 20–50 representative videos.
- Queries: at least 50 manually judged queries before model comparison:
  - 15 transcript paraphrases;
  - 10 exact names/code terms;
  - 10 visual-object/scene queries;
  - 10 motion/state-change queries;
  - 5 deliberately difficult or ambiguous queries.
- Baselines:
  - Typesense lexical only;
  - transcript semantic only;
  - intra-frame only;
  - inter-frame only;
  - lexical plus all semantic channels.
- Measurements: Recall@10, nDCG@10, MRR, zero-result rate, index bytes, indexing
  throughput, cold/warm query latency, peak RSS, and concurrent throughput.

### Initial promotion gates

- Visual queries improve materially over transcript-only retrieval.
- Exact names and code terms do not regress relative to lexical search.
- Every visual result exposes its evidence kind and timestamp.
- Model code and weight licenses are recorded by exact revision.
- Indexing is resumable and does not retain frames contrary to the approved
  retention policy.
- The measured machine fits the agreed RAM, disk, latency, and concurrency
  budgets.
- Shadow mode succeeds before a user-visible feature flag is enabled.

## Decisions to make after owner review

The answers above should produce these concrete outputs in the next revision:

1. supported platforms and minimum hardware;
2. approved corpus and retention policy;
3. exact model/checkpoint shortlist and licenses;
4. scene sampling and clip-duration configuration;
5. manifest and on-disk index schema;
6. benchmark query set and acceptance thresholds;
7. local CLI commands and dependency installation;
8. deployment shape, concurrency target, and cost ceiling.

