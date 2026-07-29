# Search memory halving strategy

Date: 2026-07-22

Status: implementation-ready research plan; no production collection changes
have been made.

## Executive decision

Keep Typesense as the production engine while building two isolated candidates
against the same frozen corpus:

1. **Typesense v2, the first production candidate:** rebuild transcripts into
   20-second passages, index each video title only once, and stop indexing
   display-only fields. Preserve the current API, grouping, highlighting,
   low-signal ordering, and update semantics.
2. **Tantivy, the long-term disk-backed candidate:** store separate video and
   passage indexes on NVMe through memory maps. Promote it only if it matches
   Typesense relevance and grouping while staying inside the same 4 GB limit.

The near-term objective is a continuously warm 4 GB search node, not
scale-to-zero. The current Typesense snapshot took approximately **8 minutes
46 seconds** to rebuild its in-memory index after a cold start, so full stop and
on-demand wake cannot provide interactive search.

The RAM target is deliberately a gate rather than a forecast. There are two
separate success levels:

- **Strict memory halving:** steady resident memory at or below **1,975 MB**
  and Typesense allocated memory at or below **1,847 MB**;
- **Safe 4 GB deployment:** steady cgroup memory at or below **2.8 GB** and
  peak cgroup memory below **3.4 GB**, or 85% of a 4 GB machine;
- no swap;
- relevance and latency gates described below.

The cost objective is satisfied when the safe 4 GB deployment gate passes,
even if allocator overhead prevents the stricter RSS-halving result. If
Typesense v2 passes, resize only an isolated candidate to 4 GB first. On
Fly's audited `performance-2x/4GB` price, search compute would fall from about
**$147.85 to $73.92 per month**, a 50% reduction, while remaining warm. If it
does not fit safely, do not force the resize: test 6 GB, then continue the
Tantivy experiment.

## What is measured today

| Property | Current evidence |
| --- | ---: |
| Transcript documents in the benchmark snapshot | 7,159,726 |
| Indexed videos in the latest research snapshot | 26,385 |
| Average transcript documents per video | 271 |
| Live steady resident memory | about 3.9 GB |
| Live Typesense disk use | about 2.8 GB |
| Current CLI passage window | 10 seconds |
| Warm Typesense first-result time | typically 7–20 ms |
| Public end-to-end p95 in the existing study | 253–780 ms by case |
| Isolated full cold-load time | about 8m46s |

The isolated cold-start test used a fork of the production Fly volume on the
same 4-performance-vCPU/8 GB shape. The VM itself started in approximately
11 seconds; loading all 7,159,726 transcript documents dominated readiness.

The library constant in `youtube_ingest` remains five seconds, but production
CLI commands currently default to ten seconds. Migration code must use an
explicit window everywhere and must not infer production behavior from the
library constant.

## Why the current index is expensive

Typesense keeps the complete searchable index in memory and the raw documents
on disk. Its official keyword sizing guidance is roughly two to three times the
bytes of fields actually indexed. Fields marked `index:false` remain available
in results from disk and do not build an in-memory search, filter, sort, or
facet index.

The current collection has two multiplicative costs:

1. A transcript is divided into many short documents. Every document has its
   own document and filter/grouping overhead.
2. Every chunk repeats and indexes the video's full title. In the current
   snapshot that means an average title is indexed approximately 271 times.

Previous schema hygiene was correct but insufficient. Making `prev_text`
display-only reduced resident memory by only 48 MB, or about 1.2%, and changing
`channel_id` produced no measurable reduction. The large levers are passage
count and repeated searchable metadata.

Longer passages do **not** reduce transcript word volume by 59%. Most transcript
terms and positions still exist. They reduce per-document structures, repeated
filter values, repeated metadata, and some posting entries. This distinction is
why the full-corpus memory gate is mandatory.

## Corpus evidence for passage size

A measurement over 498 real cached transcripts using the production chunking
algorithm produced:

| Window | Documents | Change from 10s | Mean words |
| ---: | ---: | ---: | ---: |
| 10 seconds | 145,254 | baseline | 19 |
| 15 seconds | 84,310 | -42.0% | 33 |
| 20 seconds | 59,538 | -59.0% | 46 |
| 30 seconds | 37,614 | -74.1% | 73 |

