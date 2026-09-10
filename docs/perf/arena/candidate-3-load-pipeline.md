# Arena candidate 3 — load pipeline: FCP / LCP / CLS / INP

Direction: loading + Core Web Vitals pipeline. Cold load to FCP, search-to-results (LCP on first video poster), expand-moments (CLS/INP), and the JS task sizes that gate each one.

Grounding read: `index.html`, `src/index.tsx`, `src/index.css`, `vite.config.ts`, `src/app.tsx`, `src/api.ts`, `src/utils.ts`, `src/search-resource.ts`, `src/app-lifecycle.ts`, `src/channel-query.ts`, `src/client-runtime-loader.ts`, `src/swarm-layout.ts`, `src/search-route.ts`, `src/components/search-form.tsx`, `src/components/search-results.tsx`, `src/components/video-result.tsx`, `src/components/channel-picker.tsx`, `src/components/channel-profile.tsx`, `src/components/highlighted-text.tsx`, `src/components/dashboard.tsx`, `scripts/check-frontend-performance.ts`, `scripts/check-bundle-performance.ts`, `playwright.config.ts`, `wrangler.jsonc`, plus `dist/index.html` and `dist/assets` after a fresh `bun run build`.

> No source files were edited. All changes below are **proposed diffs as fenced code blocks**. The only file written is this one.

## 1. Closure / arrow inventory — load + render path, load lens

Bar: every render-hot closure/arrow gets `file:line — required / removable — one-line load-order or task-size reason`. Required = behavior must stay; removable = allocation or per-render work can go without changing behavior. Lines against this checkout.

### Entry: HTML to JS bootstrap (FCP-critical chain)

| Closure | Verdict | Lens |
|---|---|---|
| `src/index.tsx:17` `lazy(dashboard import)` + `:18` `.then` adapter | **required** (defer) | Route-gated split keeps dashboard (~17 KB raw / 6.1 KB gzip) out of the initial graph. |
| `src/index.tsx:23` dashboard-branch render fn / `:37` `() => <App />` | **required** | Each evaluated once on its route only. |
| `src/app.tsx:36-40` `lazy(SearchResults)` + `:37` adapter | **required** (defer) | Keeps ~13 KB raw / 4.7 KB gzip result code out of FCP. |
| `src/app.tsx:41-45` `lazy(ChannelPicker)` + `:43` adapter | **required** (defer) | Swarm code (~3.4 KB) deferred past first paint. |
| `src/app.tsx:46-60` `lazy(IndexRequestDialog / RecentSearches / IndexRequestState)` + adapters `:48,:53,:58` | **required** (defer) | 1.4–2.1 KB chunks each, correctly out of the initial graph. |
| `src/app.tsx:62` `prepareResultsRuntime` / `:63` `prepareChannelPickerRuntime` / `:64` `prepareIndexDialogRuntime` | **required** | Explicit warm hooks: submit-parallel, fetch-parallel (`:74`), and index-mode-only (`:253`). Solid dedupes preload — no double fetch. |
| `src/client-runtime-loader.ts:4` loader `.catch` reset | **required** | One-time failure reset; not hot. |
| `src/api.ts:35` `prepareSearchRuntime` `.then` / `:52-53` search `.then` | **required** | Defers `client-runtime` (~5.2 KB / 2.0 KB gzip) until first search. Good for FCP. |
| `src/api.ts:55` inflight `.finally` delete | **required** | Map hygiene, once per request. |
| `src/api.ts:74` `finish` / `:85` `abort` / `:93-95` handlers / `:97` `{...result, request_duration_ms}` | **required** | Per-search microtask chain; single spread alloc per search is unavoidable (must not mutate shared response). |
| `src/api.ts:112,127,137` `loadClientRuntime().then` (moments / index / dashboard) | **required** | All deferred past FCP by construction. |
| `src/api.ts:119` `indexedChannels` `.finally` | **required** | Dedupes bootstrap fetch. |

### App shell memos and signals (what gates first DOM vs recomputes strings)

| Closure | Verdict | Lens |
|---|---|---|
| `src/app.tsx:73-78` `refreshChannels` async + destructure | **required** | Post-FCP fetch; `Promise.all([indexedChannels, preload])` overlaps code+data. |
| `src/app.tsx:95` `executableSearch` memo | **required — gates DOM** | Single boolean gate for the results subtree. Keep. |
| `src/app.tsx:100-102` `selectedChannel` memo + inner `find` | **required semantically / removable work** | Gates `ChannelIdentity`, but `normalizeHandle(channel.handle)` reruns per channel per `youtuber()` change — precompute per list (D7 pattern). |
| `src/app.tsx:104-108` `indexingChannel` memo + inner `find` | **required, off-critical-path** | Toast-only; must not enter FCP chain — it doesn't. |
| `src/app.tsx:110` `hasChannel` / `111` `hasSearchSurface` / `112-113` `showIndexedChannels` | **required — gate DOM** | Tiny booleans deciding home-vs-results layout. Keep inline. |
| `src/app.tsx:114-116` `directChannel` memo + `find` | **required semantically / removable from keystroke path** | Exact-handle match reruns on every keystroke O(n) — keep behavior, debounce path (D3). |
| `src/app.tsx:118` `channelSuggestions` memo | **required semantically / INP hotspot** | Full O(n) filter + 3x `toLowerCase` per channel per keystroke (~0.044 ms @400 ch, §2d — linear in catalog). Debounce + index (D3/D7). |
| `src/app.tsx:119` `mentionedChannel` memo | **required semantically / redundant pass** | `find` then a second full `channelSuggestions` filter (`channel-query.ts:30`). Fuse (D7). |
| `src/app.tsx:120` `mentionQuery` / `121` `isIndexMode` / `122-125` `formMode` | **required** | Pure cheap string ops; `formMode` gates submit branch. |
| `src/app.tsx:126-130` `searchError` memo / `131-141` `successfulSearch` memo | **required — gate DOM** | Error and results subtree gates. Keep. |
| `src/app.tsx:142-149` `indexDialogState` / `indexingNotice` memos | **required** | Dialog/toast gates, idle-only. |
| `src/app.tsx:151-152` `syncRoute` + transition closure | **required** | popstate path, 8 setters batched in one transition. |
| `src/app.tsx:176-204` `selectChannel` + `:179` transition + `:192` `.then` + `:203` `.catch` | **required** | 7 setters + pushState in one transition; `.then` settles later, no INP block. |
| `src/app.tsx:206-223` `confirmIndexRequest` + `:219` error mapper | **required** | Dialog path, not load-critical. |
| `src/app.tsx:225-285` `runSearch` — largest INP task | **required semantically / must be broken up (D4)** | One sync task: trim, `directChannel()` O(n), `mentionedChannel()` O(2n), request allocs, analytics, 4-5 setters, `writeSearchUrl`, preload + fetch kickoff. Keep behavior; shrink task. |
| `src/app.tsx:287-295` `goToPage` (`:290` spread) | **required** | One small alloc per pagination; immutability for resource key. |
| `src/app.tsx:297-310` `goBack` + `:298` closure | **required** | 8 setters, one transition. Keep. |
| `src/app.tsx:313-321` `copySearchLink` async + `:320` timeout | **required (deferred)** | Dynamic-import clipboard runtime. |
| `src/app.tsx:323-344` idle-warm effect + `:336` 900 ms timer + `:340` catch + `:342` cleanup | **required — well-deferred** | Zero speculative network (code preload only). Exemplary; keep. |
| `src/app.tsx:346-359` analytics effect + `:356` saveRecentSearch updater | **required** | Once per success key. |
| `src/app.tsx:361` `scrollToLinkedResult` / `366-381` scroll effect + `:372` `.then(rAF)` + `:376` cleanup | **required / TBT note** | Chain costs one microtask + one frame per success — guard hashless case (D9). |
| `src/app.tsx:383-384` hashchange add/remove | **required** | Trivial. |
| `src/app.tsx:386-399` `selectRecentSearch` + `:395` spread | **required** | One alloc per click; unavoidable. |
| Render mappers `app.tsx:458,476,482,507,523` | **required** | `Show keyed` fns, once per state change. |
| `app.tsx:416` copy onclick / `440` `onSubmit={() => runSearch(1)}` / `450` onClear / `513,517` dialog callbacks | **removable allocs** | Per-render wrappers; hoist to stable consts (D4). |

