# ytsearch Requested Behavior TODO

This file tracks the current requested work and end-to-end verification. Mark any discovered or introduced regression in the Regression Log before changing behavior further.

## Raw Search Production Readiness (active)

Checked items are implemented and verified. Unchecked items are required before
Raw can replace Typesense as the canonical search index.

### Completed foundation

- [x] Build and serve an immutable Raw index snapshot.
- [x] Store term frequencies and first-token positions in schema v5.
- [x] Preserve exact query-token order during conjunctive matching.
- [x] Apply bounded per-token typo matching and ordered token dropping.
- [x] Reject unsupported search parameters instead of silently ignoring them.
- [x] Rank with text relevance plus likes, views, comments, and recency.
- [x] Keep the experimental browser entry bundle below the 30 KB gzip limit.
- [x] Preserve the retired Tantivy experiment and results outside production.
- [x] Build, checksum, contract-test, and deploy the full 8,142,650-document,
  32,822-video, 61-channel schema-v6 snapshot to the isolated Raw experiment.
- [x] Promote that checksum-identical snapshot to the production Raw reader on
  a 4 GiB / 2-performance-vCPU machine, route `search.pronsh.dev` reads to it,
  and retain Typesense only for rollback and the not-yet-migrated write path.
- [x] Retire the experimental frontend, API, and engine after production browser,
  contract, and concurrent-load gates pass.

### P0: relevance and compatibility

- [x] Add schema v6 complete positional postings.
  - [x] Define an allocation-bounded delta codec with corruption and size tests.
  - [x] Keep first positions in a compact hot sidecar for ordinary queries.
  - [x] Delta-encode all remaining positions into a cold sidecar.
  - [x] Checksum and length-check every generation file at startup.
- [ ] Add exact quoted phrases, phrase slop, proximity, and early-position ranking.
  - [x] Parse and execute bounded exact quoted phrases from schema-v6 positions.
  - [x] Preserve order and duplicate-token semantics inside quoted phrases.
  - [x] Add bounded phrase slop and general unquoted proximity ranking.
  - [ ] Judge and gate proximity/early-position score changes on the oracle.
- [ ] Finish multi-token fallback parity with a bounded typo/token-drop work budget.
- [ ] Add quoted/negative terms, duplicate-token semantics, Unicode and numeric rules.
  - [x] Preserve exact duplicate-token semantics inside quoted phrases.
  - [x] Add exact token-start negative terms with pre-threshold exclusion.
  - [x] Add exact negative phrases with grouped-search semantics.
  - [ ] Finish Unicode normalization and numerical-token typo rules.
- [ ] Add safe phrase-aware, configurable multi-snippet highlights and match metadata.
  - [x] Highlight complete token boundaries, prefer a hit's original exact
    tokens over fuzzy alternatives, and regression-test the reported CSS query.
  - [x] Preserve alternatives by originating query token through candidate
    generation, including typo and one-to-many split/plural replacements, so
    each hit highlights only relevant alternatives instead of a global union.
- [x] Fail closed on every unknown filter, sort, facet, operator, and malformed value in the active application contract.
- [ ] Add a versioned 300-query human-judged relevance oracle.
- [x] Cover global and channel-specific `blackhole` searches for Veritasium and Vsauce.
- [ ] Differential-test the complete application contract against Typesense.
  - [x] Run the first full-snapshot seven-case ordered top-10 differential;
    mean overlap is 7.71/10 and top-one agreement is 4/7.
  - [ ] Expand the differential to the versioned 300-query human-judged oracle.

### P1: canonical write path

- [x] Add a backend-neutral search-index administration contract.
- [x] Adapt Typesense administration behind the shared contract during migration.
- [x] Route CLI and repair jobs through the shared administration contract.
- [x] Add a durable Raw writer with SQLite WAL and idempotency keys.
- [x] Add atomic replace/delete video and upsert/delete channel operations.
- [x] Add counts, channel statistics, indexed-video pagination, and operation status.
- [ ] Publish immutable delta segments and tombstones without rewriting the base.
- [ ] Make acknowledged writes visible on both readers within 60 seconds at p95.
- [x] Add checksummed atomic generations, crash recovery, and one-generation rollback.
- [x] Replace full-generation publication with streaming bounded-memory compaction.
  - [x] Coalesce explicit publication requests to a captured operation sequence;
    mutations arriving during a build remain durable for the next requested
    generation instead of triggering an unbounded rebuild loop.
  - [x] Remove abandoned `work/*` and `*.candidate` artifacts before recovery.
  - [x] Eliminate the 6.6 GiB uncompressed merged-JSON scratch file by streaming
    the filtered base plus replacements directly into the builder.
  - [x] Replace the retained in-memory postings builder with lexicographically
    ordered, recursively bounded external runs and stream document metadata.
