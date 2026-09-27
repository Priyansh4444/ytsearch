# P01 exact set counting on the active corpus

Status: local representation experiment on copied, checksum-verified schema-6
generation 2950 files. It is not the 2 GiB Fly reader or a schema-9 release
benchmark. The [JSON receipt](active-set-measure-2950.json) contains all 31
paired samples, construction times, source identity, and input digests. The
harness is [`active_set_measure.rs`](../../../../../crates/youtube_raw_search/examples/active_set_measure.rs)
at commit `db4fded25d897b88d9b7122c51b5846dc767338b`.

The harness checked the active manifest's BLAKE3 for both source files before
decoding. It compared each count with the independent
[posting overlap receipt](active-posting-overlap-2950.md). Each sparse median is
the faster of a linear merge and ordered binary seeks on decoded ID arrays.
The bitmap median scans two already constructed dense bitmaps and popcounts
their AND. Times below are milliseconds, rounded from nanoseconds.

| Transcript pair | Exact intersection | Best sparse median | Dense bitmap median |
| --- | ---: | ---: | ---: |
| `the` + `and` | 1,944,010 | 55.813 | 0.199 |
| `in` + `the` | 1,077,539 | 34.998 | 0.194 |
| `this` + `is` | 747,843 | 29.090 | 0.208 |
| `is` + `the` | 1,298,392 | 42.158 | 0.208 |
| `coding` + `dead` | 20 | 0.061 | 0.120 |
| `css` + `dead` | 11 | 0.029 | 0.283 |
| `create` + `effect` | 535 | 0.277 | 0.113 |
| `black` + `hole` | 1,084 | 0.068 | 0.112 |
| `one` + `fire` | 824 | 0.705 | 0.114 |

The selected sparse method was linear merge for every pair; the harness
counterbalanced sparse call order across rounds. An existing bitmap
is much faster for broad count kernels; sparse arrays win for some rare pairs.
The two methods use different construction and residency costs: each full text
bitmap occupies 1,052,808 bytes. The 14 distinct terms here retained
14,739,312 bytes of bitmap vector capacity and 57,070,332 bytes of decoded
sparse vector capacity. Bitmap construction took about 0.4–9.3 ms per term in
this run. The process high-water RSS was 412,708,864 bytes, including mapped
input files, retained vectors, and runtime allocations. This does not establish
a production bitmap cache budget.

The measurements are warm local single-process **count kernels**. They exclude
posting I/O during queries, bitmap construction during queries, the schema-9
codec and skips, phrase/negative/filter evaluation, video-level field mixing,
ranking, exact Tier-0 moment summaries, concurrent sessions, publication, and
Fly CPU/memory behavior. P01's bitmap threshold and byte budget remain open.