### Search form (per-keystroke INP path)

| Closure | Verdict | Lens |
|---|---|---|
| `search-form.tsx:23` `visibleSuggestions` slice(0,8) | **required / tiny churn** | 8-item alloc per read; memoize so ArrowUp/Down doesn't re-slice (D3). |
| `search-form.tsx:24-27` `suggestionsVisible` | **required — gates DOM** | Listbox gate; the CLS culprit to reserve space for, not delete (D8). |
| `search-form.tsx:29-35` query-mode reset effect | **required** | Two batched setters on mode flip. |
| `search-form.tsx:37-56` `handleSearchKeyDown` + `:41,:44` updaters | **required** | Keyboard path; Solid-idiomatic updaters. |
| `search-form.tsx:62-65` form onSubmit | **required** | preventDefault + delegate. |
| `search-form.tsx:77-82` input onInput (`onQueryInput` + `flush()`) | **required semantically / INP hotspot by design** | `flush()` forces sync recompute of suggestion memos inside the input task (correctness: `search-form.tsx:79-81`). Keep; move cost downstream (D3). |
| `search-form.tsx:107` sort onChange / `111-112` sort For+mapper | **required** | Static 2-3 options. |
| `search-form.tsx:148-170` suggestion For mapper | **required** | Max 8 rows per filter change. |
| `search-form.tsx:158` per-row `onPointerEnter` | **removable** | Hover-only; dead weight on touch — gate behind `(pointer:fine)` (D3). |
| `search-form.tsx:159` per-row `onClick` | **required** | 8 small closures; delegation alternative noted in D3, either acceptable. |

### Results (search-to-results + LCP)

| Closure | Verdict | Lens |
|---|---|---|
| `search-results.tsx:39-44` `[...sectionVideos, ...unknownVideos]` | **removable — fuse (D5)** | Two O(n) filters + two spread allocs per render, plus a third filter for shorts (`:48`). One partition pass, zero spreads. |
| `search-results.tsx:76` `(video, index) => <VideoResult rank>` | **required** | `For` row mapper; `index()` read cheap. |
| `search-results.tsx:83-87` `sectionVideos` / `unknownVideos` | **removable — fuse (D5)** | Same fix. |
| `video-result.tsx:21` `visibleHits` slice(0,3) / `:22` `hasHiddenMoments` | **required** | Bounds initial DOM per card (good for LCP/TBT). |
| `video-result.tsx:25-28` `selectedHit` find | **required** | O(<=3); gates poster + badge. |
| `video-result.tsx:29` `playerUrl` → `256-260` `new URL()` + set + toString | **removable work — lazy-gate (D6)** | URL parse+serialize per memo read even when the iframe is never mounted. Compute only when `playerActive()`. |
| `video-result.tsx:30-34` `posterThumbnail` regex replace | **removable work — memoize (D6)** | Regex + rebuild per read per card; URL immutable per video. |
| `video-result.tsx:36-40` `activatePlayer` | **required** | One setter + analytics. |
| `video-result.tsx:41-66` `expandMoments` async + `:51` find | **required / chunk-insert candidate (D4)** | `setAllHits(full)` commits dozens of rows in one task — yield across batches. |
| `video-result.tsx:67-78` `copyResultLink` async | **required (deferred)** | Dynamic clipboard import. |
| `video-result.tsx:98` thumbnail Show mapper / `:226` error mapper / `:162` summary mapper | **required** | Once-per-state mappers. |
| `video-result.tsx:146,176` capture onclick closures | **removable allocs** | Identical bodies; hoist one `handleOpen` per card. |
| `video-result.tsx:168` copy onclick | **removable alloc** | Hoist. |
| `video-result.tsx:184-202` hits For + `:192-199` per-hit onSelect | **required semantically / removable per-hit alloc** | <=3 closures per card today, N after expand — delegate via `data-hit-id` (D4). |
| `video-result.tsx:206` expand onclick | **required** | Button delegate. |
| `video-result.tsx:244-254` `engagementSummary` filter+map + `compactNumber.format` | **removable per-render work** | ICU format + 2 allocs per card per render on immutable inputs — one memo per card (D6). |
| `video-result.tsx:271-280` `submitFeedback` / `:350` `useful` / `:354` feedback onclick | **required / removable alloc** | Hoist click (value already a prop). |
| `video-result.tsx:338-343` `resultProperties` spread | **required / hoistable** | Per analytics event only, not per render. |
| `highlighted-text.tsx:10` `parts` memo + `:15` For + `:16` Show | **required — gates DOM** | Memo correctly prevents re-tokenize; inside cost (regex + textarea `innerHTML` per part, `utils.ts:76-107`) is bounded by the 3-hit cap — keep cap. |
| `utils.ts:48-57` `formatDate` (`new Date` + `toLocaleDateString`) | **removable per-render work** | ICU date per card per render on immutable string — memoize (D6). |
| `utils.ts:26-46` `withViewTransition` + `:27` `updateAndFlush` | **required / TBT note** | Inner `flush()` is load-bearing (Solid 2 batching, `:29-31`). Shrink the update (D4), never the flush. Per-call `matchMedia(:33)` cacheable (D9). |