- [ ] Move full-generation construction off the always-on reader.
  - [x] Stop launching an implicit full-corpus publication during reader startup;
    publication now requires an explicit authenticated request.
  - [x] Pin mutation snapshots in an explicit SQLite read transaction and export
    an immutable, checksummed, read-only backup through one captured sequence.
  - [x] Implement detached build/verification for the existing on-demand SJC
    indexer and stream nine allowlisted files with corpus/schema/size/hash gates.
  - [x] Atomically activate the transferred generation, mark only the captured
    sequence published, and leave later mutations durable for the next batch.
  - [ ] Pass the full-corpus 2-GB indexer disk/RSS gate and the continuous
    search-load gate before deploying the external builder.
  - [ ] Fail closed on local reader-side builds in production after the remote
    builder passes crash, disk-full, upload-resume, and live latency gates.
- [ ] Upload checksummed versioned artifacts to private R2 once R2 is enabled.
- [x] Bootstrap Raw from the final matched snapshot and remove dual-writing from
  the repository. The deployed legacy queue remains a cutover task below.

### P1: indexing reliability and throughput

- [ ] Add bounded fetch concurrency with per-host and per-channel limits.
- [x] Classify retryable, terminal, private, deleted, and unavailable-video
  failures in an atomic, versioned per-video ledger. Legacy cooldown markers
  remain readable and are upgraded on the next failure.
- [x] Persist bounded exponential backoff with stable per-video jitter and
  resume incomplete channels after restarts through the durable gap cursor.
- [ ] Batch by bytes and document count; avoid retaining transcripts twice.
- [ ] Emit progress, retry, failure-reason, throughput, and freshness metrics.
- [x] Add `scripts/index-all-raw.sh` after restart and recovery tests passed;
  the canonical list now covers all 61 live creator profiles and one CLI process
  coalesces their mutations into a single verified generation.

### P2: operations and cutover

- [ ] Run one writer and two independently refreshing Raw readers.
- [x] Add health, readiness, generation, pending-sequence, operation, and
  publication-integrity endpoints.
- [ ] Test crashes, partial uploads, disk full, corruption, and stale readers.
- [ ] Shadow production queries and compare ordered results.
- [ ] Roll out at 1%, 10%, 50%, and 100% with automatic rollback thresholds.
- [x] Remove Typesense from the live search-read path after a checksum, contract,
  browser, and 75-request concurrent-load gate.
- [x] Remove Typesense from the repository write/control path after the durable
  Raw writer, generation publication, and recovery gates passed locally.
- [ ] Deploy that write/control path, copy and drain the durable queue, prove a
  mutation across restart, then deprecate and preserve the exact legacy Fly
  resources in `docs/operations/typesense-retirement.md`.
  - [x] Deploy the mutable Raw engine on its encrypted 20 GB volume and the Raw
    queue API on its encrypted 1 GB volume.
  - [x] Copy all five durable legacy queue rows without changing their request
    IDs, then process and acknowledge `1786191981-typesense-fly-t3dotgg` through
    Raw (95 videos / 30,684 transcript chunks imported).
  - [x] Prove idempotent replace and delete operations, publication, and both
    post-publication Machine restarts without leaving the synthetic document.
  - [x] Deploy `search.pronsh.dev` with Raw-only search and queue origins.
  - [x] Finish the nine-channel drift reconciliation, publish generation 1166,
    replay the interrupted Veritasium video into bounded generation 1167, and
    pass a post-publication restart.
  - [x] Pass final search, queue, sort-validation, last-page, bundle, and
    75-request/five-concurrent production load gates.
  - [x] Mark the four exact legacy Fly Machines deprecated with explicit Raw
    successors, stop all four, and disable autostart on the former engine/API;
    no Machine, volume, or app was destroyed.
  - [x] Observe the current production visual browser gate: T3 rendered twelve
    Raw-ranked cards, every active sort, 24/7 availability, and the 21-video
    Veritasium `blackhole` scope without an alert.
  - [ ] Finish the classified 61-creator repair and post-publication gates while
    keeping the four exact legacy Machines stopped and all seven verified
    volumes preserved. Any destruction requires a new explicit decision.

### Non-negotiable performance and resource gates

- [x] Schema v6 non-phrase p95 is no more than 10% slower than schema v5.
- [x] Raw p95 latency and CPU/query are each at most 75% of Typesense on the
  matched 1,956-video comparison (71% lower p95 and 73% lower CPU).
- [x] Raw steady-state RSS is at most 60% of Typesense. The full Raw snapshot
  warmed to 347 MiB versus about 3.9 GiB for the restored Typesense clone.
- [x] Standalone full-snapshot compaction peak RSS stays below 512 MiB. The
  release-build bounded compactor peaked at 457,928 KiB (447.2 MiB), down from
  2,575,380 KiB (2.46 GiB), while reproducing every search-artifact checksum.
