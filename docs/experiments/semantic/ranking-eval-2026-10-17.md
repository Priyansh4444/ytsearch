# Semantic vs lexical ranking, by use case (2026-10-17)

**Result: neither engine wins alone. Semantic is far better at natural-language
search; lexical is far better at exact words. A gated hybrid that trusts lexical
only when it matched most of the query scores best overall (0.65 macro average,
against 0.47 lexical and 0.52 semantic).**

## Setup

- **Corpus:** Theo's (@t3dotgg) 300 newest indexed videos, 11,324 passages,
  served by the [experimental site](../../playbooks/semantic-pilot.md#experimental-fly-site)
  (EmbeddingGemma 300M Q4, 128 dimensions, Qdrant).
- **Lexical:** the production Raw API with the same creator filter. Its results
  are restricted to the same 300 videos, so both engines rank the same set.
- **Hybrids**, fused offline from the two rankings with weighted reciprocal rank
  fusion (k=60): `rrf` 1:1, `rrf_sem2` 2:1 toward semantic, and `gated`, which
  weights 3:1 toward lexical when lexical's top video matched at least 75% of
  the query's content words (from its highlights, so fuzzy matches count) and
  3:1 toward semantic otherwise. Weights and threshold were fixed before scoring.
- **Queries:** [152](queries-2026-10-17.tsv) in six use cases. 36 target moments
  (~45 s each) were sampled at random from the middle 70% of 36 random videos;
  each got a quote, a paraphrase and a question query. 12 typo and 16 "the video
  where…" queries reuse those targets. 16 broad topic queries have many correct
  answers.
- **Scoring:** known-item queries use MRR@10 and recall for the target video, and
  moment@1 (top video is the target and one of its first three hits is within
  ±30 s of the target window). Topic queries use nDCG@5 over a pool of every
  engine's top 5 (121 videos), graded 0/1/2 [blind to engine](grades-2026-10-17.tsv)
  from titles and snippets.

## Results

MRR@10 for known-item use cases, nDCG@5 for topics ([raw scores](scores-2026-10-17.json)):

| Use case | n | Lexical | Semantic | RRF 1:1 | RRF sem 2:1 | Gated |
| --- | --: | --: | --: | --: | --: | --: |
| Exact quote | 36 | **0.97** | 0.28 | 0.95 | 0.57 | 0.94 |
| Paraphrase | 36 | 0.10 | 0.31 | 0.34 | 0.41 | **0.42** |
| Question | 36 | 0.04 | **0.59** | 0.29 | 0.39 | 0.42 |
| Typo | 12 | 0.57 | 0.25 | **0.58** | 0.50 | 0.50 |
| Vague "the video where…" | 16 | 0.63 | 0.80 | 0.73 | 0.80 | **0.82** |
| Topic (nDCG@5) | 16 | 0.50 | **0.88** | 0.78 | 0.82 | 0.80 |
| **Macro average** | | 0.47 | 0.52 | 0.61 | 0.58 | **0.65** |

Recall@10 tells the same story: semantic finds the target for 75% of questions
against 6% for lexical, and lexical finds 97% of quotes against 47% for semantic.

Moment@1 (right video and right moment first):

| Use case | Lexical | Semantic | Gated |
| --- | --: | --: | --: |
| Exact quote | 0.97 | 0.19 | 0.86 |
| Paraphrase | 0.06 | 0.11 | 0.17 |
| Question | 0.03 | 0.42 | 0.19 |
| Typo | 0.42 | 0.17 | 0.42 |

Latency: semantic p50 64 ms / p95 83 ms server-side, lexical 104 / 391 ms.
These are not comparable: semantic searched 11k passages, lexical searched
9.4M chunks.

The gate behaved as intended: it chose lexical-led fusion for 31/36 quotes,
0/36 paraphrases, 1/36 questions, 1/12 typos, 2/16 vague and 5/16 topic queries.
None of the 152 semantic requests fell back to lexical.

## Why each engine fails

- **Lexical on natural language.** Questions and paraphrases share only common
  words with the target, so best-match ranking surfaces whatever repeats them.
  It scores 0.04 on questions.
- **Semantic on verbatim quotes.** A short exact phrase embeds as its topic:
  "they made their own language" returns *Claude made its own language*, not
  the Meta/PHP moment in *Life after TypeScript*.
- **Semantic on a detail inside a long video.** Passages are up to 60 s and
  each video has dozens. Queries about a passing remark lose to videos whose
  main subject matches: "Theo almost joined that hosting startup" returns *Theo
  fell off*, not the Railway aside. Most paraphrase misses are this case.
- **Semantic picks the wrong moment.** When it finds the right video, its best
  passage is often the intro (0:00), which summarizes the whole topic. Four
  question queries ranked the right video first with the wrong moment.
- **Typos** hurt semantic more than lexical, whose fuzzy matching recovers
  "utilising your unused bandwith". Fused rankings recover most of the gap.
- **Equal-weight fusion hurts questions.** RRF 1:1 halves semantic's question
  score because lexical's confident-looking noise outranks the answer. The gate
  helps (0.42) but does not reach semantic alone (0.59).

## Recommendation

1. Keep lexical as the default and the only engine for exact phrases. Do not
   replace it with semantic.
2. If semantic ships, ship the gated hybrid, not semantic alone and not plain
   RRF. Fusion belongs in `youtube_api`, next to `SearchSort`.
3. Next experiments, each needing a **new** query set so they are not tuned on
   this one:
   - route question-shaped queries (wh-word or `?`) to semantic-led fusion;
   - smaller passages (~30 s) or several vectors per passage, for details;
   - down-weight intro passages, or choose the moment within a video by
     lexical overlap, for moment accuracy;
   - a cross-encoder reranker over the fused top 20.
4. Re-run on a larger corpus before trusting absolute numbers. 300 videos
   flatters both engines; more distractors will lower scores.

## Caveats

One person (the agent) wrote the queries after reading the targets, and graded
the topic pool. Paraphrases avoid the target's distinctive words but are still
informed by them. The query set is small (12–36 per use case), so differences
under about 0.1 are within noise. Raw rankings contain transcript text and stay
out of git.

## Reproduce

```bash
python3 scripts/semantic-eval.py fetch /tmp/semantic-eval.json   # ~6 min, rate-limited
python3 scripts/semantic-eval.py pool  /tmp/semantic-eval.json   # topic pool for grading
python3 scripts/semantic-eval.py score /tmp/semantic-eval.json
```

The experimental site stops when idle; the first request wakes it.
