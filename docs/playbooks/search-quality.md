# Search quality

## Contract

Best-match ranking, not mandatory all-term matching. Low-signal queries use
newest-for-filter ordering. Stopwords must not dominate snippets. Results group
multiple timestamp hits into video cards; `SearchSort` lives in `youtube_api`.

## Dropping words when a search finds too little

Raw matches a query as a conjunction of its words. When that finds fewer than
five chunks it keeps a subset of the words (`terms_retention_order` in
`crates/youtube_raw_search/src/engine.rs`):

- A query with a common word (in at least 1% of transcript chunks and at least
  1,000 of them, such as "why", "does", "think") keeps its **rarest** words and
  drops the most common first. Questions put the topic last ("why he dropped
  prisma"), and dropping from the right kept the filler.
- A word that is in no transcript is dropped before any other. Keeping it made
  every smaller query fail, which cost typos half their hits.
- A query of only uncommon words keeps its **leading** words as before. Its
  words rarely share a chunk, the rarest least, so rarest-first stopped at one
  incidental word; on a held-out set it lost 5 of 20 keyword searches.
- Common means transcript frequency. Counting title matches called "game"
  filler and lost another keyword search.

This only runs when the full conjunction is too small. "who is dan abramov"
still finds 7 videos against 69 for "dan abramov", because seven chunks say all
four words; `mode=hybrid` rewrites the question for that case. Applying the
rewrite to every lexical search cost exact quotes that begin with "did" or "is"
(8-word quotes 99.0% to 97.0%), so it stays in hybrid.

Measured on a local index of all 8.1M archived chunks (69 channels), replaying
201 random moments (3 per creator) with generated queries; first 12 results,
right video first, 95% bootstrap intervals in the PR:

| Query | Before | After |
| --- | --: | --: |
| 8 or 4 exact words, three rare words | unchanged | unchanged (identical top 12 on all 603) |
| One letter wrong | 43.3% | 81.6% |
| Question | 0.5% | 11.1% |
| Question and creator filter | 6.4% | 27.7% |
| "The part where…" | 0.5% | 13.8% |

On 40 held-out moments with queries written by another model, keyword searches
held at 13 of 20 and questions went from 0 of 20 to 8 of 20. Gemma graded the top
three results of changed queries: the top result was on topic 3% to 9% of the
time before and 31% to 39% after for questions, and 41% to 76% for typos. Latency
was unchanged (median 70 ms, p99 under 650 ms, sequential).

To repeat it, build an index from the archive with `youtube-raw-search build`,
serve a baseline and a patched build, and replay one query set against both.

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