- [x] Live publication stays below the 4 GiB allocation while the old reader
  remains queryable. Generation 1167 peaked at 753,764 KiB process RSS; a clean
  restart settled at 359,004 KiB RSS.
- [x] Complete-position artifact growth is measured and documented.
- [x] Benchmarks name corpus, hardware, build mode, concurrency, and cache state.
- [x] Every optimization has a correctness invariant and rollback condition.
- [ ] Every representative global and creator-scoped query completes within
  200 ms during baseline, mutation-snapshot export, generation transfer,
  activation, and warmed-new-generation phases. The isolated 8.32M-document
  reader passed baseline (57/57, p95 143.3 ms, max 147.5 ms), but same-reader
  publication still produced multiple 5-second timeouts and is rejected.
- [x] Experimental browser JavaScript remains at or below 30 KB gzip (24,340
  bytes for the initial route after the full-corpus deployment).
- [x] Production browser JavaScript remains below 30 KB gzip after cutover and
  the no-stale-search-cache fix (24,492 bytes for the initial route).

### Documentation and evidence

- [x] Keep common experiment outputs in one versioned Markdown report.
- [ ] Document schema layouts, scoring, visibility, recovery, and rollback. (Schema-v6 layout and rollback are recorded; atomic visibility/recovery remain open.)
- [x] Record debug/release/Clippy/test/benchmark/RSS/artifact-size results.
- [x] Record the production cutover topology, exact corpus parity, resource
  allocation, rollback IDs, browser evidence, and post-cutover load result.
- [ ] Update this ledger in the same commit that completes each item.

## Regression Log

- [ ] Full-corpus generation publication shares the reader's 2-vCPU/2-GB
  cgroup and causes synchronized multi-second search stalls. A clean canary
  baseline passed the 200 ms ceiling, but requests timed out above five seconds
  immediately after publication began. Keep full builds off the serving reader;
  the remaining work is the full-corpus external-builder gate and deployment.
- [x] Single-token compact-split fallback treated `react` as `re + act` and
  `fire` as `fi + re`, returning unrelated results and driving 11-13 second
  backend searches. Split recovery now runs only after exact/prefix results are
  insufficient and requires at least three characters on both sides.
- [x] `coding is dead` reduced 36,114 `coding` candidates to 356 with `dead`,
  then scanned an estimated 2.57 million `is` postings. A guarded document-local
  conjunction path now handles 129-512 survivors only when the remaining
  posting work is at least 512 times larger; full-corpus p95 improved from
  61.9 ms to 48.4 ms with byte-identical responses across the query matrix.
- [x] Raw generation construction re-read and hashed about 1.445 GB of output
  immediately after the builder had already checksummed it. Candidate activation
  now reuses the trusted build proof while normal startup/recovery retains full
  checksum validation.
- [x] `Seek::stream_position` implicitly reached `ColdWriter::seek` once per
  document block, turning offset reads into roughly 127,000 `sync_data` calls.
  Seeks now flush only for cursor correctness; final file/tree fsync and atomic
  activation remain the durability boundary.

- [x] A scoped long query paid global-corpus posting and position costs, while
  an exact phrase outside the selected creator could suppress that creator's
  prefix/typo fallback. The rarest posting is now filtered before later terms,
  fuzzy combinations use the same bounded conjunction, and an adversarial
  cross-creator test prevents the recall leak.
- [x] Dynamic search JSON was edge-cacheable and a zone override retained it
  for four hours, so a cache hit could display a previous request's 680 ms or
  6,140 ms origin sample. API, Worker subrequest, CDN, browser, and frontend
  persistent caches now fail closed with `no-store`; the UI separately reports
  the current browser request duration and backend search duration.
- [x] A typing prefetch and a later submitted search shared the prefetch's start
  timestamp, overstating the wait visible to the submitting user. Fetch work is
  still coalesced, but each caller now measures its own wait interval.
- [x] A creator could publish some videos, return success despite retryable
  per-video gaps, and be deleted from the Cloudflare queue. Retryable gaps now
  fail the indexing pass, return the request to `queued`, retry idempotently on
  a later pass, and acknowledge only after a checked successful deletion;
  transport and malformed queue responses fail closed.

- [x] The Pacific-hours lifecycle policy stopped an active repair at midnight
  and intentionally made production search unavailable for half the day.
  Production now uses an explicit `always_on` mode; the five-minute reconciler
  restarts a stopped Raw reader, while the repair indexer retains its separate
  five-minute idle shutdown.
- [x] Transcript failure markers conflated rate limits, bot challenges,
  premieres, captionless/private/deleted videos, timeouts, chunking errors, and
  Raw import failures. Replaced them with crash-safe classified records,
  category-specific retry policy, stable jitter, terminal deferral, and
  per-run category summaries.