### Swarm and avatars (home LCP + pointer-task path)

| Closure | Verdict | Lens |
|---|---|---|
| `channel-picker.tsx:22` `swarm` memo + `129-138` `positionChannels` (`:131` map, `swarm-layout.ts:15` `Array.from`) | **required — one-shot** | O(n) trig per channel-list change, never per frame. Keep. |
| `channel-picker.tsx:28-59` `moveSwarm` + `:31` rAF closure | **required / already batched** | Pointermove coalesced to one rAF; rect read once, writes only for pulled items. Add D9 early-outs. |
| `swarm-layout.ts:32-37` `magneticOffset` (`Math.hypot`) | **required semantically / removable math** | sqrt per item per active frame — squared-distance early-out first (D9). |
| `channel-picker.tsx:53-56` reset loop / `:61` `releaseSwarm` / `:140-144` `resetMagnet` | **required** | Touches previously-active elements only. |
| `channel-picker.tsx:67-83` `onSettled` + `:79` cleanup (`{ passive: true }`, pointer-fine + reduced-motion guards) | **required** | Correct shape. |
| `channel-picker.tsx:85-90` visible effect / `:92-94` rAF cleanup / `:99` stage ref | **required** | Trivial. |
| `channel-picker.tsx:104-123` For mapper + `:107` per-item ref (`Map.set`) + `:113-117` per-item onclick | **required** | `Map.set` per mount is the correct trade (avoids DOM re-query per frame). |
| `channel-profile.tsx:6-9` `indexedCountLabel` | **required** | Max + template per row; cheap. |
| `channel-profile.tsx:17` `sizeClass` / `:21` `outlineClass` / `:22-23` `transitionStyle` / `:24-25` `initials` | **required** | Negligible string branches. |
| `channel-profile.tsx:26-30` `avatarUrl` + `:81-88` `imageVersion` FNV loop | **required semantically / removable per-read work** | FNV over URL per read per avatar (N avatars on home) — cache per channel (D6). |
| `channel-profile.tsx:45` avatar Show mapper / `:55` onError | **required** | Fallback path. |
| `dashboard.tsx:13-186` whole Dashboard (+ memos `:15,:21,:22`, settled `:30`, mappers) | **required / correctly deferred** | Never in initial route; no load action. |

### Lifecycle and data (post-FCP by construction)

| Closure | Verdict | Lens |
|---|---|---|
| `search-resource.ts:17-43` effect + `:29,:32` handlers + `:39` cleanup-abort | **required** | Dedupes inflight; abort prevents stale inserts. |
| `app-lifecycle.ts:17-90` settled + refresh/check fns + handlers + intervals `:78-79` | **required — deferred** | First calls post-mount; 15 s / 60 s intervals hidden-tab guarded. |
| `channel-query.ts:4-12` `channelSuggestions` filter + `:9` predicate | **INP hotspot — split, not delete (D3/D7)** | See `app.tsx:118`. |
| `channel-query.ts:14-21` `channelMentionQuery` | **required** | Cheap trim+regex per keystroke. |
| `channel-query.ts:23-32` `resolveChannelMention` | **removable second pass (D7)** | find + full re-filter; single pass suffices. |
| `search-route.ts:6` provisional / `:29` `searchStateFromLocation` / `:43` `writeSearchUrl` / `:52-88` anchors | **required** | Parse-once-per-navigation. |

## 2. V8 / build evidence (numbers)

Fresh `bun run build` on this checkout (vite v8.2.2, 74 modules, 462 ms) + independent gzip audit + bundle-budget script.

### 2a. Build output (authoritative)

```text
vite v8.2.2 building client environment for production...
transforming...
74 modules transformed.
dist/index.html                                2.05 kB │ gzip:  0.65 kB
dist/assets/index-DzKb0luR.css                34.67 kB │ gzip:  7.67 kB
dist/assets/recent-searches-D6uJk6gd.js        1.37 kB │ gzip:  0.74 kB
dist/assets/index-request-state-BI9a5l1Q.js    1.54 kB │ gzip:  0.73 kB
dist/assets/index-request-dialog-B6GHgfQp.js   2.09 kB │ gzip:  1.01 kB
dist/assets/api-DXs7_Y6X.js                    2.96 kB │ gzip:  1.43 kB
dist/assets/bootstrap-api-ClzosCug.js          3.21 kB │ gzip:  1.25 kB
dist/assets/channel-picker-D9Uag_db.js         3.36 kB │ gzip:  1.66 kB
dist/assets/client-runtime-DpETn1Yn.js         5.23 kB │ gzip:  1.96 kB
dist/assets/search-results-Cr0RXKKE.js        13.10 kB │ gzip:  4.72 kB
dist/assets/dashboard-BDEFxOxU.js             17.17 kB │ gzip:  6.15 kB
dist/assets/index-BLXuHG_o.js                 32.10 kB │ gzip: 11.53 kB
dist/assets/web-w2PGKLa2.js                   37.12 kB │ gzip: 14.37 kB
built in 462ms
```

Independent `gzipSync` re-measurement (raw to gzip bytes): web 37125 to 14191; entry index 32106 to 11464; dashboard 17172 to 6128; search-results 13104 to 4707; client-runtime 5232 to 1961; channel-picker 3362 to 1662; bootstrap-api 3212 to 1249; api 2960 to 1438; index-request-dialog 2098 to 1017; index-request-state 1545 to 734; recent-searches 1377 to 741; CSS index 34673 to 7631.

### 2b. Largest assets + budgets

| Asset | Raw | Gzip | Load role |
|---|---|---|---|
| `web-w2PGKLa2.js` (Solid runtime shared chunk) | 37.12 KB | 14.19 KB | Initial graph (modulepreload). Largest JS; framework floor, not app code. |
| `index-BLXuHG_o.js` (entry: App + form + shell) | 32.10 KB | 11.46 KB | Initial graph. Holds all §1 shell closures. |
| `dashboard-BDEFxOxU.js` | 17.17 KB | 6.15 KB | Lazy, `/dashboard` only. Correctly excluded from initial. |
| `search-results-Cr0RXKKE.js` | 13.10 KB | 4.71 KB | Lazy, warmed on submit/idle. Correctly excluded. |
| `index-DzKb0luR.css` (Tailwind v4) | 34.67 KB | 7.63 KB | Render-blocking stylesheet; under the 8 KB budget by ~370 bytes — thinnest margin in the pipeline. |

