# Typesense Semantic Experiment (Retired) - 2026-07-13

## Decision

The experiment was rejected and its public UI and Fly resources were removed.
Production remained on Keyword search and no production collection was
backfilled or switched. The provider-neutral code can remain dormant for future
offline research, but semantic search must not be advertised or enabled in a
deployment.

The narrow security fixture made the integration look better than it generalized.
On the expanded experimental corpus, manual creator-filtered checks returned:

| Query                                  | First three semantic video titles                                                                                                             | Assessment                                                                                 |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `going all the way back to the server` | `Going Back To Next`; `How Minecraft AI ACTUALLY works`; `Chrome Exposes Scary System Functions To All Google Domains`                        | Only the first result is plausibly related; generic words dominated the rest.              |
| `react server components`              | `React Server Components: A Comprehensive Breakdown`; `Reflecting on React Server Components`; `I Was Wrong About React Server Components...` | Exact terminology worked, but this did not demonstrate semantic value over Keyword search. |
| `battery gate`                         | `Apple isn't built for the AI era`; `AI has rewired my brain`; `I'm so tired.`                                                                | Failed to retrieve a reliably specific BatteryGate result.                                 |
| `c programming arrays`                 | `Claude made its own language (it took 3 months)`; `What's the best programming language for AI?`; `How Programming Will Change In 2024`      | Unrelated broad programming results.                                                       |

The preview could also retain the previous query's result surface while the next
request was in flight. That made BatteryGate text appear during a subsequent
React Server Components search and was misleading even when the eventual
response was correct. Server-side lexical fallback improved exact queries, but
it obscured whether vector retrieval added value and did not repair this UI
isolation failure.

Reasons not to ship this version:

- The corpus was incomplete and biased, so embedding recall varied by topic.
- MiniLM similarity over transcript chunks over-weighted generic conversational
  language and broad programming concepts.
- There was no versioned judged corpus or MRR/Recall@10 gate for representative
  queries.
- Vector storage and a separate API/database added cost and failure modes without
  consistently improving production relevance.
- Previous-query results were visible during a new semantic request.
- Lexical fusion could mask semantic failures instead of measuring them.

A future experiment must run in shadow/offline mode first, use a representative
judged query set, preserve exact-match quality, clear stale UI state immediately,
and pass relevance, latency, storage, and cost budgets before a user-facing
selector is deployed.

## Retired inventory

| Resource                                  | Configuration                          | Purpose                                            |
| ----------------------------------------- | -------------------------------------- | -------------------------------------------------- |
| `ytsearch-typesense-semantic-exp`         | Typesense 30.2, shared 2 CPU, 4 GB RAM | Isolated vector collection                         |
| `typesense_data`                          | Encrypted 3 GB Fly volume              | Experiment model/index data                        |
| `ytsearch-api-semantic-exp`               | shared 1 CPU, 512 MB RAM               | Semantic-enabled Rust API                          |
| `https://c5b7f453.ytsearch-755.pages.dev` | Cloudflare Pages preview               | Replaced with a Keyword-only build during teardown |

No experiment resource has production queue credentials or a production data
volume. The experiment Typesense key is separate and is not stored in the repo.

## Corpus and schema

The fixed fixture contains 250 real transcript chunks selected by the lexical
query `linux exploit`. The experiment collection clones the active document
schema and adds:

```json
{
  "name": "embedding",
  "type": "float[]",
  "embed": {
    "from": ["title", "text"],
    "model_config": { "model_name": "ts/all-MiniLM-L12-v2" }
  }
}
```

Typesense downloaded the built-in model and imported/embedded all 250 documents
in four seconds with zero document failures. After indexing, Typesense reported
225 MB resident memory and 308 MB used disk for the Machine as a whole.

## Search results

For `breaking into a computer through a security bug`, lexical search returned
2 videos. Hybrid semantic search returned 18 videos and promoted plausible
security/exploit results. For the exact query `linux exploit`, both modes kept
the strongest exact-title results at the top.

Twenty browser-API-shaped requests for the paraphrase produced:

| Mode     | Median |    p95 |               Maximum |
| -------- | -----: | -----: | --------------------: |
| Lexical  | 103 ms | 165 ms | 1,848 ms cold outlier |
| Semantic | 155 ms | 185 ms |                216 ms |

The semantic latency premium is currently about 20 ms at p95 on the isolated
machine. The corpus is intentionally narrow, so these numbers validate the
integration and failure behavior, not full production relevance.

## Browser verification

Playwright verified against the immutable Pages preview that:

- Keyword is selected by default.
- Semantic must be selected explicitly.
- The request URL contains `mode=semantic`.
- The experimental API returns HTTP 200.
- Result cards render and URL state retains semantic mode.

Run it again with:

```bash
PLAYWRIGHT_BASE_URL=https://c5b7f453.ytsearch-755.pages.dev bun run test:e2e
```

## Cost and promotion

The two always-running experiment Machines are approximately a few cents per
hour at Fly's published shared-CPU rates, plus the 3 GB volume. They should be
stopped when experiments pause. Do not delete them until the fixed fixture,
schema, metrics, and cleanup process have been captured.

Production promotion still requires a broader judged corpus with exact-match,
creator-filtered, paraphrase, vague-concept, low-signal, and near-duplicate
cases. It must pass MRR/Recall@10, p95, memory, import failure, browser, and
lexical fallback gates before any active collection alias changes.

Cleanup performed after recording the results:

```bash
flyctl apps destroy ytsearch-api-semantic-exp --yes
flyctl apps destroy ytsearch-typesense-semantic-exp --yes
```