- [x] yt-dlp's literal `rate-limited` diagnostic was falling through to
  `unknown`, and a full repair continued requesting later videos after the
  provider blocked the session. The classifier now recognizes that wording,
  reclassifies preserved unknown records by reason, uses a one-hour jittered
  cooldown, and aborts every remaining creator task without publishing a
  partial generation.
- [x] Direct YouTube caption-track URLs failed for videos whose English
  subtitles were still retrievable through `yt-dlp`, leaving repairable corpus
  gaps. Added one bounded subtitle-download fallback that reuses the existing
  parser, cache, timeout, classification, and circuit-breaker contracts; live
  John Hammond and Asianometry canaries recovered, while truly captionless
  videos remain terminally classified.
- [x] YouTube's `We're processing this video. Check back later.` response was
  stored as `unknown`. It is now classified as retryable `scheduled`, including
  reclassification of preserved unknown records through the current retry
  policy, so processing uploads cannot become permanent unexplained gaps.
- [x] Mutable creator counts compared canonical IDs such as
  `bettersoftwareconference` with handle-form mutation IDs such as
  `@BetterSoftwareConference` byte-for-byte, temporarily reporting 0/15 while
  a repair was pending. Counts and published ownership now ignore an optional
  leading `@` and ASCII case, with pre- and post-publication regression tests;
  production reports the correct 15/15.
- [x] The first creator-ID compatibility fix still treated legacy
  `@cursor_ai` and canonical `cursor-ai` as different, briefly computing 5/51
  in the pending control plane. Raw now applies the same allocation-free ASCII
  canonicalization used when creator IDs are created: optional `@`, lowercase,
  punctuation-run collapse, and no trailing separator. The corrected live
  pending count is 50/51 before publication.
- [x] Prefix candidates were skipped whenever five exact candidates existed,
  hiding valid videos for multi-token queries such as `linux exploit`. Prefix
  expansion now participates before the typo threshold; the full corpus returns
  the same 189-video recall as the Typesense control.
- [x] Startup recovery and `publish_pending` chased every mutation sequence while
  a batch writer was active, repeatedly rebuilding the entire corpus and leaving
  abandoned scratch directories. Publication now targets a captured explicit
  sequence, coalesces only explicit requests, and removes uncommitted scratch.

- [x] A browser tab could retain a pre-cutover Typesense search response because
  the API advertised a public stale-while-revalidate window. Search and
  video-moment reads now use `cache: "no-store"`; a clean production browser
  rendered Raw's 45-video/195-moment contract and Raw-ranked first result.
- [x] Experimental frontend creator metadata could survive a backend/corpus swap for seven days, briefly requesting images for creators absent from the active Raw generation. Versioned the cache, reduced its paint-only lifetime to 30 minutes, and retained network refresh as the authority.
- [x] A queued channel could not be cleanly removed through a guarded operator workflow. Added explicit queue cancellation guidance and made `remove-youtuber` require `--allow-destructive-remove` before deleting both transcript chunks and the channel profile.
- [x] The persistent worker could sit idle for up to 24 hours after a completed repair pass even while channel cards reported indexable gaps. Reduced the post-pass cooldown to five minutes, retained immediate startup repair and single-writer backpressure, and verified the live worker entered background repair after the request queue emptied.
- [x] Production Pages still embedded the retired `ytsearch-index-queue-staging` origin after queue ownership moved into the active Typesense API, causing browser `NetworkError` failures before requests reached the worker. Redeployed production with the active API for both search and queue routes, browser-queued `@lauriewired`, verified indexing progress, and added a bundle/queue-route deployment guard.
- [x] Typesense 30.2 rejected fresh collections because optional `published_at_timestamp` was also declared as `default_sorting_field`. Removed the invalid default; API queries continue to sort explicitly by the numeric field.
- [x] Queue worker was polling the retired queue API URL while frontend requests were stored by the active Typesense API, so it reported an empty queue and left `@marimo` frozen. Corrected the worker secret, canonicalized the request to `@marimo-team`, prioritized queue work, and retained failed rows for retry.
- [x] None currently introduced by this change after local and live verification.
- [x] Potential latency regression reviewed against the active Typesense deployment; Meilisearch comparison is retired and no longer deployable.
- [x] Regression found in deployed API: Meilisearch grouped count returned raw hit/document count semantics instead of distinct video count for `total_videos` (`react/@t3dotgg` returned 12635 videos). Fixed with backend distinct-count trait, Meili facet counts, and live Meili `faceting.maxValuesPerFacet=100000`.
- [x] Regression found during the full rewipe: new chunks used handle-form `youtuber_id` values such as `@t3dotgg`, while count/delete operations used canonical `t3dotgg`. Fixed at the ingestion boundary, made Meili operations compatible with both forms, and added backend-to-backend copy normalization.
- [x] Regression found in count maintenance: scripts labeled as count refreshes ran full YouTube discovery through `refresh-youtuber(s)`, making reconciliation slow, source-dependent, and prone to racing active indexing. Fixed by separating backend-only count reconciliation from optional metadata/profile enrichment.
- [x] Regression found in indexer secrets: raw Fly Machines used Launch-style secret deployment and could attempt to create an unrelated machine; the YouTube fallback key also could not be mounted as a raw file secret. Fixed with staged base64 file secrets, in-process loading, and explicit worker rotation.
- [x] Historical Meilisearch destructive-rewipe regression closed by retiring and destroying that backend. Active updates are non-destructive; rewipe remains explicit and fail-closed.
- [x] Regression found in channel directory: both search clients returned only their first 100 creator documents. Fixed with backend pagination, so the frontend can render every searchable indexed channel.
- [x] Regression found in queue operations: the runner allowed a daemon with both target backends. Fixed by requiring exactly one backend per worker and adding a two-worker launcher/check.
- [x] Regression found in Typesense ordering: low-signal newest queries used `t_start_ms` because the schema had no publication sort field. Fixed in code with `published_at_timestamp`; an intentional Typesense rewipe is required to backfill deployed chunks.
- [x] Regression found in queue diagnostics: the intended on-demand worker was reported as missing while stopped, and the launcher could create a second Machine against the JSONL volume. The invariant now accepts one healthy stopped worker, rejects duplicates, and starts the existing Machine instead of creating another.
- [x] Regression found in conference imports: the script loaded self-hosted Typesense variables but did not map them to the generic names required by `youtube_cli`, so every item failed before discovery. The importer now fails early, exports the correct aliases, and has a fixture test.
- [x] Regression found in conference imports: `index-video` used its destructive default even though the playlist runner is resumable. The runner now always passes `--no-wipe`, and its fixture fails if that guard is removed.
- [x] Regression found in creator discovery: an unavailable or empty uploads tab aborted before the streams tab was considered, so stream-only creators could not validate or index. Validation and discovery now fall back independently across both tabs while retaining `yt-dlp` as primary.
- [x] Regression found in conference imports: a valid video ID beginning with `-` was parsed as a CLI option. The runner now places video IDs after an explicit `--` argument boundary.
- [x] Audit the three unattached legacy indexer volumes
  (`vol_v8e6wk3g1l6kj15v`, `vol_458zq611m0yppz14`, and
  `vol_4qlzgynglnz786gr`) and preserve their useful data. The active Raw cache
  now contains all historical transcripts; exact deletion remains part of the
  final seven-volume retirement gate.

## Product/UI Behavior

- [x] Video cards show three moments initially and reveal additional returned moments incrementally; useful/not-useful feedback is attached to each moment instead of the whole video.
- [x] Hide “Show all moments” when the result has no fourth moment, including
  the exact three-hit boundary case.
- [x] Verify browser Back and toolbar Back clear the submitted result surface after a channel-scoped search on the deployed frontend. Playwright verified both paths leave zero result cards and an empty query.
- [x] Verify the global "Currently indexing @handle" toast appears for five seconds while a queue worker is actively adding that channel's videos. Browser-verified on production with the `@marimo-team` name and avatar.
- [x] Results header shows total matching moments across the full result set, not just moments returned on the current page.
- [x] PostHog captures search loads, video opens, timestamp selections, moment expansion, and per-moment useful/not-useful feedback with video/creator/content/rank/moment metadata; Axiom captures backend search timing and result-volume fields for offline relevance analysis.
- [x] Results header shows total matching videos across the full result set, not just videos returned on the current page.
- [x] Results header shows time to first result.
- [x] Videos and Shorts are displayed as separate result categories.
- [x] Category counts are clearly labeled as either current-page counts or true full-result totals. Prefer true full-result totals if the API can provide them without excessive backend cost.
- [x] Search URL/query parameters can fully represent a search state so the frontend can be used like a search engine.
  - [x] Query text is represented in the URL.
  - [x] Creator filter is represented in the URL.
  - [x] Sort is represented in the URL.
  - [x] Page is represented in the URL.
  - [x] Loading a URL restores the same search state and executes the search.
- [x] Add debounced search prewarming if it improves perceived latency without adding noisy backend load.
- [x] Make the home search responsive without fixed form widths, retain its shared-element transition, remove perpetual avatar animation, and limit cursor-magnet DOM writes to nearby creators.
- [x] Rename frontend component files and imports to kebab-case.
- [x] Make the Ko-fi destination configurable through `VITE_KOFI_URL` with a production fallback.
- [x] Remove the homepage navbar, title, magnetic interaction hint, and self-hosting footer copy; keep only the configurable Ko-fi link at the bottom.
- [ ] Measure current request volume impact after deployment. Rechecked 2026-07-18: both rotated Axiom keys authenticate and list datasets, but both still return HTTP 403 for `query:read`; grant that action to the query key, then run `scripts/axiom-agent.sh report`.
  - [x] Cancel stale prewarm requests.
  - [x] Do not prewarm index-request mode queries that start with `@`.
- [x] Retire the semantic search experiment without changing production Keyword search. The experiment failed the general-relevance and UI-isolation gates documented in `docs/semantic-experiment-2026-07-13.md`; its Pages preview was replaced with a Keyword-only build and its Fly resources were destroyed. (WE PLAN TO EXPERIMENT MORE IN THE FUTURE)
  - [x] Add an explicit `lexical|semantic` request/response contract across frontend, API, shared search crate, CLI adapter, and Typesense client; lexical remains the default and URL/cache state includes the mode.
  - [x] Add a non-default Semantic dropdown option with a build-time capability gate so an older or unconfigured deployment cannot silently return lexical results under a semantic label.
  - [x] Add Typesense hybrid query construction with configurable embedding field, `alpha: 0.7`, caching, lexical filtering, and full-reranking disabled by default to bound query cost.
  - [x] Fail closed unless both `YTSEARCH_SEMANTIC_SEARCH_ENABLED=true` and `VITE_SEMANTIC_SEARCH_ENABLED=true`; backend tests confirm lexical queries never send vector parameters.
  - [ ] Before reconsidering semantic search, select the embedding model and dimensions using a judged offline relevance set. Initial candidates are Nomic v1.5 at 128/256/768 dimensions and BGE Small at 384 dimensions.
  - [x] Create and seed an isolated Typesense vector collection, measure its narrow-fixture latency and recall, then reject promotion after broader manual queries failed relevance gates.
    - [x] Provision isolated Typesense/API Fly Machines and encrypted volume with separate credentials and no queue access.
    - [x] Create a Typesense 30.2 auto-embedding schema using `ts/all-MiniLM-L12-v2`; import 250 fixed real chunks with 250/250 successes in four seconds.
    - [x] Verify hybrid recall and latency on the narrow security fixture: paraphrase recall expanded from 2 to 18 videos; semantic p95 was 185 ms versus lexical 165 ms over 20 runs; Typesense resident memory was 225 MB.
    - [x] Deploy and browser-test an immutable semantic Pages preview with Playwright.
    - [ ] A future experiment must expand to a versioned judged corpus and calculate MRR/Recall@10 before exposing a UI; the narrow security fixture proved integration but hid serious general-relevance failures.
  - [x] Do not backfill production or switch an alias. Production remained Keyword-only throughout the rejected experiment.

## API/Search Behavior

- [x] Benchmark the public search response before changing wire formats: JSON was approximately 116 KB for `react/@t3dotgg`, with 20-run end-to-end latency dominated by API/search/network work rather than serialization.
- [x] Keep JSON as the browser API format and enable transparent gzip compression at the API boundary; protobuf was re-evaluated on 2026-07-14 and remains unjustified until a bulk internal pipeline demonstrates at least a 20% end-to-end improvement.
- [x] API response exposes full-result moment totals from the backend.
- [x] API response exposes full-result grouped video totals, or a defensible approximation with clear naming.
- [x] Replace the five-moment truncation with a lazy video-scoped fetch. Production verification expands 689 matching chunks into all 60 distinct timestamp moments in about 724 ms without bloating the initial search response; the API regression fixture retains all 12 collected moments.
- [ ] API response exposes full-result video/short totals. Blocked on adding and backfilling `content_kind` across the existing Typesense collection; filtering the pre-migration schema correctly returns HTTP 400, so the incompatible change was rolled back.
- [x] Low-signal query behavior remains best-match/newest-for-filter, not strict all-terms matching.
- [x] Stopword-heavy query behavior continues to prefer stronger title/transcript matches.
- [x] `q=fable on high&youtuber=@t3dotgg` ranks `A proper guide to Fable 5` near the top on Meilisearch.
- [x] `q=I&youtuber=@t3dotgg` returns newest Theo videos on Meilisearch.
- [x] Compare equivalent Meilisearch and Typesense queries for result completeness and relevance.
- [x] Investigate why Typesense appears faster now and why it has missing indexed videos versus Meilisearch.
- [x] Investigate Meilisearch slowdown with current staging data.
  - [x] Check Fly machine CPU/RAM/disk state.
  - [x] Check Meilisearch task/update backlog.
  - [x] Check index size and disk pressure.
  - [x] Benchmark equivalent queries against both backends.
  - [x] Record whether slowdown is API processing, engine search, cold start, write pressure, or network latency.

## Indexing/Queue Behavior

- [x] Publish validated channel name/avatar metadata before the long transcript pass so the live indexing toast can show a real profile picture for newly requested channels.
- [x] A permanently invalid/unavailable channel cannot freeze the one-item queue worker; failed rows are retained for inspection and skipped until explicitly re-requested.
- [x] Queue/index request API includes indexed/public video count context on the frontend after update.
- [x] Queue accepts all requested creators and special video/conference requests without duplicating existing pending work.
- [x] Queue daemon targets the single active Typesense backend; the retired dual-backend launcher cannot start a Meilisearch writer.
- [x] Queue failures log clearly and do not permanently block later requests.
- [x] Daily update is enabled for the active Typesense backend with conservative cloud concurrency.
- [x] Count refresh runs after indexing so frontend channel cards show correct indexed/public counts. Live `@marimo-team` reached `151 / 151` after the resumed queue run.
- [x] Confirm Typesense missing videos are repaired by update mode where possible. Live `@vsauce` is now `621 / 621` videos.
- [x] Confirm update mode still preserves existing chunks. `@marimo-team` resumed across a worker replacement and advanced from 14 to 151 indexed videos without deleting prior chunks.
- [x] Confirm rewipe mode still deletes and rebuilds a creator when intentionally used. Isolated Typesense 30.2 test deleted 54 existing chunks and rebuilt the same 54 chunks for a capped `@marimo-team` run.

## Requested Creators And Targets

- [x] `@DwarkeshPatel`
- [x] `@cursor_ai`
- [x] `@lexfridman`
- [x] `@RyanLPeterman`
- [x] `@hankschannel`
- [x] `@rossmanngroup`
- [x] `@pragmaticengineer`
- [x] `@BetterSoftwareConference`
- [x] `@MollyRocket`
- [x] `@aiDotEngineer`
- [x] `https://www.youtube.com/watch?v=1fZTOjd_bOQ`
- [x] `@LowLevelTV`
- [x] `@bawad` with a 50-video cap if the tooling supports per-request limits. Queued handle with note; current queue does not support per-request video cap.
- [x] `@Fireship`
- [x] `@michaelreeves`
- [x] `@DougDoug`
- [x] `@codebullet`
- [x] Veritasium target verified and queued/indexed. `@veritasium` was already queued.
- [x] Vsauce target verified and queued/indexed. `@vsauce` queued for repair.
- [x] NVIDIA GTC 2026 official playlist imported as far as publicly possible: 29/32 videos indexed; two have no English captions and one is private.
- [x] Google I/O 2026 All Sessions official playlist imported resumably: 68/68 videos.
- [x] Apple WWDC26 Sessions official playlist imported resumably: 107/107 videos.

