# Lazy-exact structural measurement harness

Status: throwaway schema-9 prototype. It does not define a production format,
open an index, mutate a generation, or provide production latency evidence.

Run the complete corpus-shaped lane with one command:

```bash
scripts/measure-lazy-exact-prototype.sh \
  /tmp/lazy-exact-prototype.jsonl \
  --documents 1000000 \
  --rounds 9
```

The output is one canonical JSONL summary row with every raw nanosecond sample.
An interrupted command retains `<output>.partial`; an existing result is never
overwritten. Non-temporary output is rejected unless the caller deliberately
sets `ALLOW_NON_TEMP_PROTOTYPE_OUTPUT=1`.

## Decisions this run can inform

| Decision | Exact output and timing fields |
| --- | --- |
| Bitmap versus sparse skip intersection counting | `exact_set_count[].exact_intersection`, `bitmap_and_popcount`, `sparse_skip_leapfrog` |
| Dense set-bit versus sparse posting moment scan | `exact_moment_aggregation[].matching_videos`, `exact_moments`, `dense_set_bit_scan`, `sparse_posting_scan` |
| Bitmap versus skip phrase candidate intersection | `phrase_candidates[].exact_candidates`, `exact_phrase_matches`, and both timing fields |
| Representation construction and owned bytes | `bitmap_construction`, `sparse_construction`, `bitmap_bytes`, `sparse_bytes` |

Every timed answer is checked outside the timing region against the competing
implementation and a deliberately simple exhaustive oracle. Empty/no-candidate
negative controls remain in the output but have `timing_eligible: false`, so
zero work cannot win a comparison. Query scratch is reported explicitly; these
forks use no query-owned allocation.

The deterministic generator uses dense document IDs, distinct `DocId` and
`VideoId` types, about 240 transcript documents per video, ordered skewed
timestamps, and fixed SplitMix64 seeds. Density bands span absent through
universal. The moment oracle copies Raw's anchored, non-transitive rule: a match
stays in its current moment at exactly 20 seconds and starts a new one only
after 20 seconds.

## First local observation

Environment: release profile, 1,000,000 synthetic documents, 4,167 videos,
nine in-process samples after one warmup. These are
`steady_state_in_process_os_cache_warm` observations, not cold-open or
independent-process measurements.

| Target DF | Bitmap count median | Skip count median | Dense moment median | Sparse moment median |
| ---: | ---: | ---: | ---: | ---: |
| 0.1% | 21.4 us | 6.1 us | 20.2 us | 2.5 us |
| 1% | 21.4 us | 171.9 us | 216.7 us | 62.8 us |
| 10% | 21.4 us | 2.21 ms | 1.05 ms | 704.9 us |
| 50% | 13.2 us | 7.17 ms | 2.67 ms | 2.28 ms |
| 90% | 12.8 us | 4.60 ms | 4.68 ms | 4.12 ms |

This run supports a density-aware counting representation: sparse skip wins at
the rarest measured band while bitmap popcount wins by roughly 1% DF. It does
not show a dense-bit iteration win for moment aggregation; both paths still
perform the required timestamp gathers, and sparse IDs were modestly faster at
every measured non-empty band. Bitmap phrase candidates won from 1% upward;
the 0.1% case had no candidates and is explicitly ineligible.

## Limits that remain open

- The synthetic distribution is corpus-shaped, not a production snapshot.
- Construction measures owned representation allocation from the same logical
  posting list, but does not model serialization or mmap page faults.
- Timing is warm and in one process. No result is labeled cold; fresh-process
  physical I/O, peak RSS, independent repetitions, and stability gates remain
  unmeasured.
- The harness does not yet cover overlap matrices, OR/NOT trees, title-only
  moments, Tier-1 histograms, codecs, ranking frontiers, or sessions.
- The phrase fork uses one stored position per term. Multiple/repeated/cross-
  field position cases remain a later prototype before a format decision.

Those limits prevent selecting a schema or claiming the 2 GB / 200 ms release
gate from this evidence alone.