Applying the measured 20/10-second ratio to the newer 7,159,726-document
snapshot projects approximately **2.93 million** v2 passage documents. This is
a document-count projection, not a RAM measurement.

Twenty seconds is the chosen candidate because 46-word passages remain compact
enough for the current UI while eliminating substantially more documents than
15 seconds. Thirty seconds is rejected as the first candidate because its
73-word average creates a larger timestamp and snippet-quality risk.

If the 20-second collection fails the relevance or timestamp gates, the only
specified fallback is 15 seconds with the same sparse-title schema. Do not tune
an arbitrary intermediate value against the held-out test set.

## Typesense v2 document design

Continue using one transcript collection. A second title collection, JOIN, or
union search is unnecessary for the first candidate and would complicate
totals, grouping, pagination, and failure handling.

Add one optional field:

```text
search_title: Some(video title) only when sequence == 0
search_title: absent on all other passage documents
```

Keep `title` on every document for API display, but make it disk-only. Searching
one title-bearing document per video reduces the number of documents carrying
searchable title postings by approximately **99.63%** in the current snapshot.

### Target transcript schema

| Field | v2 behavior | Reason |
| --- | --- | --- |
| `id` | built-in/indexed | Stable upsert/delete identity |
| `text` | indexed | Primary lexical passage search |
| `search_title` | indexed, optional | Title search on sequence zero only |
| `title` | stored, `index:false`, optional in schema | Public display value |
| `video_id` | indexed and `facet:true` | Exact filter, grouping, distinct count |
| `youtuber_id` | indexed | Admin deletes/counts and canonical creator filter |
| `youtuber_handle` | indexed and `facet:true` | Preserve the current public creator filter |
| `youtuber_name` | stored, `index:false`, optional in schema | Public display |
| `published_at_timestamp` | indexed/sortable | Newest/oldest ordering |
| `duration` | stored, `index:false`, optional in schema | Short/video display classification |
| `channel_id` | stored, `index:false`, optional in schema | Display/metadata |
| `prev_text`, `next_text` | stored, `index:false`, optional | Readable context |
| `sequence` | stored, `index:false`, optional | Passage identity/debugging |
| chapters and timestamps | stored, `index:false`, optional | Display/jump target |
| thumbnail/date/uploader strings | stored, `index:false`, optional | Display only |
| embeddings | absent | Vector/model memory is out of scope |

Keep both creator ID and handle filtering in the first production candidate.
This avoids adding a handle-to-ID dependency to the public search path during
the same migration. If the target narrowly misses the 4 GB gate, benchmark an
additional ablation that resolves handles through `youtube_youtubers` and
makes `youtuber_handle` display-only. Do not accept that extra complexity
without a measured memory benefit.

Keep `video_id` faceted because Typesense requires a faceted field for
`group_by`. Do not enable `infix`, range indexes, or vectors. Stopword sets do
not reduce index RAM because stopwords are removed from queries rather than from
the indexed corpus.

### Query contract

Change the internal Typesense query from:

```text
query_by=text,title
highlight_fields=text,title
```

to:

```text
query_by=text,search_title
highlight_fields=text,search_title
```

Add explicit query-field weights to the shared `TextSearchQuery` type and the
Typesense URL builder rather than depending on implicit field order. Preserve
the current effective preference as a frozen fixture before changing it.

When Typesense returns a `search_title` highlight:

- expose it through the existing public `title_highlight` property;
- classify the hit as `MatchKind::Title` when there is no text highlight;
- link to the video root without a timestamp;
- return the stored `title`, never the internal sparse field;
- keep at most one title-only hit per video through the existing deduper.

No public HTTP response type changes are required.

Low-signal behavior remains unchanged. Queries such as `I` use the existing
newest-for-filter path and `published_at_timestamp`, and must not suddenly
require all terms. Stopwords such as `on` must not dominate `fable on high`.

## Passage construction

Use a checksummed provider-neutral export of the current collection as the
authoritative migration input. Captions can disappear from YouTube and the
local caption cache is not guaranteed to be complete, so a production rebuild
must not depend on refetching every video.

