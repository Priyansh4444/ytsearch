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
| Representation construction and owned bytes | `construction_source_bytes`, `bitmap_construction`, `sparse_construction`, `bitmap_bytes`, `sparse_bytes` |

Every timed answer is checked outside the timing region against the competing
implementation and a deliberately simple exhaustive oracle. Every attempted
case remains in its section's array, including failures and negative controls.
`outcome` is a tagged `eligible`, `ineligible { reason }`, or
`failed { reason }` value; an oracle mismatch becomes a failed
`oracle_mismatch` and its timing fields are `null`. Measurement failures have a
separate `measurement_failure` reason. Thus an
incorrect or zero-work fork remains in the denominator and cannot win by
disappearing. Query scratch is reported explicitly; these forks use no
query-owned allocation.

The top-level `schema_version` is 3. `run_identity` binds the effective document
count, round count, repetition, ordered DF bands, fixed seed, generator version,
harness version, actual source HEAD and dirty flag, Rust compiler and target,
build profile, injected-failure mode, and host environment into a BLAKE3 digest. The wrapper supplies
these values from the process that performs the run rather than embedding an
old commit in the binary. The fixture
test proves identical effective inputs have identical semantic output and that
changing an effective input changes the identity. This is a report schema, not
a Raw runtime or on-disk index schema.

`counters` states attempted, eligible, excluded, and failed denominators. The
explicit prototype-only `--inject-failure oracle|measurement` control proves
end to end that a bad case remains serialized, all of its timing fields are
null, the failed denominator increments, and the top-level outcome becomes
`complete_with_failures`. This retention covers failures observed inside the
running harness. A subprocess crash cannot be converted into a typed row; the
wrapper honestly leaves `<output>.partial`, and callers must treat it as an
incomplete run rather than a report.

Each timing includes wall-clock raw samples, median, p95, min/max, and process
CPU samples when the OS clock is available (otherwise a reason). Competing A/B
fork order alternates between samples within each DF case; order fields record
the first sample's order. Construction/query starting order is inverted
within a set-count case. Both construction forks consume their resulting
collections through the same optimization barrier, without an extra checksum
traversal. The report captures baseline/final process peak RSS and
`/proc/self/io` deltas, or an explicit unavailable reason on unsupported hosts.

The deterministic generator uses dense document IDs and distinct `DocId`,
`VideoId`, `DocSet`, and `VideoSet` types, about 240 transcript documents per video, ordered skewed
timestamps, and fixed SplitMix64 seeds. Density bands span absent through
universal. The moment oracle copies Raw's anchored, non-transitive rule: a match
stays in its current moment at exactly 20 seconds and starts a new one only
after 20 seconds.

## Historical local observation — superseded

These measurements predate the review fixes for asymmetric construction work
and per-case timing order. They are retained as trial history only. They do not
support a crossover threshold or a representation choice; collect a fresh run
with the corrected harness before making either decision.

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

The recorded values suggested density-sensitive counting behavior, but the
timing-order confound prevents using that suggestion as a decision. The 0.1%
phrase case had no candidates and was explicitly ineligible.

## Corrected local observation (2026-09-13)

[Raw report with samples and provenance](evidence/lazy-exact-92a2760-r1.jsonl).
Source `92a2760aad88a01fc533e4298475340db6310b59`, clean tree, harness v4,
release profile, one million synthetic documents, ten alternating paired
samples per case after warmup. This is an uncontrolled local Linux host, not
the Fly 2-GB profile. The outcome contains 24 attempted cases: 20 eligible,
four explicit exclusions, zero failures. Absence/zero-candidate controls remain
in the report rather than disappearing from the denominator.

| Target DF | Bitmap count median | Skip count median | Dense moment median | Sparse moment median |
| ---: | ---: | ---: | ---: | ---: |
| 0.1% | 21.44 us | 5.74 us | 15.95 us | 2.78 us |
| 1% | 22.17 us | 158.72 us | 200.79 us | 93.90 us |
| 10% | 23.40 us | 2.22 ms | 1.00 ms | 650.18 us |
| 50% | 28.66 us | 7.08 ms | 3.19 ms | 2.64 ms |
| 90% | 28.66 us | 4.40 ms | 4.58 ms | 3.93 ms |
| 100% | 13.35 us | 557.53 us | 4.32 ms | 3.59 ms |

This corrected observation supports retaining both representations for the
next corpus experiment. It does not establish a production DF threshold or
bitmap byte budget. Sparse moment enumeration remained faster in these fixtures;
bitmap cardinality alone does not replace the timestamp scan for exact moments.

The process reported peak RSS 57,364,480 bytes at both resource snapshots and
zero physical read-byte delta. The zero peak increment does **not** mean zero
allocation: the baseline is a process high-water mark after fixture setup.
No index was opened, so this says nothing about mmap cold-open I/O. No
end-to-end latency, whole-set summary, or Typesense speedup is claimed.

## Limits that remain open

- The synthetic distribution is corpus-shaped, not a production snapshot.
- Construction measures owned representation allocation from the same logical
  posting list, but does not model serialization or mmap page faults.
- Timing is warm and in one process. No result is labeled cold; fresh-process
  physical I/O, peak RSS, independent repetitions, and stability gates remain
  unmeasured.
- The harness does not yet cover overlap matrices, OR/NOT trees, Tier-1
  histograms, codecs, ranking frontiers, or sessions.
- The DF-band phrase corpus still stores one position per term. Separate timed
  exact semantic rows exercise repeated terms and multiple occurrences;
  adversarial oracle fixtures also cover reverse and non-adjacent positions and
  prohibit cross-field phrases. Title-only and
  mixed title/transcript moment fixtures implement the response contract:
  title-only contributes one moment, while a mixed match adds no phantom one.

Those limits prevent selecting a schema or claiming the 2 GB / 200 ms release
gate from this evidence alone.
