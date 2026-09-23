# P01 structural-set measurement checkpoint

This is a corrected **synthetic** baseline at clean source commit
`5b3e646c1f592c4f912a63733c9b9b8a90fa0077`. The release-mode harness
used one million generated documents and nine alternating paired samples per
case. It reported 24 attempted cases, 20 eligible, four explicit exclusions,
and zero failures. The [raw report](prototype-5b3e646-r1.jsonl) includes the
input identity, all samples, denominators, and resource counters.

| Target term DF | Bitmap AND count median | Sparse skip count median | Bitmap bytes | Sparse bytes |
| ---: | ---: | ---: | ---: | ---: |
| 0.1% | 21.40 µs | 5.99 µs | 250,000 | 8,156 |
| 1% | 21.45 µs | 154.41 µs | 250,000 | 80,308 |
| 10% | 21.71 µs | 2,238.04 µs | 250,000 | 801,088 |
| 50% | 22.95 µs | 5,742.35 µs | 250,000 | 4,001,512 |
| 90% | 17.22 µs | 3,528.33 µs | 250,000 | 7,201,208 |
| 100% | 10.69 µs | 460.43 µs | 250,000 | 8,000,000 |

The measurements support retaining both sparse and dense exact-set paths. They
do not choose a production DF threshold or bitmap byte budget: the generated
corpus, local host, and warm in-process timing differ from the 2 GiB Fly
reader. P01 still needs real-corpus overlap and timing, phrase, codec,
rank-frontier, session, and independent performance evidence. The P02 set implementation on
the child branch is a unit foundation, not a P01 or P02 completion verdict.

The separate [active lexicon survey](active-lexicon-df-2950.md) records actual
schema-6 term frequencies and payload-only bitmap capacities. It does not
convert these synthetic timing results into production performance evidence.