`bun scripts/check-bundle-performance.ts` on this build: `{"ok": true, "initialJavaScriptGzipBytes": 28593, "totalJavaScriptGzipBytes": 45599, "cssGzipBytes": 7679}` vs budgets 30000 / 55000 / 8000 — pass, but initial-JS headroom is 1.4 KB and CSS headroom ~320 bytes (<5% each). Every diff below is budget-neutral or negative (no new deps, no new chunks, HTML attrs only).

`scripts/check-frontend-performance.ts` budgets: JS gzip 55 KB, CSS gzip 8 KB, FCP 1800 ms, search-to-results 2500 ms (3 FCP samples via `performance.getEntriesByType("paint")`, median).

`dist/index.html` head order today: entry `<script type="module">`, three `<link rel="modulepreload">` (web, bootstrap-api, api), then `<link rel="stylesheet">`. Stylesheet-last + late preload discovery = D1. No font, no hero preload, no `fetchpriority` = D1/D2 gaps.

### 2c. Chunking / defer measurement (static, from `dist/`)

- Entry contains **6 dynamic `import()` sites** (five `lazy()` routes + `loadClientRuntime`) — 6 split boundaries survive minification. The budget script's static graph confirms the initial JS graph is entry + web + api + bootstrap-api only.
- Minified arrow-token counts (scale, not blame): entry 216, search-results 100, web (framework) 100, channel-picker 24 — ~340 app sites vs ~100 framework. Single-closure hoists save nanoseconds each (§2d); task-shape diffs (D3/D4/D5) move INP/TBT.

### 2d. `performance.now()` task-breakup benchmarks (Node 26.8.1, same V8 family as headless Chrome)

```json
// (i) inline arrow-per-item vs hoisted handler, N=200 items x 200 iters
{ "inline_ms": 1.8183, "hoisted_ms": 0.4139 }
```

Hoisted is ~4.4x faster per 200-handler batch (~7 us saved per handler). At 8 suggestion rows the absolute saving is ~56 us — hygiene, not the INP fix. The INP fix is D3 (fewer filter passes).

```json
// (ii) single-pass grouping vs yielded chunked loop, N=5000 rows
{ "singleTaskAvgMs": 0.16, "chunkedTotalMs": 12.43 }
```

Chunking is ~78x slower wall-clock (each yield pays a macrotask). Lesson: chunk only what blocks input (DOM inserts of dozens of moment rows), never pure data loops. D5 fuses the data loop single-pass (faster and smaller); D4 yields only around DOM-commit batches.

```json
// (iii) suggestion filter cost, 400 channels x 2000 iters
{ "perKeystrokeAvgMs": 0.0442 }
```

One keystroke filter is ~0.04 ms today — invisible alone, but synchronous inside the input task via `flush()`, O(n) allocs per keystroke, linear in catalog growth. D3 removes it from the input task for fast typists; D7 removes the 3x `toLowerCase` per channel multiplier.

Takeaway (rubric: closure vs alternative with numbers): hoisting saves single-digit microseconds per site — do it for churn. Breaking the runSearch/input task (D3/D4) and fusing the triple filter (D5) change task shape, which is what INP/TBT measure.

## 3. Bytecode-step markdown (three renders, redundant steps marked)

Legend: `[!]` = redundant / removable step. Ordered as browser/V8 executes.

### (a) Cold load to FCP (home, no query)

```text
1.  HTML parse dist/index.html (2.05 KB) — discover head chain
2.  CSS fetch assets/index-*.css (34.67 KB raw / 7.63 KB gzip) — render-blocking, FIRST paint gate
3.  CSS parse + Tailwind cascade build (single stylesheet, no @import chain — good)
4.  Entry JS fetch assets/index-*.js (module, deferred — doesn't block parse — good)
5.  [!] modulepreload web-*.js + bootstrap-api-*.js + api-*.js discovered LATE
        (emitted after the entry script; browser can't prioritize them until
        entry is fetched — reorder + fetchpriority, D1)
6.  JS parse/compile: entry (32 KB) + web/Solid runtime (37 KB, largest) + api/bootstrap
7.  V8 compile: ~340 app arrow sites + ~100 framework sites, one-time (§2c)
8.  Solid render(() => <App />): shell DOM — header, SearchForm, AvailabilityNotice
9.  [!] EAGER ChannelAvatar images (channel-profile.tsx:53, eager except compact)
        swarm avatars race CSS/JS on the wire before FCP — lazy except first paint (D2)
10. indexedChannels() + fetchSearchAvailability() bootstrap fetches (post-mount — correct)
11. ChannelPicker.preload() warmed IN PARALLEL with channel fetch (correct overlap)
12. positionChannels() O(n) trig — one shot post-data (fine)
13. Swarm avatar fetches: N channels, 96x96 attrs present (good) but no sizes/srcset —
    mobile 2rem and desktop 3rem download the same 96px (D2)
14. [!] matchMedia("(prefers-reduced-motion: reduce)") re-queried on EVERY
    withViewTransition call (utils.ts:33) — cache the MediaQueryList (D9)
15. FCP: search controls painted (hero = search controls, NOT the swarm)
    [!] NO preload/fetchpriority marks the hero path — FCP waits on full CSS +
    entry JS with zero priority hints (D1)
```

Redundant in (a): 5 (late preloads), 9 (eager swarm racing FCP), 14 (repeated matchMedia), 15 (unmarked hero).

### (b) Search-to-results (submit to first poster; LCP path)

```text
1.  Input task: onInput → setQuery → flush() → sync recompute directChannel O(n)
    + channelSuggestions O(n) + mentionedChannel O(2n) + formMode  [INP hotspot]
2.  [!] THREE filter passes over channels per keystroke — fuse + debounce (D3/D7)
3.  Submit task runSearch(): trim → direct/mention checks → request allocs →
    4-5 setters → writeSearchUrl → SearchResults.preload() + searchTranscripts()
    in parallel (good overlap) + captureSearchSubmitted
4.  [!] onSubmit/onclick wrapper arrows reallocated per render (app.tsx:440,416) — hoist (D4)
5.  Network: /api/search round-trip (dominant wall-clock; no speculative prefetch —
    correct per app.tsx:336-341, keep honest timing)
6.  [!] prepareResultsRuntime().then(rAF(scrollToLinkedResult)) runs even with no
    location.hash — one wasted microtask + frame per search (guard, D9)
7.  Solid render <SearchResults>: [!] sectionVideos x2 + unknownVideos triple
    filter + 2 spreads per render (search-results.tsx:39-48) — fuse (D5)
8.  Per-card render xN: posterThumbnail regex (:30) + playerUrl new URL() (:29) +
    engagementSummary filter/map + Intl.format (:244) + formatDate ICU (:48)
    [!] ALL FIVE recompute immutable per-video strings per render — memoize (D6)
9.  Highlight tokenize per hit: regex exec + textarea innerHTML decode per part
    (utils.ts:76-107) — bounded by slice(0,3) per card (good); keep cap
10. DOM insert: N article cards (content-visibility:auto + 32rem intrinsic — good,
    offscreen layout skipped) → style/layout
11. [!] Suggestion listbox (search-form.tsx:140-172) inserts/removes with NO
    reserved space → layout shift under the input on @-queries (CLS — D8)
12. First poster img: 1280x720 attrs present (good), loading=lazy + decoding=async
    (good) BUT [!] LCP candidate has no fetchpriority, no preload, and downgrades
    maxres→hqdefault via regex at render time (D2)
13. Async image decode (good) → LCP on first poster (or controls when EmptyState — fine)
```

