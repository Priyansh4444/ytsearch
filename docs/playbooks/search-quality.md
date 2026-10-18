# Search quality

## Contract

Best-match ranking, not mandatory all-term matching. Low-signal queries use
newest-for-filter ordering. Stopwords must not dominate snippets. Results group
multiple timestamp hits into video cards; `SearchSort` lives in `youtube_api`.

## Verification

After relevance changes, exercise low-signal, stopword and creator-filtered queries:

```bash
curl -fsS 'https://ytsearch-api-raw-production.fly.dev/api/search?q=fable%20on%20high&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at,title_highlight,hit_count:(.hits|length), first:.hits[0].text_highlight})'
curl -fsS 'https://ytsearch-api-raw-production.fly.dev/api/search?q=I&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at})'
```

`I` should return Theo's most recent indexed uploads first. `fable on high`
should put `A proper guide to Fable 5` near the top with relevant snippets.
Use `scripts/raw-schema-differential.sh` before shipping an index-format change;
use the [API differential](api-changes.md) for response-preserving API changes.

For ranking changes beyond these spot checks, the
[use-case evaluation](../experiments/semantic/ranking-eval-2026-10-17.md) scores
152 judged queries (quotes, paraphrases, questions, typos, vague and topic
queries) with `scripts/semantic-eval.py`. Lexical currently scores 0.97 MRR@10 on
exact quotes but 0.04 on questions; keep the quote score when changing lexical.