## Live Frontend Verification

- [x] `https://search.pronsh.dev` serves the Cloudflare Worker frontend and proxies the active Typesense Fly API.
- [x] `https://search.experimental.pronsh.dev` serves the Raw frontend and the
  deployed API exposes all 61 creators from the full snapshot.
- [x] Production shows total moments/videos, time to first result, and separated Videos/Shorts.
- [x] Frontend indexed/public counts update correctly after queue/indexing runs. Browser/API verified with `@marimo-team`.
- [x] Frontend shows all the channels indexed and not only a subset. API pagination and production frontend currently expose all 41 searchable channels; `@` suggestions use that complete list.
- [x] Search URL query parameters work on deployed frontends.
- [x] PostHog integration browser-verified on production: configuration and analytics assets load successfully; relevance feedback is now captured per transcript moment.

## Required Local Verification

- [x] `cargo fmt --check`
- [x] `cargo test -p youtube_api -p youtube_meilisearch -p youtube_typesense -p youtube_search`
- [x] `bun run build`
- [x] `bun run frontend:check`
- [x] `sh -n scripts/cloud-index-queue-entrypoint.sh scripts/fly-run-queue-indexer.sh`
- [x] `scripts/check-frontend-backends.sh`
- [x] Deployment guard follows Vite lazy chunks and checks both production and
  experimental same-origin API paths.