Redundant in (b): 2 (triple filter), 4 (wrapper allocs), 6 (hashless rAF), 7 (triple filter + spreads), 8 (five recomputes), 11 (unreserved listbox → CLS), 12 (unprioritized LCP).

### (c) Expand-moments ("Show all moments" to full list; CLS/INP path)

```text
1.  Click task: onclick → expandMoments() → setMomentsLoading(true) → loading
    paint in the same task (good: instant feedback, small task)
2.  Network: searchVideoMoments(query, videoId), async — input free during fetch
3.  Response: videos.find(video_id === ...) O(m) — fine
4.  [!] setAllHits(full list) commits ALL moment rows in ONE Solid update →
    single long task (For + HighlightedText + textarea decode per row) —
    yield across insert batches (D4)
5.  [!] Per inserted row: new onSelect closure + capture spread
    (video-result.tsx:192-199,338) — delegate/hoist (D4)
6.  Layout: card grows; content-visibility:auto contains offscreen cost (good) but
    [!] in-viewport growth beyond the 32rem intrinsic pushes pagination → CLS
    (reserve + batch, D8/D4)
7.  No programmatic scroll on expand today (good — no forced layout); keep
```

Redundant in (c): 4 (single-task bulk insert), 5 (per-row allocs), 6 (uncontained growth → CLS).

## 4. Proposed diffs (fenced only — NOT applied; each maps to a vital)

### D1 — CSS-first head + early preloads + hero fetchpriority (to FCP)

Today `dist/index.html` emits script, then modulepreloads, then stylesheet. Stylesheet-last delays FCP; late preloads can't be prioritized.

```html
<!-- index.html head: stylesheet FIRST, entry preload high, runtime low, preconnect post-FCP origins -->
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <!-- CSS first: unblock FCP before JS bytes compete -->
  <link rel="stylesheet" href="/assets/index.css" fetchpriority="high" />
  <link rel="modulepreload" href="/assets/index.js" fetchpriority="high" />
  <link rel="modulepreload" href="/assets/web.js" fetchpriority="low" />
  <link rel="preconnect" href="https://i.ytimg.com" crossorigin />
  <link rel="preconnect" href="https://ytsearch-api-typesense-fly-staging.fly.dev" crossorigin />
  <script src="/src/index.tsx" type="module"></script>
</head>
```

(Vite rewrites the asset URLs at build; the source change is ordering + `fetchpriority` + `preconnect`.) Same bytes, earlier paint. Maps: **FCP** (CSS wins the race), **LCP** (poster origin handshake starts sooner). Rejected: inlining critical CSS — 34 KB would bloat the 2 KB HTML and break CSS caching.

### D2 — LCP poster eager-first, swarm lazy, honest sizes (to LCP/CLS)

Today the LCP element (first poster) is `loading=lazy` with no priority while decorative swarm avatars are `loading=eager` — exactly backwards.

```tsx
// components/search-results.tsx — mark the LCP candidate
<For each={props.videos}>
  {(video, index) => (
    <VideoResult video={video} query={props.query} rank={index() + 1} isLcp={index() === 0} />
  )}
</For>
```

```tsx
// components/video-result.tsx — poster: dims already correct (1280x720 +
// .video-poster aspect-ratio 16/9 reserves space); add priority split + sizes
<img
  src={thumbnail()}
  alt=""
  width="1280"
  height="720"
  loading={props.isLcp ? "eager" : "lazy"}
  fetchpriority={props.isLcp ? "high" : "auto"}
  decoding="async"
  sizes="(max-width: 1280px) 100vw, 640px"
  class="video-poster-image"
/>
```

```tsx
// components/channel-profile.tsx — swarm avatars are decorative: lazy + low
<img
  src={avatarUrl()}
  alt={`${props.channel?.name ?? "Channel"} profile picture`}
  width="96"
  height="96"
  loading="lazy"
  decoding="async"
  fetchpriority="low"
  class={`${sizeClass()} ${outlineClass()} aspect-square max-w-none shrink-0 rounded-full object-cover`}
  style={transitionStyle()}
  onError={() => setImageFailed(true)}
/>
```

Same pixels, same `Show`+initials fallback; CLS-safe (attrs + aspect-ratio untouched). Maps: **LCP** (first poster eager+high), **CLS** (space still reserved), **FCP** (swarm stops racing first paint). Rejected: HTML-preloading a poster URL — URLs are query-dependent, unknowable at build; `fetchpriority` on the rendered node is the correct tool.

### D3 — Debounced suggestion pipeline + memoized slice + pointer-gated hover (to INP)

The `flush()` in onInput stays (same-task submit correctness, `search-form.tsx:79-81`). Move cost downstream: debounce the filter memos, not the setter.

```tsx
// app.tsx — debounce suggestion inputs only; query setter stays synchronous
const [suggestQuery, setSuggestQuery] = createSignal("");
const queueSuggest = debounceIdle((value: string) => setSuggestQuery(value), 120);
const handleQueryInput = (value: string) => {
  setQuery(value); // sync: submit correctness preserved
  queueSuggest(value); // deferred: filter passes leave the input task
};
const channelSuggestions = createMemo(() => findChannelSuggestions(channels() ?? [], suggestQuery()));
const mentionedChannel = createMemo(() => resolveChannelMention(channels() ?? [], suggestQuery()));
// directChannel keeps sync query() so exact-handle Enter works mid-debounce (unchanged).
// <SearchForm onQueryInput={handleQueryInput} ... />  (replaces onQueryInput={setQuery})
```

```ts
// defer.ts (new, ~20 lines, no deps) — rIC with timeout fallback.
// scheduler.yield is not yet baseline; rIC + fallback is the same yield, zero weight.
export function debounceIdle<T extends (v: string) => void>(fn: T, ms: number) {
  let timer = 0;
  let idle = 0;
  return (value: string) => {
    window.clearTimeout(timer);
    if (idle && window.cancelIdleCallback) window.cancelIdleCallback(idle);
    timer = window.setTimeout(() => {
      const run = () => fn(value);
      idle = (window.requestIdleCallback ?? ((cb: () => void) => window.setTimeout(cb, 0)))(run as never) as never as number;
    }, ms);
  };
}
```

