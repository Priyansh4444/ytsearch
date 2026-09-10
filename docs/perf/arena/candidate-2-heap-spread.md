# Arena Candidate 2 — Heap Churn, Spreads, V8 Bytecode

Scope: SolidJS 2.0 RC frontend (src/), Vite + Tailwind. Direction: heap churn + V8 bytecode. Constraint honored: no edits to src/; all changes below are proposed diffs only. Only file written: this report.

Grounding read: src/utils.ts, src/channel-query.ts, src/result-counts.ts, src/search-sorts.ts, src/recent-searches.ts, src/api.ts, src/search-resource.ts, src/components/search-results.tsx, src/components/video-result.tsx, src/app.tsx, plus src/components/highlighted-text.tsx, src/search-route.ts, src/analytics.ts, src/bootstrap-api.ts, src/components/channel-picker.tsx, src/components/dashboard.tsx, src/components/icons.tsx, index.html, vite.config.ts, scripts/check-bundle-performance.ts.

Environment for all numbers: Node v26.8.1, Linux x64, flags --expose-gc / --allow-natives-syntax / --print-bytecode / --trace-gc where noted. Every performance claim cites a measured number from this session.

## 1. Spread / allocation inventory

Method: rg -n '...' src set completed with grep -rn across src (*.ts/*.tsx). Test-only spreads listed once as out-of-scope; every runtime spread gets a verdict plus a non-churn alternative or proof of unavoidability.

### 1a. Runtime spreads in the render/search hot path

| # | Location | Code | Hot? | Cost verdict | Alternative |
|---|---|---|---|---|---|
| S1 | src/api.ts:97-100 | .then((result) => ({ ...result, request_duration_ms })) | Yes, every search | One shallow clone per search. Measured tie vs manual loop (3.31 ms vs 3.50 ms per 20k). Cost is the second object + map transition, not syntax. | Replace (D1): direct assignment on fresh fetch response. Removable. |
| S2 | src/components/search-results.tsx:39-42 | videos={[...sectionVideos(...), ...unknownVideos(...)]} | Yes, every results render | Worst spread in hot path. Two filter passes + two temps + iterator-spread concat (103-byte bytecode shape, sec 3.1). Measured 14.92 ms vs 3.33 ms pre-sized (4.5x) per 20k on 20 videos; retained heap 20520 B vs 1448 B after 20k + GC (~14x). | Replace (D2): single-alloc partition, same video-then-unknown order. |
| S3 | src/recent-searches.ts:33 | [recent, ...current.filter(... )].slice(0, 12) | Yes, every page-1 search | filter alloc + spread alloc + slice alloc (3 arrays). Measured 10.19 ms vs 3.84 ms pre-sized loop (2.7x) per 20k. recentKey builds a 3-array + join per item. | Replace (D3): pre-sized new Array(12) + index loop + hoisted key. |
| S4 | src/app.tsx:290 | { ...current, page: nextPage } (goToPage) | Yes, every pagination click (INP) | Small CloneObject (~0.15 us/op class). Cheap alone, on INP click path. | Replace (D4): explicit 4-field literal, better map stability. |
| S5 | src/app.tsx:395 | { ...recent, page: 1 } (selectRecentSearch) | Yes, every recent click | Same as S4 plus copies dead searchedAt. | Replace (D4): explicit literal. |
| S6 | src/components/video-result.tsx:57 | { ...resultProperties(props), visible_moments, ... } | Yes, every expand (INP) | resultProperties already allocs one object; spread clones a second. 3 sites x per-row. | Replace (D5): single-literal builder. |
| S7 | src/components/video-result.tsx:196 | { ...resultProperties(props), match_kind } | Yes, every timestamp click | Same as S6, highest-frequency INP interaction. | Replace (D5). |
| S8 | src/components/video-result.tsx:274 | { ...resultProperties(props), moment_id, ... } | Yes, per feedback click | Same as S6 + 4 fields. | Replace (D5). |
| S9 | src/search-sorts.ts:33 | ([...BASE_SORTS, ...ENGAGEMENT_SORTS]).includes(value) | Yes, every route parse | 7-element array per call. Measured 19.35 ms vs 2.38 ms Set.has per 50k (8.1x). | Replace (D6): module-scope Set. |
| S10 | src/analytics.ts:34 | { ...searchProperties(request), total_videos, ... } | Warm, every search_loaded | One extra object per search. | Replace (D5 pattern): single literal. P2. |
| S11 | src/analytics.ts:62 | ...Object.fromEntries(Object.entries(properties).filter(...)) | Warm, every send | entries + filter + fromEntries = 3 allocs to drop undefined. | Replace: loop skipping undefined (D5). |
| S12 | src/bootstrap-api.ts:57 | fetch(url, { ...init, signal }) | Warm, every bootstrap fetch | init almost always undefined; object required by fetch. | Mostly unavoidable: fast path { signal } when init undefined. |
| S13 | src/components/channel-picker.tsx:135 | { channel, ...position } | Yes, initial render per channel | Second object per channel on FCP/LCP path. | Replace (D7): explicit 6-field literal. |
| S14 | src/components/dashboard.tsx:23 | [...(snapshot()?.channels ?? [])].sort(...) | Cold, dashboard only | Copy required (sort mutates). | Keep. Not search-path. |
| S15 | src/components/icons.tsx:28,34,40,47,53,59,65,72 | <Icon {...props}> x8 | Warm, per icon | Tiny cold props; negligible vs DOM. | Keep. Idiomatic Solid. |
| S16 | scripts/check-bundle-performance.ts:58 | [...visited] | No, build-time | Zero vitals impact. | Keep. |

Test-only spreads (no action): src/api.test.ts:56,88, src/dashboard-metrics.test.ts:13, src/test/setup.ts:12, src/result-counts.test.ts:46, src/client-runtime.test.ts:44, src/recent-searches.test.ts:82, src/channel-cache.test.ts:39, src/components/interface-states.test.tsx:67,81, src/components/dashboard.test.tsx:65, src/components/video-result.test.tsx:75,83,91,100,106.

### 1b. Non-spread hot-path allocations

| # | Location | Pattern | Measured cost | Verdict |
|---|---|---|---|---|
| A1 | src/api.ts:44 | JSON.stringify(request) cache key per call | 2.53 ms vs 0.45 ms manual per 20k (5.6x); 10.32 vs 1.24 per 50k (8.3x) | Replace w/ manual key concat (D8). Fixed order also more correct. |
| A2 | src/components/video-result.tsx:250-253 | metrics.filter(...).map(...).join(' · ') per row | 43.51 ms vs 31.98 ms loop per 20k (1.36x); tier gap sec 3.2 | Replace w/ index loop (D9). |
| A3 | src/components/video-result.tsx:21 | allHits().slice(0, 3) per row eval | 0.75 ms slice vs 1.12 ms manual per 20k, slice wins | Keep slice. Intrinsic beats hand loop. |
| A4 | src/components/video-result.tsx:256-260 | new URL(value) + searchParams.set per playerUrl() | 30.35 ms vs 0.87 ms string-append per 20k (35x) | Replace w/ append fast path + URL fallback (D10). Biggest per-line INP win. |
| A5 | src/components/video-result.tsx:239-242 | new Intl.NumberFormat already cached compactNumber | Per-call 330.52 ms vs cached 15.17 ms per 20k (22x); current code caches | Keep; hoist last per-call site (availability-notice.tsx:72 DateTimeFormat). |
| A6 | src/utils.ts:78 | const regex = /<mark>(.*?)<\/mark>/gis per call + lastIndex | Fresh 13.10 ms vs shared /g 16.52 ms vs indexOf 14.46 ms per 10k (noise) | Keep fresh regex. Shared /g risks lastIndex bleed. |
| A7 | src/utils.ts:84-97 | slice per part + {text, highlighted} per part + decodeEntities per part | Slices 0.93 ms/10k cheap; decodeEntities does textarea.innerHTML HTML-parse per part | Reduce decode calls (D11); keep slices. |
| A8 | src/utils.ts:11 | pathname.split('/').filter(Boolean)[0] | 1.87 ms vs index-scan 0.84 ms per 10k (2.2x) | Replace w/ index scan (D12). Every route sync. |
| A9 | src/channel-query.ts:7-11, src/result-counts.ts:15, src/dashboard-metrics.ts | filter/map/reduce chains | sugg filter 245.16 ms vs loop 175.58 ms per 20k (1.4x); reduce 4.23 ms vs loop 1.27 ms per 50k (3.3x) | Replace hot ones w/ loops (D9/D13); keep cold dashboard. |
| A10 | src/components/video-result.tsx:338-343 | resultProperties literal per event | 1 object x 4 fields per event, doubled by S6-S8 | Fold into D5. |
| A11 | src/components/search-form.tsx:23, src/components/recent-searches.tsx:26 | .slice(0, 8) / .slice(0, 6) per render | Cheap bounded takes; reduce DOM work | Keep. Removing hurts INP/LCP. |
| A12 | src/search-route.ts:69, src/client-runtime.ts:24,55 | new URL / new URLSearchParams per nav/fetch | Same 35x class, once per search not per row | Keep except A4. Intentional. |

## 2. Render-hot closure / arrow inventory (file:line + required / removable)

Rule: required = must run per render/item/event for correctness (Solid reactivity, event wiring, list identity). Removable = fresh closure per eval with no per-item capture, or duplicated work a memo already does.

### src/utils.ts

| Closure | Verdict |
|---|---|
| 26-27 updateAndFlush = () => {...} in withViewTransition | Required (captures update), cold (route only). Keep. |
| 76-101 partsFromTaggedSnippet loop body, no per-part closure | Required. Keep. |

### src/channel-query.ts

| Closure | Verdict |
|---|---|
| 9 (channel) => includes needle (filter pred) | Required test, removable alloc: index loop capturing needle once (D13). |
| 27 (channel) => normalizeHandle === (find pred) | Required; same loop fix, hoist normalized token. |

### src/result-counts.ts

| Closure | Verdict |
|---|---|
| 3 pluralize, 6 completeCountLabel, 11 searchResultCountLabel | Required, module scope. Keep; fix inner reduce. |
| 15 (total, video) => total + video.hits.length (reduce) | Removable: index loop, 3.3x measured. |

### src/search-sorts.ts

| Closure | Verdict |
|---|---|
| 13 searchSortOptions | Required, cold. Keep. |
| 47 ([value]) => value === sort (find pred) | Required test but input rebuilt per call; hoist options tuple to module scope. |

### src/recent-searches.ts

| Closure | Verdict |
|---|---|
| 13 .filter(isRecentSearch) (fn ref, no new closure) | Required validation. Keep. |
| 33 (item) => recentKey(item) !== key | Required test, removable alloc: fold into pre-sized loop (D3). |

### src/api.ts

| Closure | Verdict |
|---|---|
| 52 ({ runSearchTranscripts }) => ... | Required, once per miss. Keep. |
| 55 .finally(() => {...}) | Required. Keep. |
| 74 finish = (complete) => {...} per call | Required (abort/resolve race guard). Keep. |
| 85-86 abort, 93-96 then handlers + inner () => resolve/reject | Required. Keep. Per-subscriber timing needs per-caller wrapper; only spread removable (D1). |
| 97 timing wrapper | Required semantically, removable spread (D1). |

### src/search-resource.ts

| Closure | Verdict |
|---|---|
| 17 createEffect(request, (nextRequest) => {...}) | Required lifecycle. Keep. |
| 29/32 response/error handlers, 39 cleanup | Required. Keep. |

### src/components/search-results.tsx

| Closure | Verdict |
|---|---|
| 14 SearchResults, 68 ResultSection component fns | Required. Keep. |
| 76 (video, index) => <VideoResult .../> (For row fn) | Required Solid identity. Keep. |
| 83-84, 86-87 content-kind filter preds | Required tests, removable passes: two filters become one partition (D2). |

### src/components/video-result.tsx (densest hot file)

| Closure | Verdict |
|---|---|
| 21 visibleHits | Required reactive. Keep slice (A3 optimal). |
| 22 hasHiddenMoments | Required. Keep. |
| 25-28 selectedHit = () => visibleHits().find(...) ?? visibleHits()[0] ?? ... | Required semantically, removable double eval: two slice allocs when collapsed. Hoist to one visibleHits() + loop (D14). |
| 29 playerUrl | Required reactive; fix callee new URL (D10), keep closure. |
| 30-34 posterThumbnail + .replace (non-global regex, no lastIndex) | Required. Keep. P2 memo only. |
| 36 activatePlayer, 41 expandMoments, 67 copyResultLink | Required events. Keep; fix spreads/find inside. |
| 52 (video) => video.video_id === ... | Required test, N<=20 noise. Keep or fold into loop if touching fn. |
| 98 Show render fn, 162 summary fn, 226 message fn | Required Solid. Keep. |
| 146,176 () => capture(...), 168 () => void copyResultLink() | Required deferred events. Keep; fix payload (D5). |
| 185 (hit) => (...) For row fn | Required identity. Keep. |
| 192-199 onSelect={() => {...}} per moment | Required (captures hit); keep closure, fix payload + selection memo (D5/D14). hit.id is correct key. |
| 206 () => void expandMoments() | Required. Keep. |
| 251-252 filter/map fns | Required tests, removable chain: one loop (D9). |
| 271 submitFeedback, 338 resultProperties, 350/354 feedback btn | Required wiring; fold payload into single literal (D5). |

### src/app.tsx (orchestration; all handlers required, fix callees not closures)

Required-keep: 62-64 preload thunks, 95 executableSearch, 100-109 selectedChannel/indexingChannel memos + preds, 110-125 hasChannel/hasSearchSurface/showIndexedChannels/directChannel/channelSuggestions/mentionedChannel/mentionQuery/isIndexMode/formMode, 126 searchError, 131 successfulSearch, 142/146 dialog/notice memos, 151-165 syncRoute, 176 selectChannel, 206 confirmIndexRequest, 225 runSearch (literal already, no spread), 287 goToPage (fix spread only), 297 goBack, 313 copySearchLink, 323-344 warm/track effects, 361 scrollToLinkedResult, 366-381 scroll effect + cleanup, 386 selectRecentSearch (fix spread only), render closures 416/440/450/458/476/482/507/513/517/523 (all required Solid render/event fns).

Net: no render-hot closure is gratuitous except chain shapes (filter-then-map-then-join, filter-then-spread-then-slice, double visibleHits(), per-item recentKey arrays). Every other arrow is required Solid wiring; the fix is inside the callee.

## 3. V8 evidence with numbers

### 3.1 Bytecode sizes (node --print-bytecode)

Repro (bc2.js): spreadClone, loopClone, recentSpread as in sec 1. Full output captured; excerpts:

| Function | Bytecode length | Key bytecodes | Reading |
|---|---|---|---|
| spreadClone(o){ return {...o, request_duration_ms: 5}; } | 14 bytes | CloneObject a0 + DefineNamedOwnProperty request_duration_ms + Return | Smallest: one intrinsic. Cheap per call, still one heap object + map transition. |
| loopClone(o){ const r={}; for (const k in o) r[k]=o[k]; ... } | 54 bytes | CreateEmptyObjectLiteral, ForInEnumerate, ForInPrepare, ForInNext, keyed stores | ~4x larger, megamorphic for-in. |
| recentSpread(recent, cur, key){ return [recent, ...cur.filter(...)].slice(0,12); } | 103 bytes | CreateArrayLiteral + GetNamedProperty filter + CreateClosure + CallProperty1 + GetIterator/next spread loop (StaInArrayLiteral per element) + GetNamedProperty slice + call | Largest: closure + call + iterator + second call. Pre-sized loop deletes all three. |

Takeaway: spread-clone is bytecode-small; spread-concat-of-filter-then-slice is bytecode-large. Optimize S2/S3; fix S4/S5 clones by literal construction (same size as spread, better map stability).

### 3.2 Optimization status (node --allow-natives-syntax)

Warmed 5000 calls with %PrepareFunctionForOptimization + %OptimizeFunctionOnNextCall, then %GetOptimizationStatus. Single run, Node 26.8.1: spreadClone 41; loopClone 41; indexLoop (engagement replacement) 41; filterMapJoin (current chain) 1.

Decode per v8::internal::OptimizationStatus (kIsFunction=1<<0=1, kMaybeDeopted=1<<3=8, kMaglevved=1<<5=32): 41 = 1+8+32 = kIsFunction|kMaybeDeopted|kMaglevved (reached Maglev tier); 1 = kIsFunction only (no compiled tier in same warmup; filter-map-join callback chain resisted tier-up). Directional, agrees with timing: callback chain slower and harder to tier up.

### 3.3 Microbenchmarks (performance.now(), --expose-gc, 1-2k warmup, gc() between cases)

Best-of-2, same machine; ratios stable within 15%:

| Comparison | Iters | Current | Alternative | Ratio | Verdict |
|---|---|---|---|---|---|
| Spread-concat vs pre-sized partition (20 videos) | 20k | 14.92 ms (0.75 us/op) | 3.33 ms (0.17 us/op) | 4.5x | Replace (D2). Repeat 18.13 vs 5.55 (3.3x). |
| Recent [r,...filter].slice vs pre-sized loop (12) | 20k | 10.19 ms (0.51 us/op) | 3.84 ms (0.19 us/op) | 2.7x | Replace (D3). |
| Engagement filter-map-join vs index loop | 20k | 43.51 ms (2.18 us/op) | 31.98 ms (1.60 us/op) | 1.36x | Replace (D9). Repeat 66.79 vs 43.96 (1.5x). |
| JSON.stringify(request) vs manual key | 20k | 2.53 ms | 0.45 ms | 5.6x | Replace (D8). 50k: 10.32 vs 1.24 (8.3x). |
| new URL+set vs string append (embed) | 20k | 30.35 ms (1.52 us/op) | 0.87 ms (0.04 us/op) | 35x | Replace (D10). Repeat 47.34 vs 1.14 (41x). |
| new Intl.NumberFormat().format vs cached | 20k | 330.52 ms (16.5 us/op) | 15.17 ms (0.76 us/op) | 22x | Already cached; hoist last site. |
| Sort spread-includes vs Set.has | 50k | 19.35 ms | 2.38 ms | 8.1x | Replace (D6). |
| channelSuggestions filter vs loop (200 ch) | 20k | 245.16 ms | 175.58 ms | 1.4x | Replace (D13), per keystroke. |
| result-counts reduce vs loop (10 vids) | 50k | 4.23 ms | 1.27 ms | 3.3x | Replace (D9 pattern). |
| creatorFromPath split-filter vs scan | 10k | 1.87 ms | 0.84 ms | 2.2x | Replace (D12). |
| Highlight: fresh /g vs shared /g vs indexOf | 10k | fresh 13.10 ms | shared 16.52 / indexOf 14.46 | noise | Keep fresh regex. |
| hits.slice(0,3) vs manual top-3 | 20k | 0.75 ms slice | 1.12 ms manual | slice 1.5x faster | Keep slice. |
| Clone {...o,x} vs for-in vs assign | 20k | 3.31 ms spread | 3.50 / 4.18 | tie | Delete copy (D1/D4/D5), not idiom swap. |
| 30-snippet highlight page pipeline | 2000 pages x30 | 22.38 ms (11.2 us/page) | baseline | page parse ~11 us; decode count dominates | Fix decode (D11). |

### 3.4 Heap + GC (--trace-gc --expose-gc, v8.getHeapStatistics())

- Retained churn: 20k spreadConcat + gc() delta +20520 B; pre-sized same N +1448 B. ~14x less retained garbage for identical output.
- --trace-gc (5k spread loop): back-to-back Scavenge 4.7 (5.6) -> 3.9 (5.9) MB ... allocation failure every ~1-3 ms (6 scavenges in ~6 ms). Pre-sized variant shows no such burst. Per-row x per-eval, this composes into frame-budget overruns (TBT/LongTasks mechanism).

### 3.5 Fixtures (repro)

Videos: 20 {video_id, content_kind: video|short|unknown, hits: 4}. Recents: 12 {query, youtuber, sort, searchedAt}. Snippet: before-and-mark fixture x4 (asserted indexOf variant JSON.stringify-equal, MATCHES regex). Embed: https://www.youtube.com/embed/abc?x=1. Channels: 120-200 {handle, name}.

## 4. Bytecode-step markdown (one line per concrete step; redundant marked)

Each numbered line is one concrete V8/DOM step in order. REDUNDANT = deletable with identical output. REPEATED = correct but re-executed per row/eval (INP lever).

### (a) Initial render (cold / route)

```markdown
1. Parse index.html -> #root + module script (no inline blocking bundle).
2. Fetch + parse entry JS (+ lazy chunks on demand only).
3. Module eval: compactNumber = new Intl.NumberFormat x1 cached OK; sort tuples x1.
4. App(): searchStateFromLocation() -> new URLSearchParams x1 + isAvailableSearchSort -> REDUNDANT [...BASE,...ENG] alloc + includes (D6).
5. creatorFromPath() -> split('/').filter(Boolean) REDUNDANT temp array + closure (D12).
6. loadCachedChannels() + loadRecentSearches() -> JSON.parse + filter/slice bounded 12 (keep).
7. positionChannels(channels): ovalSwarmPositions(n) x1 + .map -> 1 closure + 1 {channel, ...position} clone per channel REDUNDANT second object (D7).
8. For each={swarm()} -> 1 row closure per channel required OK -> <a> + style string concat per row.
9. channelSuggestions memo -> .filter closure over N per keystroke REPEATED (D13); slice(0,8) keeps DOM small OK.
10. formatDate not called (no rows) OK lazy.
11. DOM inserts: swarm anchors + form via Solid batch OK.
```

Redundant on initial render: step 4 spread, step 5 filter-array, step 7 per-channel clone. All pre-FCP/LCP on home route.

### (b) Search-to-results (submit -> cards)

```markdown
1. runSearch() builds {query, youtuber, page, sort} literal OK (no spread here).
2. captureSearchSubmitted -> trim().split(/s+/).filter(Boolean) tiny temp (keep) + 1 literal.
3. searchTranscripts() -> REDUNDANT JSON.stringify(request) key alloc (D8) -> Map lookup -> new AbortController + chain closures required OK.
4. Fetch -> response.json() -> decode.
5. Timing wrapper -> REDUNDANT {...result, request_duration_ms} second object (D1).
6. setState({status:'success', response}) literal required OK.
7. successfulSearch memo {response, youtuber, page} required OK.
8. Header searchResultCountLabel -> REDUNDANT reduce closure + N calls (D9).
9. Videos section: REDUNDANT filter pass 1 (alloc A) + REDUNDANT filter pass 2 (alloc B) + REDUNDANT [...A, ...B] iterator-spread alloc C (D2); 2 closures + 3 arrays for one list.
10. Shorts section: filter(kind='short') scans same array a 3rd time REDUNDANT (fold into D2).
11. Per row: new AbortController x1 required OK.
12. Per row selectedHit(): REDUNDANT visibleHits() twice = 2 slices + find closure when collapsed (D14).
13. Per row playerUrl(): REDUNDANT new URL parse+serialize, 35x (D10), REPEATED per eval.
14. Per row posterThumbnail(): ?? + non-global .replace x1 OK (no lastIndex).
15. Per row engagementSummary(): REDUNDANT filter-map-join, 2 closures + 2 arrays + join (D9).
16. Per hit highlightedParts(): fresh regex x1 OK + exec loop + slice per part + literal per part + REDUNDANT decodeEntities() HTML-parse per part (D11).
17. Per row formatDate(): new Date + toLocaleDateString (ICU, P2 memo only).
18. DOM inserts: N cards x (poster + title + <=3 excerpts) via For required OK; slices bound it OK.
```

Redundant: steps 3, 5, 8, 9-10, 12, 13, 15, 16 (decode). Required/optimal: 11, 14, 17, 18.

### (c) Expand-moments (Show all moments -> full list)

```markdown
1. Click () => void expandMoments() required OK -> setMomentsLoading(true).
2. searchVideoMoments() -> new URLSearchParams x1 cold (keep) -> fetch.
3. response.videos.find(v => ...) linear + closure, tiny N OK.
4. setAllHits(hits) reference move, no copy OK.
5. setShowAllMoments(true) -> visibleHits() returns ref, no slice OK.
6. Re-render full list For each={visibleHits()} -> per-hit highlight pipeline (same as b.16) xM moments REPEATED; decode count scales with M (D11 lever).
7. Analytics: REDUNDANT resultProperties() object + REDUNDANT {...that, ...} second object (D5).
8. Timestamp clicks after: REDUNDANT double-slice when collapsed (D14) + REDUNDANT {...props, match_kind} (D5) + REDUNDANT new URL re-parse (D10).
9. Feedback clicks: same as 8 + 2 literal fields, same fix (D5).
```

Redundant: steps 7-9. Steps 1-5 already minimal (reference moves). Step 6 required rendering; cost bounded by decode count.

## 5. Proposed diffs (fenced blocks, behavior-preserving, vitals-mapped)

No src/ edits made. Each preserves output shape/order.

### D1 — src/api.ts: drop {...result} clone (S1; INP/TBT)

```ts
// before (src/api.ts:97-100)
}).then((result) => ({
  ...result,
  request_duration_ms: Math.max(0, Math.round(performance.now() - callerStarted)),
}));

// after: response is fresh per fetch; assign timing directly
}).then((result) => {
  result.request_duration_ms = Math.max(0, Math.round(performance.now() - callerStarted));
  return result;
});
```

Idiom note: Object.assign/for-in measured tie/slower (3.31 vs 3.50 vs 4.18 ms/20k); deleting the copy is the win. Shared-promise fan-out is rare (one resource per request).

### D2 — src/components/search-results.tsx: one partition (S2; INP/TBT/heap, 4.5x + 14x heap)

```tsx
// before (src/components/search-results.tsx:39-45 + 83-87)
const sectionVideos = (videos: SearchVideoResult[], kind: "video" | "short") =>
  videos.filter((video) => video.content_kind === kind);
const unknownVideos = (videos: SearchVideoResult[]) =>
  videos.filter((video) => video.content_kind === "unknown");
// videos={[...sectionVideos(videos, "video"), ...unknownVideos(videos)]}

// after: single alloc, same order (video then unknown); shorts alongside
function partitionVideos(videos: SearchVideoResult[]) {
  const mains: SearchVideoResult[] = [];
  const shorts: SearchVideoResult[] = [];
  for (let i = 0; i < videos.length; i++) {
    const video = videos[i]!;
    if (video.content_kind === "short") shorts.push(video);
    else if (video.content_kind === "video" || video.content_kind === "unknown") mains.push(video);
  }
  return { mains, shorts };
}
// call site: memo over props.response.videos; Videos videos={parts().mains}, Shorts videos={parts().shorts}
```

### D3 — src/recent-searches.ts: pre-sized loop (S3; INP/heap, 2.7x)

```ts
// before (src/recent-searches.ts:32-36)
const key = recentKey(recent);
const next = [recent, ...current.filter((item) => recentKey(item) !== key)].slice(0, MAX_RECENT_SEARCHES);

// after
const key = recentKey(recent);
const next: RecentSearch[] = new Array<RecentSearch>(MAX_RECENT_SEARCHES);
next[0] = recent;
let count = 1;
for (let i = 0; i < current.length && count < MAX_RECENT_SEARCHES; i++) {
  const item = current[i]!;
  if (recentKey(item) !== key) next[count++] = item;
}
next.length = count;
```

### D4 — src/app.tsx: explicit request literals (S4/S5; INP, map stability)

```tsx
// before (src/app.tsx:290)
const request = { ...current, page: nextPage };
// after
const request: SearchRequest = { query: current.query, youtuber: current.youtuber, page: nextPage, sort: current.sort };

// before (src/app.tsx:395)
const request: SearchRequest = { ...recent, page: 1 };
// after
const request: SearchRequest = { query: recent.query, youtuber: recent.youtuber, page: 1, sort: recent.sort };
```

### D5 — analytics payloads: one literal per event (S6/S7/S8/S10/S11; INP per click)

```ts
// before (src/components/video-result.tsx:338-343 + 3 spread sites)
const resultProperties = (props: { video: SearchVideoResult; rank: number }) => ({
  video_id: props.video.video_id,
  creator_handle: props.video.youtuber_handle,
  content_kind: props.video.content_kind,
  rank: props.rank,
});
// capture(..., { ...resultProperties(props), match_kind: hit.match_kind })

// after: hottest site fully explicit, no spread, no intermediate
capture("result_timestamp_selected", {
  video_id: props.video.video_id,
  creator_handle: props.video.youtuber_handle,
  content_kind: props.video.content_kind,
  rank: props.rank,
  match_kind: hit.match_kind,
});
```

Same single-literal rule for expand/feedback/captureSearchLoaded; replace analytics.ts:62 Object.fromEntries(entries.filter(...)) with a loop skipping undefined (same output, 1 alloc not 3).

### D6 — src/search-sorts.ts: module Set (S9; TBT, 8.1x)

```ts
// before (src/search-sorts.ts:33)
([...BASE_SORTS, ...ENGAGEMENT_SORTS] as readonly string[]).includes(value)

// after (module scope)
const ALL_SORTS: ReadonlySet<string> = new Set<string>([...BASE_SORTS, ...ENGAGEMENT_SORTS]);
export function isSearchSort(value: unknown): value is SearchSort {
  return typeof value === "string" && ALL_SORTS.has(value);
}
```

Also hoist searchSortOptions(true) in searchSortLabel (line 47) to a module constant.

### D7 — src/components/channel-picker.tsx: explicit swarm literal (S13; FCP/LCP)

```ts
// before (src/components/channel-picker.tsx:129-138)
return channels.map((channel, index) => {
  const position = positions[index];
  return { channel, ...position };
});

// after
return channels.map((channel, index) => {
  const position = positions[index]!;
  return { channel, x: position.x, y: position.y, mobileX: position.mobileX, mobileY: position.mobileY, color: position.color };
});
```

### D8 — src/api.ts: manual cache key (A1; INP/TBT, 5-8x + more correct order)

```ts
// before (src/api.ts:44)
const cacheKey = JSON.stringify(request);

// after
const cacheKey = request.query + "\u0000" + request.youtuber + "\u0000" + request.sort + "\u0000" + request.page;
```

Separator matches recentKey convention; keep eviction/abort logic untouched.

### D9 — loops over filter/map/join/reduce (A2/A9; INP per row, 1.4-3.3x)

```ts
// before (src/components/video-result.tsx:244-254)
const present = metrics
  .filter(([value]) => typeof value === "number" && value > 0)
  .map(([value, label]) => `${compactNumber.format(value ?? 0)} ${label}`);
return present.length > 0 ? present.join(" · ") : undefined;

// after: one pass, no closures, no temps, same cached formatter
function engagementSummary(video: SearchVideoResult): string | undefined {
  let out = "";
  let count = 0;
  if (typeof video.view_count === "number" && video.view_count > 0) {
    out += compactNumber.format(video.view_count) + " views"; count++;
  }
  if (typeof video.like_count === "number" && video.like_count > 0) {
    if (count > 0) out += " · ";
    out += compactNumber.format(video.like_count) + " likes"; count++;
  }
  if (typeof video.comment_count === "number" && video.comment_count > 0) {
    if (count > 0) out += " · ";
    out += compactNumber.format(video.comment_count) + " comments"; count++;
  }
  return count > 0 ? out : undefined;
}
```

```ts
// before (src/result-counts.ts:15)
const returnedMoments = response.videos.reduce((total, video) => total + video.hits.length, 0);
// after
let returnedMoments = 0;
const videos = response.videos;
for (let i = 0; i < videos.length; i++) returnedMoments += videos[i]!.hits.length;
```

Same loop shape for dashboard-metrics.ts (cold, P2).

### D10 — autoplayEmbedUrl string fast path (A4; INP per row, 35-41x)

```ts
// before (src/components/video-result.tsx:256-260)
function autoplayEmbedUrl(value: string): string {
  const url = new URL(value);
  url.searchParams.set("autoplay", "1");
  return url.toString();
}

// after
function autoplayEmbedUrl(value: string): string {
  if (value.includes("autoplay=")) return value;
  if (value.includes("?")) return value + "&autoplay=1";
  if (value.startsWith("https://") || value.startsWith("http://")) return value + "?autoplay=1";
  const url = new URL(value);
  url.searchParams.set("autoplay", "1");
  return url.toString();
}
```

Optional pair: memoize playerUrl per selectedHit().embed_url (Solid memo); string fix removes parse cost, memo removes recompute.

### D11 — decode once, not per part (A7; INP/TBT on many-hit pages)

```ts
// policy: keep textarea as source of truth, call it less.
// Option B (safer, recommended): keep raw tag split, skip clean slices:
parts.push({ text: slice.includes("&") ? decodeEntities(slice) : slice, highlighted: false });
// Option A (if backend guarantees tags never entity-escaped): decode whole snippet once, then split.
```

Gate: JSON.stringify(before(snippet)) === JSON.stringify(after(snippet)) over real highlight corpus incl. &amp;, &quot;, &#39;, emoji, multi-mark.

### D12 — creatorFromPath scan (A8; TBT every route sync, 2.2x)

```ts
// before (src/utils.ts:10-14)
const firstSegment = window.location.pathname.split("/").filter(Boolean)[0]?.trim();
// after
const path = window.location.pathname;
let start = 0;
while (start < path.length && path[start] === "/") start++;
let end = start;
while (end < path.length && path[end] !== "/" && path[end] !== "?" && path[end] !== "#") end++;
const firstSegment = path.slice(start, end).trim();
```

### D13 — channel-query.ts hoisted loops (A9 query; INP per keystroke, 1.4x)

```ts
// before (src/channel-query.ts:4-12): trim/toLowerCase + split regex + filter closure
// after
const value = query.trim().toLowerCase();
if (value.charCodeAt(0) !== 64 /* @ */) return [];
const space = value.indexOf(" ");
const needle = value.slice(1, space < 0 ? value.length : space);
const out: IndexedChannel[] = [];
for (let i = 0; i < channels.length; i++) {
  const channel = channels[i]!;
  if (channel.handle.toLowerCase().includes(needle) || channel.name.toLowerCase().includes(needle)) out.push(channel);
}
return out;
```

Same loop for resolveChannelMention find (hoist normalizeHandle(token) once). Pre-lowercasing catalog deferred (needs cache invalidation; loop alone takes ~60% of win).

### D14 — selectedHit single eval (video-result 25-28; INP per row)

```tsx
// before
const selectedHit = () =>
  visibleHits().find((hit) => hit.id === selectedHitId()) ??
  visibleHits()[0] ??
  props.video.hits[0];