- [x] `scripts/check-staging-machines.sh`
- [x] `scripts/check-indexed-channel-status.sh @vsauce @thePrimagen @t3dotgg`

## Live Search Quality Checks

- [x] Meilisearch: `q=fable on high&youtuber=@t3dotgg&per_page=10`
- [x] Meilisearch: `q=I&youtuber=@t3dotgg&per_page=10`
- [x] Typesense Fly: equivalent `fable on high` check.
- [x] Typesense Fly: equivalent `I` check.
- [x] Compare Veritasium, Vsauce, and newly requested creator coverage on the active backend. Cross-backend comparison is retired with Meilisearch; Typesense live counts: Vsauce `621/621`, Veritasium `150/517`, marimo `151/151`.

## Deployment/Operations

- [ ] Apply and measure the guarded Typesense display-field memory optimization after the active `@ryansolid` writer finishes; use the measured restart peak before reducing the 8 GB Machine.

### Autoresearch Follow-ups (Non-semantic)

- [x] Benchmark public search with gzip on/off using the frozen query suite. A 20-pair alternating production run on 2026-07-15 returned identical counts and zero errors: gzip reduced average transfer from 90,461 to 17,351 bytes, average latency from 479 to 263 ms, p50 from 338 to 212 ms, and p95 from 980 to 566 ms.
- [ ] Run an isolated Fly API sizing experiment across the lowest viable shared and performance CPU classes. Use the same image and request replay, include warm and cold-start latency, enforce `MAX_COST_USD` and `TTL_MINUTES`, and destroy every candidate app/machine after recording results.
- [ ] Build a fixed cached-transcript indexing fixture so indexer CPU/RAM and `JOBS` settings can be compared without YouTube/network variance or backend imports. Measure successful videos/minute, peak RSS, failures, and estimated cost per 1,000 videos.
- [ ] Compare always-on, stop/autostart, and suspend/autostart for the public API while keeping one warm production-equivalent control. Reject any configuration that introduces frontend-visible 5xx responses or exceeds the agreed cold-start p95; never apply proxy autostop to the background indexer.
- [x] Add a repeatable frontend budget (`bun run perf:frontend`) covering gzip bundle bytes, lab FCP, search-to-first-result rendering, and complete channel-directory rendering.
  - [ ] Capture true LCP, INP, and CLS once Chrome DevTools MCP is available; the current Playwright check deliberately does not label lab timings as field Core Web Vitals.
  - [ ] Capture and inspect Chrome flamecharts once the `chrome-devtools` MCP server is configured (`npx -y chrome-devtools-mcp@latest`). The required trace tool was unavailable on 2026-07-15.
  - [x] Route browser search and moment requests through same-origin `/api` on the Cloudflare Worker instead of opening a separate direct Fly connection.
