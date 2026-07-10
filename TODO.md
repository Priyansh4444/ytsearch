# ytsearch Requested Behavior TODO

This file tracks the current requested work and end-to-end verification. Mark any discovered or introduced regression in the Regression Log before changing behavior further.

## Regression Log

- [ ] None currently introduced by this TODO-only change.
- [ ] Potential latency regression: adding true full-result video totals currently requires an extra grouped backend query per search. Benchmark Meilisearch and Typesense before deploying.
- [x] Regression found in deployed API: Meilisearch grouped count returned raw hit/document count semantics instead of distinct video count for `total_videos` (`react/@t3dotgg` returned 12635 videos). Fixed with backend distinct-count trait, Meili facet counts, and live Meili `faceting.maxValuesPerFacet=100000`.
- [x] Regression found during the full rewipe: new chunks used handle-form `youtuber_id` values such as `@t3dotgg`, while count/delete operations used canonical `t3dotgg`. Fixed at the ingestion boundary, made Meili operations compatible with both forms, and added backend-to-backend copy normalization.
- [x] Regression found in count maintenance: scripts labeled as count refreshes ran full YouTube discovery through `refresh-youtuber(s)`, making reconciliation slow, source-dependent, and prone to racing active indexing. Fixed by separating backend-only count reconciliation from optional metadata/profile enrichment.
- [ ] Regression found in destructive rewipe: Meili deleted a creator's existing chunks before confirming replacement captions were available, then a YouTube rate limit left several creators partially or fully empty. Restore them from Typesense and redesign rewipe to stage or preflight replacement data before deletion.
- [x] Regression found in channel directory: both search clients returned only their first 100 creator documents. Fixed with backend pagination, so the frontend can render every searchable indexed channel.
- [x] Regression found in queue operations: the runner allowed a daemon with both target backends. Fixed by requiring exactly one backend per worker and adding a two-worker launcher/check.
- [x] Regression found in Typesense ordering: low-signal newest queries used `t_start_ms` because the schema had no publication sort field. Fixed in code with `published_at_timestamp`; an intentional Typesense rewipe is required to backfill deployed chunks.

## Product/UI Behavior

- [x] Results header shows total matching moments across the full result set, not just moments returned on the current page.
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
- [ ] Measure current request volume impact after deployment.
  - [x] Cancel stale prewarm requests.
  - [x] Do not prewarm index-request mode queries that start with `@`.

## API/Search Behavior

- [x] API response exposes full-result moment totals from the backend.
- [x] API response exposes full-result grouped video totals, or a defensible approximation with clear naming.
- [ ] API response exposes full-result video/short totals if category counts need to be accurate globally.
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

- [ ] Queue/index request API includes indexed/public video count context on the frontend after update.
- [x] Queue accepts all requested creators and special video/conference requests without duplicating existing pending work.
- [x] Queue daemon processes both active backends: `meilisearch` and `typesense-fly`.
- [x] Queue failures log clearly and do not permanently block later requests.
- [x] Daily update is enabled for both active backends with conservative cloud concurrency.
- [ ] Count refresh runs after indexing so frontend channel cards show correct indexed/public counts.
- [ ] Confirm Typesense missing videos are repaired by update mode where possible. Current gap confirmed: `@vsauce` is 617 Meili videos vs 117 Typesense videos before queued repair.
- [ ] Confirm update mode still preserves existing chunks.
- [ ] Confirm rewipe mode still deletes and rebuilds a creator when intentionally used.

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
- [ ] NVIDIA conference videos researched, scoped, and queued/indexed without adding unrelated NVIDIA uploads. Researched likely source channels; current queue cannot express scoped playlist/video-set indexing.
- [ ] Google I/O videos researched, scoped, and queued/indexed without adding unrelated Google uploads. Researched Google I/O playlist sources; current queue cannot express scoped playlist/video-set indexing.
- [ ] Apple WWDC videos researched, scoped, and queued/indexed without adding unrelated Apple uploads. Researched Apple/Apple Developer sources; current queue cannot express scoped playlist/video-set indexing.


## Live Frontend Verification

- [x] `https://head.ytsearch-755.pages.dev` points at the intended Meilisearch API.
- [x] `https://meili.ytsearch-755.pages.dev` points at the intended Meilisearch API.
- [x] `https://typesense.ytsearch-755.pages.dev` points at the intended Typesense Fly API.
- [x] Head frontend shows total moments/videos, time to first result, and separated Videos/Shorts.
- [x] Meili frontend shows total moments/videos, time to first result, and separated Videos/Shorts.
- [x] Typesense frontend does not break with the updated response/UI contract.
- [ ] Frontend indexed/public counts update correctly after queue/indexing runs.
- [ ] Frontend shows all the channels indexed and not only a subset. And you can filter them using the @ keyword (if none you can index them)
- [x] Search URL query parameters work on deployed frontends.

## Required Local Verification

- [x] `cargo fmt --check`
- [x] `cargo test -p youtube_api -p youtube_meilisearch -p youtube_typesense -p youtube_search`
- [x] `bun run build`
- [x] `bun run frontend:check`
- [x] `sh -n scripts/cloud-index-queue-entrypoint.sh scripts/fly-run-queue-indexer.sh`
- [x] `scripts/check-frontend-backends.sh`
- [x] `scripts/check-staging-machines.sh`
- [x] `scripts/check-indexed-channel-status.sh @vsauce @thePrimagen @t3dotgg`

## Live Search Quality Checks

- [x] Meilisearch: `q=fable on high&youtuber=@t3dotgg&per_page=10`
- [x] Meilisearch: `q=I&youtuber=@t3dotgg&per_page=10`
- [x] Typesense Fly: equivalent `fable on high` check.
- [x] Typesense Fly: equivalent `I` check.
- [ ] Compare Veritasium, Vsauce, and newly requested creator coverage on both backends.

## Deployment/Operations

- [x] Deploy API changes to `ytsearch-api-meili-staging` if backend response changes.
- [x] Deploy API changes to `ytsearch-api-typesense-fly-staging` if backend response changes.
- [x] Deploy frontend variants with `scripts/cloudflare-pages-deploy-backends.sh` after frontend changes.
- [x] Before replacing queue daemon, confirm existing `ytsearch-indexer-staging` queue daemon state and logs.
- [x] After replacing queue daemon, confirm only one persistent daemon is processing the JSONL queue. Existing daemon was left running; no replacement daemon started.
- [x] Record final machine state, frontend URLs, and indexing status.