Sort exported documents by `(video_id, sequence)`, strip generated neighbor
context, and treat their ordered text/timestamps as source segments for the
20-second regrouping pass. Before import, require the normalized concatenated
text hash and video set for every source video to match the export manifest.
Where original cached caption segments are available, compare both rebuild
paths on a representative subset to quantify timestamp drift.

For every passage:

- `t_start_ms` is the first valid caption segment start;
- `t_end_ms` is the maximum included caption segment end;
- `prev_text` and `next_text` are generated after 20-second grouping;
- `search_title` is populated only on sequence zero;
- all existing publication and creator metadata remain stored;
- document IDs continue to use video plus sequence within the versioned
  collection.

Changing the incremental default without rebuilding would mix passage sizes.
Therefore v2 must be a new collection populated by a full rebuild. Future
updates to v2 always pass 20 seconds explicitly.

## Code changes

### `youtube_ingest`

- Keep the shared `TranscriptChunk` model backend-neutral.
- Add 10-, 15-, and 20-second deterministic fixtures that assert boundaries,
  context fields, IDs, and timestamps.

### `youtube_search`

- Add an optional `query_by_weights` property to `TextSearchQuery`.
- Keep the backend trait unchanged.
- Add builder and contract tests for the sparse-title query.

### `youtube_typesense`

- Add an explicit `TypesenseLayout` setting with `classic_v1` and `compact_v2`
  variants.
- Derive `search_title` only in compact-v2 import serialization when
  `sequence == 0`; leave classic-v1 serialization byte-compatible.
- Add `search_title` to the versioned schema.
- Change `title`, `youtuber_handle`, `youtuber_name`, and `duration` to
  display-only fields in v2, except keep `youtuber_handle` indexed in the first
  production candidate.
- Keep `video_id` faceted.
- Serialize `query_by_weights` to Typesense.
- Add schema tests that enumerate every memory-resident field.
- Add a test that display-only fields remain returned after indexing.
- Persist layout and chunk seconds in the collection build manifest and refuse
  imports or updates whose configured layout does not match the collection.

### `youtube_api`

- Query and highlight `text,search_title`.
- Map the internal highlight to `title_highlight`.
- Preserve title-only classification, URLs, grouping, page totals, and
  low-signal sorting.
- Add fixtures for title-only, title-plus-transcript, filtered creator,
  stopword-heavy, low-signal, and pagination behavior.

### `youtube_cli` and scripts

- Make every admin/count client honor the explicitly selected transcript
  collection. The current `youtuber_client()` path must not silently bind
  shadow operations to the default collection.
- Require `--chunk-seconds 20` in v2 build wrappers.
- Give the physical collection a versioned name such as
  `youtube_transcript_chunks_20s_sparse_title_v1`.
- Introduce the stable alias `youtube_transcript_chunks_active`.
- Refuse v2 update/rewipe wrappers when the configured chunk size is absent or
  not 20.
- Preserve the existing explicit destructive acknowledgement for rewipe.

## Full-corpus experiment

Use isolated machines and a frozen source snapshot. Do not build the second
full collection beside production on the current 8 GB node.

Create these candidates:

| Candidate | Purpose |
| --- | --- |
| A: current 10s schema | Same-snapshot control |
| B: 10s sparse title | Attribute sparse-title savings independently |
| C: 15s sparse title | Lower-risk timestamp fallback |
| D: 20s sparse title | Production candidate |

Promote D if it passes every gate. Promote C only if D fails a
relevance/timestamp gate and C still fits safely in 4 GB. This experiment
ordering separates the two major memory levers instead of attributing both to
one result.

For each candidate:

1. Import the identical creator/video set with `JOBS=1`.
2. Fail on any per-row import error even if the HTTP response is 200.
3. Record raw input bytes, document count, distinct videos, index disk bytes,
   import duration, and peak import memory.
4. Perform a full stop/start and poll readiness every five seconds.
5. Record system/cgroup memory, RSS, Typesense allocated/active/resident/retained
   memory, CPU, disk, and swap every five seconds.
6. Clear or normalize the Typesense query cache before comparable runs.
7. Alternate baseline and candidate tests at least five times.
8. Run bounded updates concurrently with search using the production
   single-writer policy.

### Hard memory gates