- [x] Add automatic autoresearch result comparison: `scripts/compare-autoresearch-search.sh` requires at least three runs, checks confidence-adjusted means and p95, rejects HTTP errors/unstable counts/corpus drift, and emits keep/discard/invalid without modifying production or the working tree. Fixture tests cover all three decisions.

- [x] Add token-efficient agent diagnostics: one-line JSON staging snapshot, bounded/redacted indexer log summary, compact machine invariant output, and explicit verbose modes.
- [x] Document immediate and experimental cost reductions, performance gates, combined-server trade-offs, and the protobuf decision in `docs/cost-optimization.md`.
- [x] Increase daily Fly snapshot retention from 5 to 30 days for the Typesense corpus and persistent queue, schedule fresh snapshots, and add checksummed provider-neutral backup/restore scripts plus `docs/disaster-recovery.md`.
- [ ] Automate provider-neutral exports from an isolated snapshot restore. Never export the full collection from serving Typesense: the 2026-07-15 trial raised a health-check search from 99 ms to 5.7 seconds and was aborted; the script now fails closed for the production URL.
- [ ] Enable R2 once in the Cloudflare dashboard, create a private `ytsearch-backups` bucket, and automate daily upload/retention of the checksummed exports. Rechecked 2026-07-18: Wrangler still returns Cloudflare API code `10042` because R2 is disabled.
- [x] Reduce the always-on indexer from 4 to 2 performance CPUs while retaining 8 GB RAM, swap, and single-writer backpressure (Fly shared CPU is capped at the previously failing 4 GB memory tier).
- [x] Finish the portable self-hosting guide: added provider-neutral Compose services for Typesense, API, frontend, and one persistent queue worker plus bare-process guidance.
- [x] Remove legacy Meilisearch crate, Rust branches, Fly configuration, scripts, and non-historical documentation; retain only the README pointer and `docs/legacy/meilisearch-retirement.md`.
- [x] Complete the environment-key audit across local `.env`, Fly apps, Pages, and scripts. No legacy Meili/Typesense Cloud keys remain locally or on active Fly apps; all remaining API/indexer secret names map to active code paths.
- [x] Deploy backend response changes to the active `ytsearch-api-typesense-fly-staging` API.
- [x] Deploy frontend changes to the `ytsearch-search` Worker and smoke-test `search.pronsh.dev`.
- [x] Before replacing queue daemon, confirm existing `ytsearch-indexer-staging` queue daemon state and logs.
- [x] After replacing queue daemon, confirm only one persistent daemon is processing the JSONL queue. Existing daemon was left running; no replacement daemon started.
- [x] Record final machine state, frontend URLs, and indexing status.