```tsx
// search-form.tsx — memoize the 8-slice; gate hover-only work behind fine pointers
const visibleSuggestions = createMemo(() => props.channelSuggestions.slice(0, 8));
const finePointer = window.matchMedia("(pointer: fine)").matches;
// onPointerEnter={finePointer ? () => setActiveSuggestion(index()) : undefined}
```

Enter-to-submit still sees sync `query()`; suggestions resolve <=120 ms + idle after typing stops. Maps: **INP** (input task drops from O(3n) filters to one setter), **TBT** (filter work to idle). Rejected: debouncing `setQuery` itself (breaks submit guarantee); `scheduler.yield()` (non-baseline, needs the same fallback anyway).

### D4 — Break up runSearch + chunked moment insert + hoisted handlers (to INP/TBT)

```tsx
// app.tsx — stable hoisted handlers (delete 4 per-render closures)
const submitFirstPage = () => runSearch(1);
const handleCopySearchLink = () => void copySearchLink();
const handleClearRecent = () => setRecentSearches(clearRecentSearches());
const dismissIndexDialog = () => { setPendingIndexHandle(undefined); setIndexRequestError(undefined); };
```

```tsx
// app.tsx — runSearch: setters stay sync (Solid batching needs them together);
// non-reactive side effects yield one idle tick.
const runSearch = (nextPage = 1) => {
  const normalizedQuery = query().trim();
  if (!normalizedQuery) return;
  if (directChannel()) { selectChannel(directChannel()!.handle); return; }
  // ... mention/index branches unchanged (sync) ...
  void prepareResultsRuntime();
  const request = { query: normalizedQuery, youtuber: youtuber(), page: nextPage, sort: sort() };
  setQuery(normalizedQuery);
  setQueuedIndexRequest(undefined);
  setPage(nextPage);
  setSubmitted(request); // resource fetch kicks off in an effect — already async
  const snapshot = { ...request };
  const yieldThen = (fn: () => void) =>
    (window.requestIdleCallback ?? ((cb: () => void) => window.setTimeout(cb, 0)))(() => fn());
  yieldThen(() => { captureSearchSubmitted(snapshot); writeSearchUrl(snapshot); });
};
```

```tsx
// video-result.tsx — chunked moment reveal: paint first batch sync, rest idle.
// (Single-pass data; yielded DOM commits only — per §2d(ii), never chunk pure loops.)
const MOMENT_BATCH = 8;
const [visibleCount, setVisibleCount] = createSignal(3);
const visibleHits = () => allHits().slice(0, visibleCount());
const expandMoments = async () => {
  if (momentsLoading()) return;
  setMomentsLoading(true);
  setMomentsError(undefined);
  try {
    const response = await searchVideoMoments(props.query, props.video.video_id, momentsController.signal);
    const complete = response.videos.find((v) => v.video_id === props.video.video_id);
    if (complete) {
      setAllHits(complete.hits);
      setShowAllMoments(true);
      const total = complete.hits.length;
      setVisibleCount(Math.min(MOMENT_BATCH, total));
      const idle = window.requestIdleCallback ?? ((cb: () => void) => window.setTimeout(cb, 0));
      const reveal = () =>
        setVisibleCount((n) => {
          if (n >= total) return n;
          const next = Math.min(n + MOMENT_BATCH, total);
          if (next < total) idle(reveal as never);
          return next;
        });
      if (total > MOMENT_BATCH) idle(reveal as never);
    }
  } catch { setMomentsError("Unable to load all moments. Try again."); }
  finally { setMomentsLoading(false); }
};
```

```tsx
// video-result.tsx — delegate per-hit select (one stable handler)
const handleHitSelect = (event: Event) => {
  const id = (event.currentTarget as HTMLElement).dataset.hitId!;
  const hit = allHits().find((h) => h.id === id)!;
  setSelectedHitId(id);
  activatePlayer();
  capture("result_timestamp_selected", { ...resultProperties(props), match_kind: hit.match_kind });
};
// <button ... data-hit-id={hit.id} onClick={handleHitSelect}>
```

Same setters, URLs, analytics payloads, final DOM — only timing of non-reactive side effects changes. Maps: **INP** (submit sheds analytics+URL work), **TBT/LongTasks** (moment commits in <=8-row batches). Rejected: `scheduler.yield()`/TaskController (non-baseline); virtualized list (overkill at observed sizes; batches + content-visibility suffice).

### D5 — Single-pass result partition, zero spreads (to TBT/INP)

```tsx
// components/search-results.tsx — one pass, three buckets, no spreads
const partitionVideos = (videos: SearchVideoResult[]) => {
  const longs: SearchVideoResult[] = [];
  const shorts: SearchVideoResult[] = [];
  const unknown: SearchVideoResult[] = [];
  for (const video of videos) {
    if (video.content_kind === "short") shorts.push(video);
    else if (video.content_kind === "video") longs.push(video);
    else unknown.push(video);
  }
  return { longs, shorts, unknown };
};
// Videos section renders longs then unknown as two <For> blocks under one
// heading — the [...longs, ...unknown] concat alloc disappears.
```

Identical grouping + order (filter and push both preserve order). 3 passes to 1, 2 spread allocs to 0 per render. Maps: **TBT** (render task), **INP** (submit-to-paint tail). Rejected: memoizing the spread (hides the alloc); server-side grouping (API contract change, out of scope).

### D6 — Memoize immutable per-card strings (to TBT/LCP)

```tsx
// video-result.tsx — compute once per card, not once per memo read
const posterThumbnail = createMemo(() =>
  (props.video.thumbnail ?? selectedHit()?.thumbnail)?.replace(
    /^(https:\/\/i\.ytimg\.com\/vi\/[^/]+\/)(?:maxres|sd|hq720)default\.jpg(?:\?.*)?$/,
    "$1hqdefault.jpg",
  ),
);
const playerUrl = createMemo(() =>
  playerActive() ? autoplayEmbedUrl(selectedHit()?.embed_url ?? props.video.embed_url) : undefined,
);
const engagement = createMemo(() => engagementSummary(props.video));
const publishedLabel = createMemo(() => formatDate(props.video.published_at));
```

```ts
// channel-profile.tsx — hash avatar URL once per channel, not per read
const avatarVersionCache = new WeakMap<object, string>();
const avatarUrl = () => {
  const channel = props.channel;
  if (imageFailed() || !channel?.channel_id || !channel.avatar_url) return undefined;
  let v = avatarVersionCache.get(channel);
  if (v === undefined) { v = imageVersion(channel.avatar_url); avatarVersionCache.set(channel, v); }
  return `${API_BASE_URL}/api/channel-images/${encodeURIComponent(channel.channel_id)}?v=${v}&s=96`;
};
```