Candidate D achieves strict memory halving only if, after cold restart:

- allocated memory is at most 1,847 MB;
- resident memory is at most 1,975 MB;
- steady cgroup use is at most 2.8 GB;
- startup and update peak cgroup use is below 3.4 GB;
- swap remains zero;
- the engine remains healthy for a 24-hour soak.

Candidate D or C may still authorize a 4 GB resize if it misses only the strict
allocated/RSS targets but stays within the 2.8 GB steady and 3.4 GB peak cgroup
limits. Steady RSS alone never authorizes a resize.

### Relevance gates

Create at least 100 privacy-safe judged queries with tuning and held-out splits:

- exact quotes and names;
- code identifiers and punctuation-sensitive terms;
- title-only queries;
- transcript-only queries;
- multi-term queries and phrases;
- stopword-heavy queries;
- short and low-signal queries;
- creator-filtered queries;
- recent/newest and oldest sorting;
- near-duplicate moments.

Required fixed cases include:

- `I` filtered to `@t3dotgg`, with newest videos first;
- `fable on high` filtered to `@t3dotgg`, with the Fable guide near the top
  and a relevant transcript moment;
- `linux exploit`;
- `battery gate incident`;
- a title-only hit that produces no timestamp;
- a direct `video_id` filter.

Promotion requires:

- no fixed-case regression;
- held-out nDCG@10, MRR@10, and Recall@10 loss no greater than 2%;
- no creator-filter leakage;
- stable video ordering across repeated runs;
- median click-to-judged-moment error at most 10 seconds;
- p95 click-to-judged-moment error at most 20 seconds.

The existing API may merge adjacent hits separated by as much as 20,000 ms.
Test that boundary explicitly against 20-second passages. Passage and moment
counts are expected to change; exact source-video set and normalized per-video
text hashes are the migration parity requirements.

### Latency and capacity gates

On identical hardware:

- zero 5xx responses;
- warm backend p95 no more than 1.2 times the control;
- warm backend p95 at or below 250 ms under the agreed concurrency test;
- cold first-query p95 recorded separately;
- public end-to-end p95 does not regress more than 10%;
- result and response cardinalities remain internally consistent;
- indexing with `JOBS=1` does not push search p95 above 500 ms;
- full cold readiness improves from the measured 8m46s and remains below five
  minutes.

The current public tail is much larger than backend search time, so a small
backend change is not automatically user-visible. Search correctness and
memory are the primary v2 decision axes.

## Delivery effort and calendar

These estimates assume one engineer working primarily on the migration and
that the frozen export is healthy. They include implementation and verification
but not passive observation windows.

### Compact Typesense v2

| Work | Engineering effort | Typical elapsed time |
| --- | ---: | ---: |
| Export/regroup tool, manifests, and hash verification | 1–2 days | 1–2 days |
| Versioned schema/layout and sparse-title serialization | 1–2 days | 1–2 days |
| Query/highlight compatibility and regression tests | 1–2 days | 1–2 days |
| Full-corpus A/B/C/D imports and measurements | 1–2 days active | 2–4 days |
| Alias, dual-write, alerting, and rollback rehearsal | 1–2 days | 1–2 days |
| Production observation | little active work | 24-hour soak plus 48-hour rollback window |

Expected total: **6–10 engineering days** and roughly **10–14 calendar days**
for a careful production migration. A seven-day shadow-traffic period would
make the calendar closer to **two to three weeks** without adding a full week
of engineering labor.

The first useful result arrives earlier: candidate B reveals sparse-title
savings, and candidate D reveals whether 4 GB is viable, after approximately
three to five focused engineering days plus the full-corpus build time.

### Tantivy-backed ytsearch index

A minimal read-only Tantivy proof of concept can be built in **5–8 engineering
days** because `TranscriptSearchBackend` already isolates the API from the
engine. A production replacement is estimated at **15–25 engineering days**
and **four to six calendar weeks**, including:

- separate video and passage indexes;
- title/passage rank fusion and grouped pagination;
- exact or explicitly revised result-count semantics;
- highlighting, typo behavior, creator filters, and low-signal sorting;
- a private remote ingest/update service;
- generation-based commits, crash recovery, backups, and rollback;
- corpus benchmarks and a shadow-traffic period.

