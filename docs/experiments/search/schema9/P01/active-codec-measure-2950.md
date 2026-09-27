# P01 active posting ID codec comparison

The [machine-readable report](active-codec-measure-2950.json) was produced by committed harness source `e22961d039955c78688a4f7d580e433b65f51363` against the active schema-6 generation 2950 lexicon and posting files. The harness verified the files' BLAKE3 digests against the active manifest, checked source DFs and encoded lengths against the independent [posting overlap receipt](active-posting-overlap-2950.json), checked every candidate's full decoded ID list and 257 point seeks, then checked all nine independent pair intersections. It measured 16 named text terms containing 14,319,997 posting IDs in total.

The four alternatives encode **document IDs only**. The baseline is canonical delta varints with a skip directory every 128 IDs. The candidates use delta bit packing in 64, 128, or 256 ID blocks. Each directory entry supports binary search by block maximum followed by an in-block scan. The directory contributes projected 12 bytes per baseline block or 16 bytes per packed block to serialized size. The retained bytes include actual `Vec` capacities and directory entry alignment. These numbers are comparable within this prototype; they are not complete schema-9 postings file sizes.

Each term and codec had seven release-build encode, decode, and seek samples. The candidate order rotated each round. Decode and seek order alternated. Seek samples each used the same 256 evenly spaced universe targets plus the universe sentinel. The table sums **per-term medians**, so its CPU totals are descriptive and are not a timed end-to-end query or concurrent workload.

| Codec | ID bytes, all 16 | Retained capacity | Encode median sum | Decode median sum | 257 seeks median sum |
| --- | ---: | ---: | ---: | ---: | ---: |
| Delta varint + 128 skip | 15,721,855 | 24,195,248 | 34.414 ms | 52.140 ms | 2.103 ms |
| Bit-packed delta 64 | 11,384,661 | 13,927,120 | 56.852 ms | 42.085 ms | 1.763 ms |
| Bit-packed delta 128 | 10,080,335 | 12,495,024 | 53.114 ms | 40.672 ms | 1.834 ms |
| Bit-packed delta 256 | 9,641,171 | 14,071,456 | 55.728 ms | 40.081 ms | 2.126 ms |

Six broad terms (`df >= 500,000`) account for 14,104,473 of the IDs. Their packed 256 blocks have the smallest projected serialized size, 9,316,826 bytes versus 15,428,429 baseline bytes. Their point-seek sum is similar to baseline, 1.151 versus 1.157 ms. The other ten terms contain 215,524 IDs; for them, packed 64 has the fastest point-seek sum, 0.361 ms versus 0.945 ms baseline, but the baseline uses the fewest serialized bytes, 293,426 versus 337,851. These conflicting shapes argue for further distribution and query-level work before choosing a codec or block policy.

This is local, warm, single-process prototype evidence. It excludes frequencies, positions, block bounds for ranking, checksums, the file footer, writer and reader integration, allocation under publication, page ordering, phrase evaluation, exact summaries, and the 2 GiB Fly reader. Only named terms in one schema-6 generation were sampled; the results do not estimate corpus-wide size or prove the 200 ms production gate.