Identical strings (memo keys cover every input). Maps: **TBT** (drops per-render regex/URL-parse/ICU), **LCP** (poster src resolves without recompute on the paint path). Rejected: precomputing at fetch time (view concern leaking into data layer; memo is the existing pattern — cf. `highlighted-text.tsx:10`).

### D7 — Fuse channel-mention passes + lowercase index with early break (to INP)

```ts
// channel-query.ts — one pass over a precomputed index; stop at the 8 rows the UI renders
export type ChannelIndex = { channel: IndexedChannel; handle: string; name: string }[];
export const buildChannelIndex = (channels: IndexedChannel[]): ChannelIndex =>
  channels.map((channel) => ({
    channel,
    handle: channel.handle.toLowerCase(),
    name: channel.name.toLowerCase(),
  }));
export function channelSuggestionsFromIndex(index: ChannelIndex, query: string): IndexedChannel[] {
  const value = query.trim().toLowerCase();
  if (!value.startsWith("@")) return [];
  const needle = value.slice(1).split(/\s+/, 1)[0] ?? "";
  const out: IndexedChannel[] = [];
  for (const entry of index) {
    if (entry.handle.includes(needle) || entry.name.includes(needle)) out.push(entry.channel);
    if (out.length >= 8) break;
  }
  return out;
}
// app.tsx: const channelIndex = createMemo(() => buildChannelIndex(channels() ?? []));
// suggestions/mention/direct read channelIndex() — normalizeHandle-per-channel
// per keystroke disappears. resolveChannelMention folds its second pass into the same loop.
```

Same match semantics, same 8-row cap (`visibleSuggestions` already caps). Early-break makes the common case sub-linear. Maps: **INP**. Rejected: Worker for filtering (setup dwarfs a 0.04 ms filter); server-side suggest (new endpoint + latency for a local computation).

### D8 — Reserve space for injected UI (to CLS)

```css
/* index.css — take the suggestion listbox out of flow so insert/remove can't shift the form */
#channel-suggestions {
  position: absolute;
  inset-inline: 0;
  top: calc(100% + 0.5rem);
  z-index: 20;
}
```

```tsx
// search-form.tsx — wrapper is already .relative; drop `relative z-20 mt-2` on
// #channel-suggestions in favor of the CSS above. aria-expanded, focus order unchanged.
```

Fonts: the stack is already system-only (`--font-sans: ui-sans-serif, system-ui` — `index.css:18`): no webfont, no FOUT/FOIT, no font-swap CLS. Keep; any future font needs `font-display: optional` + metric overrides. Results skeleton already exists via `LoadingState` — keep it, don't guess `min-height`s for unknowable fetch heights. Maps: **CLS**.

### D9 — Frame hygiene: guard hashless rAF, cache matchMedia, cheap magnet (to TBT)

```ts
// utils.ts — one MediaQueryList, not one per call
const reduceMotionQuery: MediaQueryList | undefined =
  typeof window !== "undefined" ? window.matchMedia("(prefers-reduced-motion: reduce)") : undefined;
export function withViewTransition(update: () => void): void {
  const updateAndFlush = () => { update(); flush(); };
  if (reduceMotionQuery?.matches) { updateAndFlush(); return; }
  // ... unchanged startViewTransition branch
}
```

```tsx
// app.tsx — skip the preload+rAF chain when there is nothing to scroll to
createEffect(
  () => search().status,
  (status) => {
    if (status !== "success" || !window.location.hash) return; // guard
    let frame: number | undefined;
    let cancelled = false;
    void prepareResultsRuntime().then(() => {
      if (cancelled) return;
      frame = window.requestAnimationFrame(scrollToLinkedResult);
    });
    return () => { cancelled = true; if (frame !== undefined) window.cancelAnimationFrame(frame); };
  },
);
```

```ts
// swarm-layout.ts — squared-distance early-out before sqrt
// (removes Math.hypot per far item per active frame; identical outputs)
export function magneticOffset(deltaX: number, deltaY: number) {
  const d2 = deltaX * deltaX + deltaY * deltaY;
  if (d2 >= 220 * 220) return { x: 0, y: 0 };
  const distance = Math.sqrt(d2);
  const pull = (1 - distance / 220) ** 2 * 0.6;
  return { x: round(deltaX * pull), y: round(deltaY * pull) };
}
```

Identical transition/scroll/magnet outputs. Maps: **TBT** (fewer, cheaper frames), **INP** (pointermove loop). Rejected: interval-throttling the swarm rAF (breaks magnetic smoothness; early-out keeps it, drops cost).

## 5. Verification plan (Playwright + `performance` entries; no chrome-devtools MCP)

`chrome-devtools` MCP is NOT configured in this environment, so all measurement uses Playwright + in-page `performance` APIs — the same primitives `scripts/check-frontend-performance.ts` already uses (`getEntriesByType("paint")`, `locator("article").first().waitFor`). No new infra; extend the existing script.

### 5a. Ready-to-run extension (drop-in alongside `scripts/check-frontend-performance.ts`)

