# Raw exact quoted-phrase search

Measured 2026-08-08 against the schema-v6 Veritasium/Vsauce index: 47,860
transcript chunks, 787 videos, and 12,261,425 index bytes.

## Contract

A balanced quoted segment is an exact ordered phrase. The parser preserves
duplicate tokens inside phrases, so `"black black"` requires two adjacent
occurrences. Multiple quoted segments and unquoted terms are conjunctive, and
each phrase must occur wholly within one configured field. Quoted queries do
not use prefix, typo, split, or token-drop fallback. Empty or unterminated
quotes fail closed, phrases are capped at 32 tokens, and schema-v5 rollback
generations reject quoted search because they do not contain complete
positions.

The first implementation deliberately leaves negative terms, configurable
slop, general unquoted proximity ranking, and phrase-aware multi-snippet
highlighting open. Ordinary unquoted query behavior is unchanged.

## Reader design

Each phrase term gets a small cursor over its posting list, hot first-position
range, and cold remaining-position range. The cursors advance as a streaming
multiway intersection. When a common term is below the current candidate ID,
its cold deltas are validated and skipped without materializing positions.
Only aligned candidates decode into one reused vector per phrase token. Exact
adjacency is then checked with binary search over the sorted position vectors.

This keeps work proportional to the touched posting ranges and bounds
request-owned position vectors by query length and term frequency. It does not
hydrate document blocks to decide phrase membership.

## Correctness evidence

Fixtures cover adjacent, gapped, reversed, repeated, title-only, and mixed
quoted/unquoted cases. The writer/reader fixture reconstructs repeated
positions from the real sidecars. On the real corpus:

| Query | Matching videos | Top video |
| --- | ---: | --- |
| `"black hole"` | 16 | `3pAnRKD4raY` |
| `"event horizon"` | 4 | `6akmv1bsz1M` |
| `"speed of light"` | 28 | `ACUuFg9Y9dY` |
| `"quantum mechanics"` | 12 | `qJZ1Ez28C-A` |

All returned hits in the inspected result windows contained the phrase in the
transcript or title. `"black hole"` narrows the existing unquoted `black hole`
result from 21 to 16 videos, as expected for ordered adjacency.

## Performance evidence

The release server ran on the Intel Core Ultra 7 255H host described in the
[schema-v6 report](schema-v6-positions.md). After ten warmups per query, the
benchmark issued 100 sequential loopback HTTP searches per query with the
response cache disabled.

| Query | Median | p95 |
| --- | ---: | ---: |
| `"black hole"` | 3.646 ms | 4.850 ms |
| `"event horizon"` | 1.886 ms | 2.520 ms |
| `"speed of light"` | 6.233 ms | 8.237 ms |
| `"quantum mechanics"` | 5.148 ms | 5.730 ms |
| Pooled 400 searches | 4.188 ms | 7.032 ms |

Pooled mean was 4.173 ms externally and 3.007 ms inside the engine. The run
used 147 CPU ticks for 400 searches. Observed RSS rose from 21.09 MB to 26.02
MB as cold index pages were faulted; this is a page-residency observation, not
a stable per-request allocation estimate.

Rollback if exact-phrase fixtures regress, malformed syntax stops failing
closed, ordinary-query response parity changes, or the future judged oracle
shows an important relevance loss. Slop and unquoted proximity require their
own measured ranking gate before this P0 area is complete.
