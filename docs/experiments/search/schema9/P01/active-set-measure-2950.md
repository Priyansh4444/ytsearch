# P01 exact set counting on the active corpus

Status: local representation experiment on copied, checksum-verified schema-6
generation 2950 files. It is not the 2 GiB Fly reader or a schema-9 release
benchmark. The [JSON receipt](active-set-measure-2950.json) contains all 31
paired samples, construction times, source identity, and input digests. The
harness is [`active_set_measure.rs`](../../../../../crates/youtube_raw_search/examples/active_set_measure.rs)
at commit `58c1ea44b3aab430ea399dbe3648adc371149e79`.

The harness checked the active manifest's BLAKE3 for both source files before
decoding. It compared each count with the independent
[posting overlap receipt](active-posting-overlap-2950.md). Each sparse median is
the faster of a linear merge and ordered binary seeks on decoded ID arrays.
The bitmap median scans two already constructed dense bitmaps and popcounts
their AND. Times below are milliseconds, rounded from nanoseconds.

| Transcript pair | Exact intersection | Best sparse median | Dense bitmap median |
| --- | ---: | ---: | ---: |
| `the` + `and` | 1,944,010 | 55.702 | 0.187 |
| `in` + `the` | 1,077,539 | 34.866 | 0.194 |
| `this` + `is` | 747,843 | 28.870 | 0.189 |
| `is` + `the` | 1,298,392 | 41.803 | 0.192 |
| `coding` + `dead` | 20 | 0.061 | 0.118 |
| `css` + `dead` | 11 | 0.030 | 0.283 |
| `create` + `effect` | 535 | 0.273 | 0.109 |
| `black` + `hole` | 1,084 | 0.083 | 0.117 |
| `one` + `fire` | 824 | 0.698 | 0.110 |

The selected sparse method was linear merge for every pair. An existing bitmap
is much faster for broad count kernels; sparse arrays win for some rare pairs.
The two methods use different construction and residency costs: each full text
bitmap occupies 1,052,808 bytes. The 14 distinct terms here retained
14,739,312 bytes of bitmap vector capacity and 57,070,332 bytes of decoded
sparse vector capacity. Bitmap construction took about 0.6–9.4 ms per term in
this run. The process high-water RSS was 412,708,864 bytes, including mapped
input files, retained vectors, and runtime allocations. This does not establish
a production bitmap cache budget.

The measurements are warm local single-process **count kernels**. They exclude
posting I/O during queries, bitmap construction during queries, the schema-9
codec and skips, phrase/negative/filter evaluation, video-level field mixing,
ranking, exact Tier-0 moment summaries, concurrent sessions, publication, and
Fly CPU/memory behavior. P01's bitmap threshold and byte budget remain open.