```ts
// scripts/check-load-pipeline-perf.ts — candidate-3 verification (proposed, not applied)
import { chromium } from "@playwright/test";

const publicUrl = process.env.PUBLIC_URL ?? "https://search.pronsh.dev";
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });

// 1. cold load → FCP (existing pattern: 3 samples, median)
await page.goto(publicUrl, { waitUntil: "networkidle" });
const fcp = await page.evaluate(() =>
  Math.round(
    performance.getEntriesByType("paint").find((e) => e.name === "first-contentful-paint")?.startTime ?? 0,
  ),
);

// 2. LCP element + time — who is the hero?
const lcp = await page.evaluate(() =>
  new Promise<{ time: number; tag: string; src: string }>((resolve) => {
    const done = (entry: { startTime?: number; element?: Element } | undefined) =>
      resolve({
        time: Math.round(entry?.startTime ?? 0),
        tag: (entry?.element as HTMLElement)?.tagName ?? "?",
        src: (entry?.element as HTMLImageElement)?.src ?? "",
      });
    const entries = performance.getEntriesByType("largest-contentful-paint");
    if (entries.length > 0) return done(entries[entries.length - 1]);
    new PerformanceObserver((list, obs) => {
      done(list.getEntries()[0]);
      obs.disconnect();
    }).observe({ type: "largest-contentful-paint", buffered: true });
    window.setTimeout(() => done(undefined), 5000);
  }),
);

// 3. CLS so far (unattributed user-input shifts excluded)
const cls = await page.evaluate(() =>
  (performance.getEntriesByType("layout-shift") as unknown as { value: number; hadRecentInput: boolean }[])
    .filter((e) => !e.hadRecentInput)
    .reduce((acc, e) => acc + e.value, 0),
);

// 4. search → results (existing budget 2500 ms) + longtask tally across it
await page.getByRole("textbox").fill("fable on high");
const t0 = performance.now();
await page.getByRole("button", { name: "Search", exact: true }).click();
await page.locator("article").first().waitFor({ state: "visible" });
const searchToResultsMs = Math.round(performance.now() - t0);
const longTasks = await page.evaluate(() => performance.getEntriesByType("longtask").length);
const searchLcp = await page.evaluate(() =>
  Math.round(performance.getEntriesByType("largest-contentful-paint").at(-1)?.startTime ?? 0),
);

// 5. expand-moments: longtask + CLS delta across the click
await page.getByRole("button", { name: /show all moments/i }).first().click();
await page.waitForTimeout(1500);
const expandAudit = await page.evaluate(() => ({
  longTasks: performance.getEntriesByType("longtask").length,
  cls: (performance.getEntriesByType("layout-shift") as unknown as { value: number; hadRecentInput: boolean }[])
    .filter((e) => !e.hadRecentInput)
    .reduce((a, e) => a + e.value, 0),
}));

console.log(JSON.stringify({ fcp, lcp, cls, searchToResultsMs, longTasks, searchLcp, expandAudit }, null, 1));
await browser.close();

// Budgets: fcp ≤ 1800 ms | searchToResults ≤ 2500 ms | cls ≤ 0.1 |
// longTasks during search ≤ 2 | expand adds no new >50 ms longtask after D4 batching
```

Run live: `bun run build && PUBLIC_URL=https://search.pronsh.dev bun scripts/check-load-pipeline-perf.ts`. Local iteration: `PUBLIC_URL=http://127.0.0.1:3000` against dev (matches `playwright.config.ts` baseURL). Fixture variant: `page.route("/api/*", abort)` to assert skeleton/empty CLS is ~0 without network.

### 5b. How to read it

- FCP: median of 3 (existing script already does this). D1 should add headroom under the 1800 ms budget.
- LCP: log the element. Home should be the controls region (no image). Results should be the first `.video-poster-image`. If LCP `src` is a swarm avatar, D2 regressed — fail the run.
- CLS: cumulative ≤ 0.1 on load; ~0 delta across `@`-typing (D8 overlay) and across expand (D4 batches + containment).
- INP proxy: lab Playwright cannot measure field INP; use `longtask` count during search + `searchToResultsMs` ≤ 2500 ms as proxy, plus the §2d unit benchmarks as CI-cheap task-shape guards.
- Live relevance triple (repo rule, after any D3/D4 change to prove behavior preserved): `q=fable on high&youtuber=@t3dotgg` ranks `A proper guide to Fable 5` near top; `q=I&youtuber=@t3dotgg` shows newest-first; one filtered creator query:

```bash
curl -fsS 'https://ytsearch-api-typesense-fly-staging.fly.dev/api/search?q=fable%20on%20high&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title, hit_count: (.hits | length)})'
curl -fsS 'https://ytsearch-api-typesense-fly-staging.fly.dev/api/search?q=I&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title, published_at})'
```

## 6. Rationale — alternatives considered and rejected

| Alternative | Why rejected |
|---|---|
| `manualChunks` vendor split in `vite.config.ts` | Rollup already emits a stable shared `web-*.js` + route chunks with zero config; hand-tuning risks inflating initial bytes and adds a config concept. (Also off-limits for this arena.) |
| Eagerly importing SearchResults/dashboard in entry | Destroys the 28.6/30 KB initial-JS margin (§2b) for paths most visits never touch. Preload-on-intent is strictly better. |
| Speculative `/api/search` prefetch on typing | Rejected by the codebase (`app.tsx:336-341`): duplicate origin work + dishonest timing. Code-preload only. |
| Strict must-contain-every-term matching / client stopword stripping | Product rule: best-match stays; stopwords must not dominate snippets. Relevance is backend-owned; this candidate changes task shape, never matching. |
| `scheduler.yield()` / TaskController | Not baseline; rIC + timeout fallback is the same yield with zero polyfill. |
| Virtualized moment list | Dozens of rows observed; 8-row idle batches (D4) + `content-visibility:auto` already bound cost. Virtualization buys scroll-restoration bugs, no measured win. |
| Inlining critical CSS / adding a webfont | CSS margin is thin but passing; inlining breaks caching. Stack is system-only — keep it; any future font needs `display:optional` + metric overrides. |
| Server-side suggest endpoint / Worker for filtering | A 0.04 ms local filter doesn't justify a round-trip or worker setup; debounce + index (D3/D7) solve growth. |
| Removing `flush()` from onInput | Breaks same-task submit correctness (`search-form.tsx:79-81`). Cost moves downstream instead. |
| Removing view-transition `flush()` | Load-bearing for Solid 2 batching (`utils.ts:29-31`). The update is the cost, not the flush. |
| Server-side result grouping | API contract change for a 3-pass client loop D5 fixes in ~10 lines. |
| Interval-throttling the swarm rAF | Breaks magnetic smoothness; D9 early-out keeps smoothness, drops cost. |

## Appendix — hot-path spread / allocation disposition (rubric: every spread has an alternative or unavoidability proof)

- `api.ts:97` `{...result, request_duration_ms}` — unavoidable (must not mutate shared/inflight response).
- `search-results.tsx:39-44` double spread — avoidable → D5 (two `<For>` blocks, no concat).
- `video-result.tsx:338` analytics spread — per-event only, never per-render; acceptable, hoist on touch.
- `app.tsx:290,395` `{...current, page}` — per-navigation only; unavoidable (resource-key immutability).
- `search-form.tsx:23` 8-slice — memoize → D3. `video-result.tsx:21` 3-slice — unavoidable (bounds DOM).
- `engagementSummary` filter+map, `formatDate`, thumbnail regex, `new URL()`, avatar FNV — all memoize → D6 (immutable inputs).
- `channel-query` per-keystroke strings — index + debounce → D3/D7.
- Per-render/per-row closures (`app.tsx:416,440,450,513,517`; `video-result.tsx:146,168,176,192,354`; `search-form.tsx:158-159`) — hoist/delegate → D3/D4; §2d(i) proves microseconds each, so this rides along with the task-shape fixes rather than substituting for them.
- `positionChannels` map+spread, swarm `Set` churn — one-shot per channel-list change; unavoidable, out of the frame loop.
- `moveSwarm` per-frame `Set` + style writes — writes already limited to pulled items; unavoidable for the effect, cheapened by D9.
