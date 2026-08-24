# Raw bounded proximity ranking

Measured 2026-08-08 on the 47,860-chunk, 787-video schema-v6 corpus.

## Compatibility target

Typesense 30.2 calculates proximity from token offsets in a window of at most
10 query tokens. Its match score ranks token coverage and typo cost ahead of
proximity, with early token offset as a separate lower-order component. Primary
sources are the [10-token window implementation](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/include/match_score.h#L9-L15)
and [packed score component order](https://github.com/typesense/typesense/blob/d45d46baf3996d1de8bf96a87f375cfb43691560/include/match_score.h#L49-L60).

Raw retains its exact-token mask and BM25/title score, then applies a bounded
same-field multiplier only to exact multi-token candidates. It considers at
most the first 10 retained query tokens. A heap-free sliding window finds the
minimum displacement containing one occurrence of every term. Windows wider
than 10 tokens receive no boost. The maximum multiplier is 1.25: up to 20%
for proximity plus 5% for an early best-window offset. Prefix, typo, split, and
cross-field-only candidates receive no positional bonus.

The hot posting path already knows the sorted candidate documents. The cold
reader seeks only those document IDs and their deduplicated video IDs, skips
intermediate deltas without allocating position lists, and reuses one vector
per query term. A rejected prototype intersected positional postings across
the whole corpus: pooled p95 regressed 23.1%, mean 5.4%, and CPU 6.2%. That
shape was discarded before commit.

## Correctness and relevance evidence

The focused fixture puts the first occurrences farther apart in one document
but a later pair adjacent; that document must outrank an equal-length document
whose best pair is gapped. Multipliers are tested for ordering, the 1.25 cap,
and the no-boost boundary beyond the 10-token window.

The frozen nine-query contract retained identical found counts and ordered top
results. A 30-query manual physics/computing review changed ordering on 11
queries. Inspected changes were directionally useful:

- `energy density` moved literal adjacent uses above `energy ... electron density`;
- `operating system` moved another literal phrase above a broad operating-room/system match;
- `quantum computer` moved a literal phrase above `Quantum Computing for computer scientists`;
- `general relativity` and `quantum mechanics` retained their top four.

This review is evidence for retaining the bounded candidate, not a substitute
for the pending 300-query human-judged oracle.

## Compact identifier follow-up

Measured 2026-08-23 on the 8,142,650-document, 32,822-video production-shaped
schema-v6 index. A compact identifier such as `createEffect` previously reached
`create` plus `effect` only through split fallback. That fallback carried term
frequency but no positional relationship, so a gapped Apple result could
outrank Ryan Carniato saying the two words adjacently.

When a compact query has one unambiguous exact two-token split, Raw now sends
those terms through the existing bounded exact-proximity path after unioning
the split results. It does not alter exact masks, generic parsing, fuzzy
alternatives, ambiguous multi-boundary splits, or literal compound recall.
This gives an adjacent occurrence a modest signal rather than turning it into
a mandatory phrase, so broad results remain available.

The full-index `createEffect` request retained all 433 compact-query videos and
moved Ryan Carniato's “Designing Concurrent Transitions” to first. The spaced
`create effect` query also ranks it first and returns 446 videos. Twelve warm
compact requests over loopback measured 17.6 ms median and 24.8 ms p95 after
the final work guard. The fixture also locks down a gapped title, reversed
terms, literal compound recall, both-token highlighting, lowercase compact
input, and `@ryansolid` scoping.

The first candidate reused the bounded proximity reader but still decoded and
intersected both component posting lists before that bound was visible. Common
accidental joins such as `thisthat` therefore took 465–518 ms. Raw now inspects
the mmap lexicon's posting counts first and declines a split pair whose combined
field-specific work exceeds 1,000,000 postings. It still tries bounded plural
alternatives and the original whole-token typo path, but does not repeat the
same rejected split through fuzzy recovery. A separate 50,000-candidate cap
protects the cold positional sidecar.

On the same 8.14-million-document index, the final 30-run matrix measured
`thisthat` at 12.9 ms p95 and `createEffect` at 27.7 ms p95. Adversarial
one-edit joins remained bounded: `thizthat` was 18.2 ms p95 and `youknpw` was
55.8 ms p95. `createEffect` still returned 433 videos with five Ryan Carniato
results at the top. This is a deliberate recall/latency boundary:
missing-separator recovery is optional fallback, so an unbounded common-word
split is less valuable than predictable service latency.

## Broad conjunction follow-up

The compact-only matrix initially hid the equivalent natural queries. On the
full corpus, `this that`, `you know`, and `there are` took roughly 0.3–0.7
seconds warm. Raw materialized and scored the first multi-million-document
posting list, allocated a contribution slot per candidate for the second, then
decoded complete positions for every broad survivor.

Broad two-plan searches now use document-at-a-time mmap cursors once both plans
exceed 100,000 estimated documents. Cursors merge exact and prefix alternatives,
preserve text/title cross-field scoring, validate the complete streams, and only
calculate BM25 for documents present in both plans. Broad sets above 50,000
candidates retain exact coverage and BM25 but skip optional full-position
refinement. Grouping uses a dense generation-local video table, and only the
requested result window is fully sorted. Canonical response hashes matched the
materialized reference for all four broad queries.

Thirty warm loopback runs per case produced `this that` at 165.3 ms median and
171.3 ms p95, `you know` at 126.3/132.1 ms, `there are` at 69.2/86.3 ms, and
`thank you` at 65.0/82.4 ms. The 30-query, 900-request matrix had zero errors;
every p95 was below 200 ms. Process RSS settled at 409.2 MiB after the matrix,
and the allocator retained less than 0.1 MiB during the measured 900 requests.

## Alternating release A/B

Two 50-run rounds per query were run in candidate/control then control/candidate
order. Each variant served the same schema-v6 mmap generation over loopback,
with 10 warmups per query and response caching disabled.

| Nine-query workload | Schema-v6 baseline | Candidate-targeted proximity | Change |
| --- | ---: | ---: | ---: |
| Searches | 900 | 900 | — |
| Median | 4.006 ms | 4.205 ms | +5.0% |
| p95 | 7.740 ms | 7.230 ms | -6.6% |
| Mean | 4.253 ms | 4.323 ms | +1.7% |
| CPU ticks | 339 | 345 | +1.8% |
| Internal mean | 3.186 ms | 3.250 ms | +2.0% |

These short local tails are noisy; the result supports only that the retained
path passes the 10% p95 guard while adding a small measured mean/CPU cost. Roll
back if the judged oracle finds an important regression, production-shaped p95
exceeds baseline by 10%, CPU cost grows beyond 10%, or a non-positional query
touches the cold sidecar.
