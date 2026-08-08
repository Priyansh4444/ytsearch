# Raw exact quoted-phrase search

Measured 2026-08-08 against the schema-v6 Veritasium/Vsauce index: 47,860
transcript chunks, 787 videos, and 12,261,425 index bytes.

## Contract

A balanced quoted segment is an ordered phrase with zero slop by default. The
Raw-specific `phrase_slop` query parameter accepts 0 through 8 total
intervening tokens. The parser preserves
duplicate tokens inside phrases, so `"black black"` requires two adjacent
occurrences. Multiple quoted segments and unquoted terms are conjunctive, and
each phrase must occur wholly within one configured field. Quoted queries do
not use prefix, typo, split, or token-drop fallback. Empty or unterminated
quotes and out-of-range slop fail closed, phrases are capped at 32 tokens, and schema-v5 rollback
generations reject quoted search because they do not contain complete
positions.

Negative terms and phrase-aware multi-snippet highlighting remain open.
General unquoted proximity ranking is measured separately in
[proximity-ranking.md](proximity-ranking.md).

## Reader design

The hot posting intersection first produces sorted candidate document and
video IDs. Each phrase term then gets a small cursor over its posting list,
hot first-position range, and cold remaining-position range. The cursors seek
only those candidate IDs. When a common term is below the next candidate, its
cold deltas are validated and skipped without materializing positions. Only
aligned candidates decode into one reused vector per phrase token. A greedy
ordered walk selects the earliest next occurrence, which minimizes consumed
slop for each possible first occurrence.

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
| `"speed light"`, slop 1 | 28 | `ACUuFg9Y9dY` |

All returned hits in the inspected result windows contained the phrase in the
transcript or title. Both quoted and unquoted `black hole` happen to return 16
strict conjunctions on this snapshot; the adversarial fixture proves that a
gapped occurrence is excluded at slop 0 and included at slop 1. The compact
single-token `blackhole` recovery remains a separate 21-video path.

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
shows an important relevance loss. Phrase-aware highlighting and the full
human-judged oracle remain required before this P0 area is complete.
