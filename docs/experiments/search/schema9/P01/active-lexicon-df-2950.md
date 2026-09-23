# P01 active lexicon frequency survey

Status: read-only distribution evidence, not a schema-9 bitmap choice or
performance verdict. The [machine-readable report](active-lexicon-df-2950.json)
was produced by [`analyze-raw-lexicon-df.py`](../../../../../scripts/analyze-raw-lexicon-df.py)
at source `78f18ce8bcf14dfbef80ecc0db7c360970fd0052`.

On 2026-09-23 the started reader Machine `d89303da029328` in
`ytsearch-raw-production`, backed by volume `vol_v3gnjjw9elpy5ew4`, still
pointed to generation 2950. Its schema-6
manifest reported 8,422,419 documents, 35,215 videos, and 302,871 terms.
The lexicon was copied read-only from that generation. Remote and local SHA-256
both equaled `7641ce43647ae24bba7706115bf2d22e5c1280f4292e3657bb985568841b9c9e`;
the manifest SHA-256 was
`b97c8727ccabdbb5a45ac44010f337633849bf527e8d8a516f458d403411d7b5`.
The parsed term count exactly matched the manifest. The report exports no term
bytes or posting payloads.

| Field | Universe | Terms with DF ≥0.1% | ≥1% | ≥10% | Maximum DF | Dense bitmap bytes per term |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Transcript documents | 8,422,419 | 1,469 | 196 | 28 | 4,276,415 | 1,052,808 |
| Title videos | 35,215 | 1,149 | 85 | 7 | 7,153 | 4,408 |

At this corpus size a 64 MiB **payload-only** pool holds at most 63 full
transcript bitmaps. All 28 terms above 10% DF would consume 29,478,624 payload
bytes if selected. The title universe is much smaller, but its field has a
different ranking and provenance role. These are capacity facts, not a choice
of threshold: query mix, overlap, codec bytes, checkpoint directories,
construction peaks, concurrent sessions, and the 2 GiB reader profile still
need measurement. Schema 9 must remain exact with an empty bitmap pool.

The source generation is schema 6. A schema-9 build may change IDs and corpus
membership, so its own footer and term statistics must be checked before using
this distribution to configure production.