### Writing an engine from first principles

Do not do this for the cost-saving project. Implementing the inverted index is
only the beginning; segment merging, deletion, durability, query parsing,
scoring, snippets, typo lookup, concurrency, and corruption recovery dominate
the work. A credible single-node first version is at least **8–16 engineering
weeks**, followed by ongoing engine maintenance. Tantivy provides those
low-level mechanisms while still allowing ytsearch-specific ranking and
grouping.

## Migration and rollback

1. Create `youtube_transcript_chunks_active` pointing to the current physical
   collection.
2. Deploy API and writers using the alias while it still targets v1.
3. Verify no behavior or count change.
4. Build candidate D as a separate physical collection on isolated capacity.
5. Start dual writes only after the full backfill is caught up.
6. Soak for at least 24 hours with count divergence equal to zero.
7. Pause the single writer briefly and reconcile final counts.
8. Atomically move the alias to v2.
9. Keep v1 intact for at least 48 hours.
10. Roll back by moving the alias to v1 if any memory, error, latency, count, or
    relevance alarm fires.
11. Delete v1 only after an off-provider export and an isolated restore drill.

Typesense documents aliases specifically for zero-downtime schema changes and
recommends temporary dual writes before switching.

## Cost outcomes

Using the audited Fly `sjc` prices:

| Outcome | Search compute | Difference from current |
| --- | ---: | ---: |
| Current 4-performance-vCPU/8 GB | $147.85/month | baseline |
| 2-performance-vCPU/8 GB | $97.78/month | save $50.07 |
| 2-performance-vCPU/4 GB | $73.92/month | save $73.93 |

The 10 GB volume remains approximately $1.50/month in every shape. API,
indexer-active hours, snapshots, network, Cloudflare, and tax are separate.

The 4 GB shape is selected only after the hard memory and concurrency gates.
If Typesense v2 needs more than 4 GB but fewer CPUs pass, the 8 GB/two-CPU shape
still saves about 34%.

Do not use full stop/start as the primary saving mechanism. It saves all
compute while stopped, but the measured recovery delay is incompatible with
ordinary interactive traffic. Fly also discourages suspend for machines larger
than 2 GB, and suspend snapshots are not guaranteed to survive deploys or host
migration.

## Tantivy disk-backed track

Tantivy is the best long-term disk-backed candidate because it is a Rust
library, uses memory-mapped index files, supports BM25, phrase and Boolean
queries, fast fields, facets, incremental commits, snippets, and custom
collectors. Opening an existing index should avoid Typesense's complete
in-memory rebuild, but actual cold queries will depend on NVMe and the kernel
page cache.

Use two physical indexes:

- `videos`: one document per video containing indexed title, creator keys, and
  publication sort data;
- `passages`: 20-second searchable transcript passages with numeric video and
  creator keys, sequence, and timestamps.

Query both, fuse title/video and passage candidates in Rust, keep at most five
moments per video, and then reuse the API relevance pass. Low-signal queries can
route directly to the video index sorted by publication time.

Implement `TranscriptSearchBackend` and initially emit the Typesense-shaped
internal JSON expected by the API so the public response remains unchanged.

The difficult parts, and therefore explicit prototype gates, are:

- a custom or over-fetching video-group collector;
- exact distinct-video counts for broad text queries;
- stable grouped pagination;
- typo-tolerance parity;
- an authenticated private ingest service because the Fly indexer cannot
  mutate an embedded index on another machine;
- generation-based index swap and crash recovery.

Run Tantivy under a hard 4 GB cgroup with:

- steady process heap plus cgroup use measured separately;
- steady cgroup use at most 2.8 GB;
- peak at most 3.4 GB;
- ready-to-query target at most 15 seconds;
- cold p95 at most 750 ms;
- warm p95 at most 250 ms;
- the same relevance, count, update, and failure gates as Typesense v2.

Memory-mapped page cache is reclaimable, but it still consumes real cgroup and
physical memory while hot. Do not report heap alone as Tantivy's memory use.

### Alternatives not selected

- **OpenSearch/Lucene:** strong highlighting, filters, collapse, and inner hits,
  but the JVM heap plus filesystem cache makes a 4 GB target risky and
  operationally heavier.