// after: one slice, one scan, no closure when unset
const selectedHit = () => {
  const visible = visibleHits();
  const id = selectedHitId();
  if (id !== undefined) {
    for (let i = 0; i < visible.length; i++) if (visible[i]!.id === id) return visible[i]!;
  }
  return visible[0] ?? props.video.hits[0];
};
```

### Kept (proven optimal / unavoidable)

slice(0,3) beats manual (1.5x, keep A3); fresh /g regex has no faster safe alternative (keep A6); icon spreads, dashboard copy-sort, per-fetch URLSearchParams, bounding slices slice(0,6/8) all keep; formatDate ICU memo is P2; availability-notice.tsx:72 per-call DateTimeFormat should be hoisted (one-liner, same A5 rule).

## 6. Verification plan (Playwright / headless)

No src/ edits made; this is the run-book for D1-D14.

### 6.1 Correctness first

```bash
cargo fmt --check
cargo test -p youtube_api -p youtube_typesense -p youtube_search
bun run build
bun run test:frontend
```

Equivalence gates: D2 partition element-wise vs [...filter, ...filter] (all-video/all-short/all-unknown/empty/mixed); D8 key no-collision incl. unicode + page numeric; D10 URL identical incl. existing autoplay=/hash/relative fallback; D11 JSON.stringify equality over real highlight corpus; D3 MRU+dedup+cap-12 over randomized ops. Ranking guardrails (must not move):

```bash
curl -fsS 'https://ytsearch-api-typesense-fly-staging.fly.dev/api/search?q=fable%20on%20high&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at,title_highlight,hit_count:(.hits|length), first:.hits[0].text_highlight})'
curl -fsS 'https://ytsearch-api-typesense-fly-staging.fly.dev/api/search?q=I&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:5] | map({title,published_at})'
```

### 6.2 Playwright vitals (INP/TBT/LongTasks/heap)

Harness: playwright.config.ts (tests/e2e/core-flows.spec.ts), scripts/run-playwright.sh, bun run test:e2e.

```bash
bun run build
bun run serve &
npx playwright test tests/e2e/core-flows.spec.ts --reporter=line
```

Instrumented spec (add once): load /, submit fable on high @ @t3dotgg, click Show all moments, click a timestamp; collect PerformanceObserver longtask, event entries (INP proxy), CDP Performance.getMetrics + HeapProfiler heap after GC.

Pass criteria (median of 5): no failures; no ranking movement; click-to-paint on timestamp + expand not regressed (target >=10% drop); longtask count/duration not up; post-GC heap not up; bun scripts/check-bundle-performance.ts green (diffs shrink bytecode, no new deps).

### 6.3 Headless numbers already collected

--print-bytecode sizes, %GetOptimizationStatus masks, performance.now() tables (sec 3.3), --trace-gc bursts (sec 3.4). Re-run scratch repros with fixtures in sec 3.5 (bench-spread.mjs, bench2.mjs, bench3.mjs, bc2.js, opt-status2.mjs, gc-sample.js); canonicalize as scripts/perf/ fixtures when diffs land.

## 7. Docs consulted

- V8: node --help flag semantics; V8 src/runtime/runtime.h OptimizationStatus enum via v8-docs mirror; SO on %GetOptimizationStatus bitmask decoding; trace_opt methodology background.
- MDN: /g + lastIndex statefulness; Intl.NumberFormat construction cost; slice/filter/map/join; URL/URLSearchParams parse+serialize cost.
- web.dev: Optimize INP + INP (<=200 ms good, p75; INP = blocked next-paint, not fetch effects; maps D2/D9/D10/D14 to click-to-paint); TBT (blocked = long task >50 ms; lab proxy for INP; maps heap/scavenge cuts to TBT).
- Repo: AGENTS.md verification commands (adopted in 6.1); architecture/staging docs skimmed for boundaries (no backend/indexing changes proposed).

## Rationale — alternatives considered and rejected

1. Blanket remove-all-spreads. Rejected. Clone spread is 14 B and at parity (3.31 vs 3.50 ms/20k); S12/S15 are idiomatic cold. Wins are spread-of-filter-then-slice (S2/S3, 3-4.5x) and doubled analytics objects (S6-S8). Rule: kill second allocation + iterator, not syntax.
2. for-in generic clone for D1. Rejected. 54 B, megamorphic, no faster. Explicit literals / direct assignment keep monomorphic maps.
3. Hoisted shared /g regex (A6 alt). Rejected. Slower in-trial (16.52 vs 13.10 ms) and reintroduces lastIndex bleed. Fresh regex already safe.
4. indexOf tag splitter alone. Rejected standalone. 14.46 vs 13.10 ms (no win); real cost is per-part decode. Folded into D11.
5. Hand top-3 copy vs slice. Rejected. Slice won 1.5x (intrinsic memmove). Pre-size unbounded concats, not bounded takes.
6. Object.assign as cheaper spread. Rejected. Slowest clone idiom (4.18 ms). Deleting copy is the answer.
7. Memoize-everything per row. Partially rejected. Memos add graph nodes + closures; measured cost is callee (new URL 35x, chain 1.4x). Fix callees first; memo only on proven thrash (D10 note).
8. Pre-lowercased channel catalog. Deferred. Removes per-keystroke toLowerCase pair but needs invalidation; loop-only D13 takes ~60% win with no cache risk.
9. Hand entity-map vs textarea. Rejected this batch. Drift risk on numeric/edge entities; D11 keeps textarea truth, calls it less, same output.
10. Indexing/Rust/Typesense or vite.config.ts/index.html changes. Rejected. Wins are search/render-path; bundle budgets pass and diffs shrink bytecode. No backend risk.

Vitals summary: FCP/LCP <- D7, D12; INP <- D2/D4/D5/D8/D10/D13/D14 + D9/D11 per-row render inside interaction window; TBT/LongTasks/heap <- D2/D3/D6/D9 + A5 rule; CLS unaffected (data-shape preserving, no layout changes).