- **Quickwit:** excellent object-storage-backed Tantivy architecture for
  append-heavy event data, but its update/delete limitations conflict with
  creator refreshes and video replacement.
- **SQLite FTS5:** valuable small prototype with BM25, phrases, snippets, WAL,
  and instant database opening, but weaker typo tolerance and unproven top-five
  grouping over this corpus.
- **Typesense plus swap:** rejected. Typesense expects the searchable index in
  physical RAM; swap creates unpredictable latency and is not a disk-search
  mode.

The first candidate to pass the common 4 GB, relevance, and p95 gates wins.
Typesense v2 gets production priority because its operational and relevance
behavior already matches the application. Tantivy becomes the preferred
long-term engine if corpus growth makes in-memory indexing uneconomical.

## Monitoring after promotion

Alert on:

- steady memory above 70%;
- peak memory above 85%;
- swap above zero;
- disk above 70%;
- backend p95 above 250 ms;
- end-to-end p95 above the accepted release baseline;
- any 5xx search response;
- collection count divergence;
- queue/import failures;
- cold readiness above five minutes.

At 70% sustained RAM, project 90-day corpus growth and begin the next capacity
test. Scale vertically before introducing a three-node Typesense cluster.

## Required verification

```bash
cargo fmt --check
cargo test -p youtube_api -p youtube_typesense -p youtube_search -p youtube_ingest
bun run build
bun run frontend:check
scripts/check-frontend-backends.sh
RUNS=10 scripts/benchmark-search-cases.sh
scripts/check-indexed-channel-status.sh @t3dotgg
```

Also perform:

- full-corpus count and checksum verification;
- five alternating baseline/candidate benchmark runs;
- concurrent `JOBS=1` update/search test;
- cold stop/start measurement;
- 24-hour dual-write soak;
- alias rollback rehearsal;
- off-provider JSONL export and isolated restore.

## Sources

Primary engine documentation:

- [Typesense system requirements](https://typesense.org/docs/guide/system-requirements.html)
- [Typesense collection fields and `index:false`](https://typesense.org/docs/30.2/api/collections.html)
- [Typesense search, grouping, and field weights](https://typesense.org/docs/30.2/api/search.html)
- [Typesense server configuration](https://typesense.org/docs/30.2/api/server-configuration.html)
- [Typesense cluster memory metrics](https://typesense.org/docs/latest/api/cluster-operations.html)
- [Typesense production guidance](https://typesense.org/docs/guide/running-in-production.html)
- [Tantivy project and feature overview](https://github.com/quickwit-oss/tantivy)
- [Tantivy API documentation](https://docs.rs/tantivy/latest/tantivy/)
- [Tantivy memory-mapped directory](https://docs.rs/tantivy/latest/tantivy/directory/struct.MmapDirectory.html)
- [Lucene memory-mapped directory](https://lucene.apache.org/core/10_3_1/core/org/apache/lucene/store/MMapDirectory.html)
- [SQLite FTS5](https://www.sqlite.org/fts5.html)
- [SQLite memory-mapped I/O](https://www.sqlite.org/mmap.html)
- [Quickwit querying architecture](https://quickwit.io/docs/overview/concepts/querying)
- [Quickwit Elasticsearch API limitations](https://quickwit.io/docs/reference/es_compatible_api)
- [OpenSearch system configuration](https://docs.opensearch.org/latest/install-and-configure/configuring-opensearch/configuration-system/)

Retrieval research already reviewed for this repository:

- [BEIR heterogeneous retrieval benchmark](https://arxiv.org/abs/2104.08663)
- [DAPR document-aware passage retrieval](https://arxiv.org/abs/2305.13915)
- [Spoken document retrieval with neighboring context](https://aclanthology.org/L10-1319/)
- [Passage segmentation study](https://aclanthology.org/W08-1803/)
- [Reciprocal Rank Fusion](https://plg.uwaterloo.ca/~gvcormac/cormacksigir09-rrf.pdf)

Repository evidence:

- `docs/typesense-memory-optimization.md`
- `docs/performance-research.md`
- `docs/cold-warm-concurrency-costs-2026-07-17.md`
- `docs/disaster-recovery.md`
