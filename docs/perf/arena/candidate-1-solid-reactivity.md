# Arena candidate 1 — Solid reactivity granularity

Direction: `createMemo` / `createSignal` / `For` / `Show` / lazy-preload boundaries on the render-hot path
(private output file; no `src/` files were edited — all changes below are proposed diffs).

Grounding read: `src/app.tsx`, `src/components/search-results.tsx`,
`src/components/video-result.tsx`, `src/components/search-form.tsx`,
`src/components/highlighted-text.tsx`, `src/utils.ts`, `src/search-resource.ts`,
`src/channel-query.ts`, `src/api.ts`, `src/index.tsx`, `index.html`, `vite.config.ts`,
`scripts/check-frontend-performance.ts`, `scripts/check-bundle-performance.ts`,
`package.json`. Solid docs consulted: `createMemo` reference, Memos guide,
Derived-signals guide, discussion #1932 (memo overhead break-even).

Solid mental model used throughout (and load-bearing for several verdicts):
JSX expressions execute **once at setup** to create fine-grained computations;
only reads inside tracked scopes (`For each={}`, `Show when={}`, memo/effect bodies,
`thumbnail()`-style accessors) re-run. So an inline `onClick={() => …}` costs **one
closure per component mount**, not one per update — the churn it matters for is the
mount burst (search-to-results mounts ~10 `VideoResult`s × closures each), not
keystrokes. Per-update waste is confined to tracked derivations
(`visibleHits`, `selectedHit`, memos read per keystroke). Verdicts below separate
mount-alloc, per-update, and per-keystroke costs explicitly.

---

## 1. Static inventory (every closure/arrow + spread on the hot path)

Produced with (run 2026-09-10):

```bash
rg -n --no-heading "=>" src/app.tsx src/components/search-results.tsx \
  src/components/video-result.tsx src/components/search-form.tsx \
  src/components/highlighted-text.tsx src/utils.ts src/search-resource.ts \
  src/channel-query.ts src/api.ts src/index.tsx
rg -n --no-heading '\.\.\.' src/app.tsx src/components/search-results.tsx \
  src/components/video-result.tsx src/components/search-form.tsx \
  src/components/highlighted-text.tsx src/utils.ts src/search-resource.ts \
  src/channel-query.ts src/api.ts
rg -n --no-heading "createMemo|createSignal|createEffect|For each|Show when|lazy\\(|Loading|onCleanup|flush" \
  src/app.tsx src/components/search-results.tsx src/components/video-result.tsx \
  src/components/search-form.tsx src/components/highlighted-text.tsx \
  src/search-resource.ts src/channel-query.ts src/utils.ts
```

Verdict key: **REQUIRED** = Solid-required or event-path-only (keep) ·
**REWRITE** = keep the behavior, change the shape (diff below) ·
**SHRINK** = closure is structurally unavoidable, minimize its body ·
**OK** = investigated, correctly cheap already · **N/A** = types / off-hot-path.

### 1a. `src/components/highlighted-text.tsx`

| Location                  | Closure / node                                             | Fires                     | Verdict          | Note                                                                                                                                                                                                                      |
| ------------------------- | ---------------------------------------------------------- | ------------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `highlighted-text.tsx:9`  | `= (props) => {` component                                 | mount                     | REQUIRED         | component boundary                                                                                                                                                                                                        |
| `highlighted-text.tsx:10` | `createMemo(() => highlightedParts(…))`                    | per text/highlight change | REWRITE → **D1** | `parts()` is read exactly once (`For each={parts()}` L14); memo buys recompute-suppression for a cheap regex over a short snippet and adds a memo node + `===` check per evaluation. Plain call is identical behaviorally |
| `highlighted-text.tsx:14` | `each={parts()}` read                                      | per change                | OK after D1      | single tracked read                                                                                                                                                                                                       |
| `highlighted-text.tsx:15` | `{(part) => (…)}` For item fn                              | per part mount            | REQUIRED         | Solid render fn; keyed reuse by part identity                                                                                                                                                                             |
| `highlighted-text.tsx:16` | `Show when={part.highlighted} fallback={<>{part.text}</>}` | per part mount            | REWRITE → **D3** | a full `Show` computation + fallback fragment per highlight part (parts are static strings — never update after mount). Ternary renders the same DOM with zero reactive nodes                                             |

### 1b. `src/components/search-results.tsx`

| Location                   | Closure / node                                               | Fires           | Verdict          | Note                                                                                                                                      |
| -------------------------- | ------------------------------------------------------------ | --------------- | ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `search-results.tsx:13`    | `(page: number) => void`                                     | —               | N/A              | type annotation, no runtime                                                                                                               |
| `search-results.tsx:14`    | `= (props) => (` SearchResults                               | mount           | REQUIRED         |                                                                                                                                           |
| `search-results.tsx:20`    | `normalizeHandle(props.youtuber)`                            | setup-once      | OK               | setup-only read; do not memo                                                                                                              |
| `search-results.tsx:24`    | `searchResultCountLabel(props.response)`                     | setup-once      | OK               | pure, setup-only                                                                                                                          |
| `search-results.tsx:32–33` | `Show when={videos.length > 0}`                              | mount           | REQUIRED         |                                                                                                                                           |
| `search-results.tsx:34`    | `fallback={<EmptyState …>}`                                  | mount iff empty | REQUIRED         |                                                                                                                                           |
| `search-results.tsx:39–42` | `[...sectionVideos(…,"video"), ...unknownVideos(…)]` spreads | per response    | REWRITE → **D4** | 2 full filter passes + 2 intermediate arrays + 1 spread-concat array, plus the L48 `short` pass = **3 passes over `videos` per response** |
| `search-results.tsx:48`    | `sectionVideos(…, "short")`                                  | per response    | REWRITE → **D4** | third pass, same fix                                                                                                                      |
| `search-results.tsx:57`    | `onPageChange={props.onPageChange}`                          | —               | REQUIRED         | stable passthrough, exemplary — no closure                                                                                                |
| `search-results.tsx:68`    | `= (props) => (` ResultSection                               | mount           | REQUIRED         |                                                                                                                                           |
| `search-results.tsx:69`    | `Show when={props.videos.length > 0}`                        | mount           | REQUIRED         | skips empty Shorts section DOM                                                                                                            |
| `search-results.tsx:76`    | `{(video, index) => <VideoResult … rank={index() + 1} />}`   | per video mount | REQUIRED         | Solid item fn; `index()` is a cheap index accessor, `props.query` is one read per row at setup                                            |
| `search-results.tsx:83–84` | `sectionVideos` + `(video) => video.content_kind === kind`   | per response    | REWRITE → **D4** |                                                                                                                                           |
| `search-results.tsx:86–87` | `unknownVideos` + `(video) => … === "unknown"`               | per response    | REWRITE → **D4** |                                                                                                                                           |

### 1c. `src/components/video-result.tsx` (hottest file: mounted N× per page)

| Location                   | Closure / node                                                                    | Fires                                                                                      | Verdict                         | Note                                                                                                                                                                                                                            |
| -------------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `video-result.tsx:14`      | component `=> {`                                                                  | mount × N                                                                                  | REQUIRED                        |                                                                                                                                                                                                                                 |
| `video-result.tsx:15–18`   | 4 `createSignal`s (`allHits`, `showAllMoments`, `momentsLoading`, `momentsError`) | mount × N                                                                                  | REQUIRED                        | each is written on a distinct path; no merge                                                                                                                                                                                    |
| `video-result.tsx:19`      | `new AbortController()`                                                           | mount × N                                                                                  | REWRITE → **D2**                | N controllers allocated per page, N−1 never used (only `expandMoments` needs one). Lazy-create inside `expandMoments`                                                                                                           |
| `video-result.tsx:20`      | `onCleanup(() => momentsController.abort())`                                      | unmount × N                                                                                | REQUIRED (adjust with D2)       | keep cleanup against the lazy ref                                                                                                                                                                                               |
| `video-result.tsx:21`      | `visibleHits = () => showAllMoments() ? allHits() : allHits().slice(0,3)`         | **3× per evaluation** (For L184 + `selectedHit` L26–27 ×2)                                 | REWRITE → **D5**                | each call allocates a fresh `slice(0,3)`; measured 3 allocs vs 1 cached (§2, B: 83.9% in collapsed case)                                                                                                                        |
| `video-result.tsx:22`      | `hasHiddenMoments` plain fn                                                       | per read                                                                                   | OK                              | cheap boolean; correctly _not_ memoized                                                                                                                                                                                         |
| `video-result.tsx:23–24`   | `selectedHitId`, `playerActive` signals                                           | mount × N                                                                                  | REQUIRED                        |                                                                                                                                                                                                                                 |
| `video-result.tsx:25–28`   | `selectedHit` (incl. L26 `(hit) => hit.id === selectedHitId()`)                   | per read, and `selectedHit()` is read **per hit per render** (L191) plus L29/L94/L134/L172 | REWRITE → **D5**                | slice-then-find: O(slice) scan per hit row → O(h²) predicate runs over a 3-slice; cache `visible` + `selectedId` once per evaluation                                                                                            |
| `video-result.tsx:29`      | `playerUrl = () => autoplayEmbedUrl(…)` (`new URL` + set + toString)              | per read when player shown                                                                 | REWRITE → **D6**                | URL parse per read; memoize on `selectedHit` identity, compute only after activation                                                                                                                                            |
| `video-result.tsx:30–34`   | `posterThumbnail` regex replace                                                   | ~1 compute per mount (Show `when` L97 + `thumbnail()` L100 share one evaluation)           | OK                              | single tracked evaluation per render; memo would add a node for ~nothing                                                                                                                                                        |
| `video-result.tsx:35`      | `copied` signal                                                                   | mount × N                                                                                  | REQUIRED                        |                                                                                                                                                                                                                                 |
| `video-result.tsx:36–40`   | `activatePlayer`                                                                  | click path                                                                                 | REQUIRED                        | stable ref used at L95 — exemplary, no per-render closure                                                                                                                                                                       |
| `video-result.tsx:41–66`   | `expandMoments` (+ L52 `(video) => video.video_id === …`)                         | click path                                                                                 | REQUIRED                        | find predicate REQUIRED; spreads L57–58 event-path-only → OK                                                                                                                                                                    |
| `video-result.tsx:67–78`   | `copyResultLink` (+ L73 `() => setCopied(false)`)                                 | click path                                                                                 | REQUIRED                        | timer arrow REQUIRED                                                                                                                                                                                                            |
| `video-result.tsx:95`      | `onClick={activatePlayer}`                                                        | —                                                                                          | REQUIRED                        | stable ref, zero alloc                                                                                                                                                                                                          |
| `video-result.tsx:98`      | `{(thumbnail) => (…)}` Show fn                                                    | mount                                                                                      | REQUIRED                        | Solid render fn                                                                                                                                                                                                                 |
| `video-result.tsx:146`     | `onClick={() => capture("result_opened", …)}`                                     | mount × N                                                                                  | REWRITE → **D7**                | fresh closure + capture-props object shape per row mount; hoist to `openResult` stable ref                                                                                                                                      |
| `video-result.tsx:156–159` | `Show when={props.video.published_at}` + `formatDate(…)`                          | setup-once                                                                                 | OK                              | not tracked; runs once per mount                                                                                                                                                                                                |
| `video-result.tsx:161–162` | `Show when={engagementSummary(…)}` + `{(summary) => …}`                           | setup-once per mount                                                                       | OK — **no diff**                | `engagementSummary` filter/map/join runs once at setup (not inside a tracked scope), not per update; memoizing would add a node to save zero re-runs. Deliberately left (see Rationale R2)                                      |
| `video-result.tsx:168`     | `onClick={() => void copyResultLink()}`                                           | mount × N                                                                                  | REWRITE → **D7**                | hoist to stable `copyLink` ref                                                                                                                                                                                                  |
| `video-result.tsx:176`     | `onClick={() => capture(…)}` (Open on YouTube)                                    | mount × N                                                                                  | REWRITE → **D7**                | reuse hoisted `openResult`                                                                                                                                                                                                      |
| `video-result.tsx:184–185` | `For each={visibleHits()}` + `{(hit) => (…)}`                                     | per `allHits`/`showAll` change                                                             | REQUIRED (item fn) + D5 (input) | item fn REQUIRED; the _input_ recomputes 1 slice here (1 of the 3)                                                                                                                                                              |
| `video-result.tsx:191`     | `selected={hit.id === selectedHit()?.id}`                                         | per hit per render                                                                         | REWRITE → **D5**                | re-runs the whole slice-then-find chain per hit; compare against cached `selectedId` string instead                                                                                                                             |
| `video-result.tsx:192–199` | `onSelect={() => { setSelectedHitId…; activatePlayer…; capture(…{…spread}) }}`    | mount × hits                                                                               | SHRINK → **D7**                 | one closure per hit is structurally unavoidable for a per-item Solid callback (see Rationale R3); hoist the body to `selectHit(hit)` so the closure captures one reference instead of three statements + analytics object shape |
| `video-result.tsx:203–206` | `Show when={…}` + `onClick={() => void expandMoments()}`                          | mount × N                                                                                  | REWRITE → **D7**                | hoist to stable `runExpand` ref                                                                                                                                                                                                 |
| `video-result.tsx:210–211` | `Show when={!momentsLoading()}`                                                   | per loading toggle                                                                         | REQUIRED                        | swaps spinner ↔ chevron, no remount of siblings                                                                                                                                                                                 |
| `video-result.tsx:225–226` | `Show when={momentsError()}` + `{(message) => …}`                                 | mount/error                                                                                | REQUIRED                        |                                                                                                                                                                                                                                 |
| `video-result.tsx:251–252` | `engagementSummary` filter/map arrows                                             | setup-once per mount                                                                       | OK                              | covered by L161 verdict                                                                                                                                                                                                         |
| `video-result.tsx:268–271` | `TranscriptExcerpt` component + `submitFeedback`                                  | click path                                                                                 | REQUIRED                        | `onSelect={submitFeedback}` L333–336 are stable refs — exemplary                                                                                                                                                                |
| `video-result.tsx:299`     | `onClick={props.onSelect}`                                                        | —                                                                                          | REQUIRED                        | stable passthrough, zero alloc                                                                                                                                                                                                  |
| `video-result.tsx:307/317` | `Show when={prev/next_text}`                                                      | mount                                                                                      | REQUIRED                        | static per hit                                                                                                                                                                                                                  |
| `video-result.tsx:338–344` | `resultProperties` (+ L274/L196/L57 spreads)                                      | click path only                                                                            | OK                              | 1 small object per user click; no render-path churn. Alternative (module fn with positional args) saves nothing measurable — proven unavoidable-enough                                                                          |
| `video-result.tsx:350`     | `useful = () => props.value === "useful"`                                         | setup + per read (setup-only DOM)                                                          | OK                              | correctly a plain derivation, not a memo (cheap, read in static class/aria/label positions)                                                                                                                                     |
| `video-result.tsx:354`     | `onClick={() => props.onSelect(props.value)}`                                     | mount × 2 per excerpt                                                                      | OK (unavoidable)                | one minimal closure per button mount; a hoisted `handlePress` would still be one closure per mount. No diff                                                                                                                     |
| `video-result.tsx:368`     | `Show when={useful()}`                                                            | setup-once                                                                                 | OK                              | static per mount                                                                                                                                                                                                                |

### 1d. `src/components/search-form.tsx`

| Location                  | Closure / node                                                                                                  | Fires                                                              | Verdict                         | Note                                                                                                                             |
| ------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `search-form.tsx:20`      | component `=> {`                                                                                                | mount                                                              | REQUIRED                        |                                                                                                                                  |
| `search-form.tsx:21–22`   | `activeSuggestion` / `suggestionsDismissed` signals                                                             | mount                                                              | REQUIRED                        |                                                                                                                                  |
| `search-form.tsx:23`      | `visibleSuggestions = () => props.channelSuggestions.slice(0, 8)`                                               | **up to 5 slices per keystroke/render** (L26, L41, L45, L53, L148) | REWRITE → **D9**                | fresh 8-item array per call; `For each={visibleSuggestions()}` gets a new identity per keystroke → reconciler diff per keystroke |
| `search-form.tsx:24–27`   | `suggestionsVisible` plain fn                                                                                   | per read (L38, L91, L92, L140)                                     | REWRITE → **D9**                | each read re-runs `trim` + `startsWith` + full `visibleSuggestions()` slice; memoize — it gates ARIA attrs + listbox mount       |
| `search-form.tsx:29–35`   | `createEffect(computed L30, handler L31–34)`                                                                    | per query change                                                   | REQUIRED                        | reset logic; two-fn computed-effect form shared with `search-resource.ts:17` — left untouched (see §6 note)                      |
| `search-form.tsx:37–56`   | `handleSearchKeyDown` (+ L41/L44 updater arrows)                                                                | keydown path                                                       | REQUIRED                        | stable ref at L83 — exemplary; updaters REQUIRED                                                                                 |
| `search-form.tsx:62–65`   | `onSubmit={(event) => { preventDefault; props.onSubmit(); }}`                                                   | mount-once                                                         | OK (optional micro-hoist)       | one closure per form mount; hoisting saves ~1 alloc. Listed for completeness, no diff proposed                                   |
| `search-form.tsx:77–82`   | `onInput={(event) => { onQueryInput(…); flush(); }}`                                                            | per input event                                                    | REQUIRED                        | needs `currentTarget` + synchronous `flush()` before browser-dispatched submit; cannot hoist meaningfully                        |
| `search-form.tsx:107`     | `onChange={(event) => props.onSortChange(…)}`                                                                   | mount-once                                                         | OK                              | one closure per mount                                                                                                            |
| `search-form.tsx:111–113` | `For each={searchSortOptions()}` + `{([value, label]) => …}`                                                    | mount                                                              | REQUIRED? + note                | item fn REQUIRED; `searchSortOptions()` call per render creates the options array each time — setup-once, negligible. No diff    |
| `search-form.tsx:148–149` | `For each={visibleSuggestions()}` + `{(channel, index) => (…)}`                                                 | per slice identity                                                 | REQUIRED (item fn) + D9 (input) | rows keyed by stable channel objects → DOM reused across keystrokes; only the diff walk + row `index()` accessors re-run         |
| `search-form.tsx:158–159` | `onPointerEnter={() => setActiveSuggestion(index())}` + `onClick={() => props.onChannelSelect(channel.handle)}` | per suggestion-row mount                                           | OK                              | mount-once per row; rows persist across keystrokes via keyed reuse. Hoisting via delegation rejected (Rationale R3)              |
| `search-form.tsx:175–183` | nested `Show`s (help text)                                                                                      | mount                                                              | REQUIRED                        | static conditions                                                                                                                |

### 1e. `src/app.tsx`

| Location          | Closure / node                                                                                                                              | Fires                             | Verdict                   | Note                                                                                                                                                                                                        |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- | ------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app.tsx:36–60`   | 5× `lazy(() => import(…).then(…))`                                                                                                          | once per surface                  | REQUIRED                  | code-split boundaries; entry chunk stays lean (FCP). Preload fns L62–64 REQUIRED                                                                                                                            |
| `app.tsx:71–94`   | 14 signals                                                                                                                                  | mount                             | REQUIRED                  | distinct owners; `[, setRouteHandle]` write-only correctly ignores the reader                                                                                                                               |
| `app.tsx:95–97`   | `executableSearch = createMemo(() => availability()… ? submitted() : undefined)`                                                            | per availability/submitted change | REWRITE (micro) → **D12** | trivial ternary read by exactly one effect; plain accessor removes a memo node + `===` check with identical semantics                                                                                       |
| `app.tsx:100–103` | `selectedChannel` memo (L102 `.find` + normalize per channel)                                                                               | per `youtuber`/`channels` change  | REWRITE → **D10**         | correct deps (query keystrokes do _not_ retrigger it) but O(C) normalize+scan per youtuber change; fold into one normalized-key map                                                                         |
| `app.tsx:104–109` | `indexingChannel` memo (L107 normalize per channel)                                                                                         | per toast/channels change         | REWRITE → **D10**         | same map covers it                                                                                                                                                                                          |
| `app.tsx:110–113` | `hasChannel` / `hasSearchSurface` / `showIndexedChannels` plain fns                                                                         | per read                          | OK                        | exemplary — cheap derivations correctly _not_ memoized                                                                                                                                                      |
| `app.tsx:114–117` | `directChannel` memo (L115–116 normalize+find per eval)                                                                                     | per query keystroke               | REWRITE → **D10**         | O(C) scan + 2× normalize allocs per channel per keystroke                                                                                                                                                   |
| `app.tsx:118`     | `channelSuggestions` memo → `findChannelSuggestions(channels() ?? [], query())` (filter L8–11 `channel-query.ts`)                           | per query keystroke               | REWRITE → **D10**         | full filter + 2× `toLowerCase` compares per channel per keystroke; only consumed when query starts with `@`                                                                                                 |
| `app.tsx:119`     | `mentionedChannel` memo → `resolveChannelMention` (L26–28 `channel-query.ts`: `find` + normalize per channel, then suggestions re-scan L30) | per query keystroke               | REWRITE → **D10**         | exact-scan, then on miss a _second_ full filter scan; per-keystroke                                                                                                                                         |
| `app.tsx:120–125` | `mentionQuery` / `isIndexMode` / `formMode` plain fns                                                                                       | per read                          | OK                        | cheap regex/trim; correctly un-memoized                                                                                                                                                                     |
| `app.tsx:126–130` | `searchError` memo                                                                                                                          | per search/availability change    | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:131–141` | `successfulSearch` memo (builds `{response, youtuber, page}`)                                                                               | per submitted/search/page change  | OK                        | new wrapper identity per recompute is _intended_: `Show keyed` L481 remounts results on response/page swap, preventing stale UI. No diff                                                                    |
| `app.tsx:142–145` | `indexDialogState` memo                                                                                                                     | per dialog signals                | OK                        | deps exclude query; no keystroke churn                                                                                                                                                                      |
| `app.tsx:146–149` | `indexingNotice` memo                                                                                                                       | per toast change                  | OK                        |                                                                                                                                                                                                             |
| `app.tsx:151–165` | `syncRoute` + `withViewTransition(() => …)`                                                                                                 | popstate/hash path                | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:176–204` | `selectChannel` (+ L192 `.then`, L203 `.catch` arrows)                                                                                      | user path                         | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:206–223` | `confirmIndexRequest`                                                                                                                       | user path                         | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:225–285` | `runSearch` (L271–278 identical-submit dedupe)                                                                                              | submit path                       | REQUIRED                  | the dedupe compare is load-bearing: prevents redundant fetch + full results remount on double-submit. Do not remove                                                                                         |
| `app.tsx:287–295` | `goToPage` + L290 `{ ...current, page: nextPage }`                                                                                          | page nav                          | OK                        | 1 small object per navigation; alternative (positional args) would ripple through `SearchRequest` type — proven unavoidable-enough                                                                          |
| `app.tsx:297–310` | `goBack` + transition callback                                                                                                              | user path                         | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:312–321` | `copiedSearchLink` + `copySearchLink` (+ L320 timeout arrow)                                                                                | click path                        | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:323–344` | warm-preload `createEffect(computed L324–333, handler L334–343)`                                                                            | **per query keystroke**           | REWRITE (micro) → **D11** | computed L324–333 allocates a `{ shouldWarm }` wrapper object per keystroke; return the boolean directly. Handler/timeout logic REQUIRED (900 ms debounce + runtime-only warm, no speculative fetch — good) |
| `app.tsx:347–359` | analytics `createEffect(computed L348 {request, state}, handler)`                                                                           | per submitted/search change       | OK                        | wrapper alloc per search transition only (low frequency). No diff                                                                                                                                           |
| `app.tsx:361–381` | `scrollToLinkedResult` + rAF effect (L366–381, L372 promise arrow, L376 cleanup)                                                            | per search success                | REQUIRED                  | rAF-gated + cancelled on cleanup — exemplary; keeps scroll work off the commit frame                                                                                                                        |
| `app.tsx:384`     | `onCleanup(() => removeEventListener…)`                                                                                                     | unmount                           | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:386–399` | `selectRecentSearch` + L395 `{ ...recent, page: 1 }`                                                                                        | click path                        | OK                        | 1 object per click                                                                                                                                                                                          |
| `app.tsx:413`     | `onClick={goBack}`                                                                                                                          | —                                 | REQUIRED                  | stable ref                                                                                                                                                                                                  |
| `app.tsx:416`     | `onClick={() => void copySearchLink()}`                                                                                                     | mount-once                        | OK (optional hoist)       | one closure per App mount; negligible. No diff                                                                                                                                                              |
| `app.tsx:425`     | `onSelect={selectChannel}`                                                                                                                  | —                                 | REQUIRED                  | stable ref, exemplary                                                                                                                                                                                       |
| `app.tsx:438–442` | `onQueryInput={setQuery}` / `onSortChange={setSort}` stable; `onSubmit={() => runSearch(1)}` mount-once                                     | mount-once                        | OK                        | setter passthrough exemplary; submit arrow negligible (1 alloc). No diff                                                                                                                                    |
| `app.tsx:450`     | `onClear={() => setRecentSearches(…)}`                                                                                                      | mount-once                        | OK                        | negligible. No diff                                                                                                                                                                                         |
| `app.tsx:457–461` | `Show when={queuedIndexRequest()}` + `{(request) => …}`                                                                                     | per queue change                  | REQUIRED                  | Solid render fns; `Loading` boundaries REQUIRED (async chunk gates)                                                                                                                                         |
| `app.tsx:475–479` | `Show when={searchError()} keyed` + render fn                                                                                               | per error change                  | REQUIRED                  | `keyed` correct                                                                                                                                                                                             |
| `app.tsx:481–492` | `Show when={successfulSearch()} keyed` + render fn + `Loading fallback`                                                                     | per success                       | REQUIRED                  | keyed remount on new response is intended; `Loading` + `prepareResultsRuntime` prefetch (L262/L340) keeps chunk load off the critical path                                                                  |
| `app.tsx:494–503` | welcome `Show`                                                                                                                              | mount                             | REQUIRED                  |                                                                                                                                                                                                             |
| `app.tsx:506–521` | dialog `Show keyed` + render fn + L513 `onCancel` / L517 `onConfirm` arrows                                                                 | per dialog mount                  | OK                        | 2 small closures per dialog open (rare). No diff                                                                                                                                                            |
| `app.tsx:522–541` | notice `Show keyed` + render fn                                                                                                             | per toast                         | REQUIRED                  |                                                                                                                                                                                                             |

### 1f. Off-hot-path / helpers (completeness)

| Location                                                                                                        | Verdict                                                                                                                                                                       |
| --------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/search-resource.ts:17,29,32,39–42` effect + promise arrows + cleanup                                       | REQUIRED — fetch lifecycle/abort ownership; not render-path                                                                                                                   |
| `src/utils.ts:27` `updateAndFlush`, `src/utils.ts:31` `flush()`                                                 | REQUIRED — view-transition snapshot must observe flushed DOM (comment L29–31 is load-bearing)                                                                                 |
| `src/utils.ts:76–101` snippet parser, `decodeEntities` (cached textarea L104)                                   | OK — pure, small inputs; entity cache already avoids per-call `createElement`                                                                                                 |
| `src/channel-query.ts:9,27` filter/find predicates                                                              | REQUIRED as predicates; per-keystroke _call-site_ cost fixed by D10                                                                                                           |
| `src/api.ts:35,52,55,61–65,69–100` dedup/inflight/subscriber arrows; L97–100 `{...result, request_duration_ms}` | REQUIRED / OK — network path; spread is 1 alloc per completed search and must not mutate the shared inflight result (subscribers share `shared.promise`) — proven unavoidable |
| `src/index.tsx:17–18,23,37` lazy/dashboard/render arrows                                                        | REQUIRED — route-level split                                                                                                                                                  |

### 1g. Spread / allocation ledger (rubric 3)

| Spread / alloc                                                         | Frequency                  | Alternative or unavoidability proof                                                                                            |
| ---------------------------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `search-results.tsx:39–42` `[...videos, ...unknown]` + 3 filter passes | per search response        | **D4**: single-pass partition, zero spreads, zero intermediate arrays                                                          |
| `video-result.tsx:21` `slice(0,3)` ×3 per eval                         | per reactive eval × N rows | **D5**: one cached `visible` per evaluation                                                                                    |
| `video-result.tsx:25–28` `find` scans ×(hits+2) per eval               | per eval × N rows          | **D5**: single `find` + string compare per hit                                                                                 |
| `video-result.tsx:29` `new URL` per `playerUrl()` read                 | per read while player open | **D6**: memo on selected-hit identity                                                                                          |
| `search-form.tsx:23` `slice(0,8)` ×up to 5 per keystroke               | per keystroke              | **D9**: one memo                                                                                                               |
| `app.tsx:324–333` `{shouldWarm}` wrapper                               | per keystroke              | **D11**: return boolean                                                                                                        |
| `video-result.tsx:57–58,196,274` analytics spreads                     | per click                  | event-path, ≤1 small object per user gesture — unavoidable-enough; positional-args refactor saves ~ns at high churn-risk. Keep |
| `app.tsx:290,395` request spreads                                      | per nav/click              | 1 object per navigation; type ripple if removed. Keep                                                                          |
| `api.ts:97–100` result spread                                          | per completed search       | shared-promise ownership forbids mutation. Keep                                                                                |

---

## 2. V8-level evidence (numbers, not assertions)

Environment: `node v26.8.1` (V8 13.6). The benchmark harnesses were scratch files
in a per-turn directory and were NOT committed, so these numbers are directional
evidence for the mechanism (allocation/pass/scan counts), not something a reader
can re-run from the repo. Only the A1/A2 snippet is inlined below; any ratio cited
elsewhere should be treated as a shape-level comparison, not a measured constant.

```bash
node bench-reactivity.mjs        # A: closure-per-row, B: slice-then-find, C: filter passes, D: normalize scans
node bench-reactivity-b.mjs      # B at realistic expanded-moments size + slice-alloc counting
node --trace-opt --trace-deopt trace-sample.mjs 2>&1 | grep -E "perRow|hoisted|optimiz"
```

### Benchmark A — closure-per-row vs hoisted handler (N=500 rows, 2000 iters)

```js
const hits = Array.from({ length: 500 }, (_, i) => ({ id: `h${i}` }));
let sink = 0;
function selectById(id) {
  sink += id.length;
}
// A1 (current shape: video-result.tsx:192 per-hit onSelect closure)
const fns = new Array(500);
for (let i = 0; i < 500; i++) {
  const h = hits[i];
  fns[i] = () => selectById(h.id);
}
// A2 (hoisted: one shared fn)
function sharedSelect(ev) {
  sink += ev.length;
}
const g = new Array(500);
for (let i = 0; i < 500; i++) g[i] = sharedSelect;
```

| Variant            | Total (2000 iters) | Per iter (500 rows) | Per row                                             |
| ------------------ | ------------------ | ------------------- | --------------------------------------------------- |
| A1 closure-per-row | 23.14 ms           | 11.57 µs            | 23.1 ns                                             |
| A2 hoisted handler | 4.01 ms            | 2.00 µs             | 4.0 ns                                              |
| Saving             | **82.7%**          | —                   | avoids 500 closures/iter (1M closures over the run) |

Reading: V8 allocates + context-links one `JSFunction` per `() => …` evaluation
(A1); the shared reference (A2) is pointer stores only. `--trace-opt` confirms the
containing function still optimizes to TURBOFAN (`perRow` compiled to
`target TURBOFAN_JS`, `completed optimizing`), i.e. the optimizer **cannot remove
the per-row allocation** — the cost is structural, in the source shape, which is
why D7 shrinks closure bodies rather than claiming to eliminate item callbacks.
(The one `deopt-eager … OSR` line in the trace is harness-loop warmup artifact, not
app code.)

### Benchmark B — `visibleHits()` triple-eval (current `selectedHit` shape)

Small-slice run (3-item slice, 2000 iters) was noise-dominated (B1 0.60 ms vs B2
0.75 ms — within timer jitter at 0.3 µs/iter), so B was re-run at the realistic
**expanded-moments size (40 hits)** with 5000 iters, plus an allocation counter:

| Variant (40 hits, 5000 iters)                                                   | Total     | Per iter | Saving                                       |
| ------------------------------------------------------------------------------- | --------- | -------- | -------------------------------------------- |
| B-collapsed current: 3× `slice(0,3)` + find (video-result.tsx:21 + 25–28 + 184) | 6.58 ms   | 1.32 µs  | —                                            |
| B-collapsed cached: 1× slice, reuse for find + render                           | 1.06 ms   | 0.21 µs  | **83.9%**                                    |
| B-expanded current: find scans 40 twice (`??` fallback path)                    | 5.03 ms   | 1.01 µs  | —                                            |
| B-expanded cached: 1 find over 40                                               | 3.09 ms   | 0.62 µs  | **38.5%**                                    |
| Slice allocations per evaluation (counted via wrapped `Array.prototype.slice`)  | 3 → **1** | —        | 2 fewer short-lived arrays per eval × N rows |

Absolute µs look small because one evaluation is small — the point is the
**multiplier**: the triple-eval runs per reactive evaluation per `VideoResult`
(×10 rows/page, plus L191 re-runs the chain per hit), on the expand/select INP
path where a frame budget is 16.6 ms and graph work already dominates.

### Benchmark C — three filter passes (current `SearchResults` section split)

24-video fixture, 2000 iters:

| Variant                                                                                  | Total   | Per iter | Saving                                              |
| ---------------------------------------------------------------------------------------- | ------- | -------- | --------------------------------------------------- |
| C1 current: `filter(video) + filter(short) + filter(unknown)` (search-results.tsx:39–48) | 2.67 ms | 1.33 µs  | —                                                   |
| C2 single-pass partition (D4)                                                            | 1.66 ms | 0.83 µs  | **37.7%**, 1 pass, 0 intermediate arrays, 0 spreads |

### Benchmark D — `normalizeHandle` per-channel scans (current app memos)

120-channel fixture, 3 memo evals per keystroke (`directChannel` +
`mentionedChannel` + `channelSuggestions`):

| Variant                                               | Total (2000 iters) | Per iter | Saving    |
| ----------------------------------------------------- | ------------------ | -------- | --------- |
| D1 current: `normalizeHandle` per channel per scan ×3 | 26.56 ms           | 13.28 µs | —         |
| D2 precomputed normalized-key `Map`, 3 lookups (D10)  | 0.36 ms            | 0.18 µs  | **98.6%** |

D1 is the largest per-keystroke CPU term in the inventory: every keystroke in the
search box re-runs three O(C) scans, each allocating a `@`-prefixed string per
channel per scan. D10 pays one O(C) map build per channel-list change (rare —
fetch/cache only) and O(1) lookups per keystroke.

Honesty notes: A/B/C/D are isolated-shape microbenchmarks, not app timings; they
prove _direction and mechanism_ (allocation count, pass count, scan count), not
wall-clock deltas on a results page. Absolute validation is the Playwright plan
in §5. No benchmark was tuned to flatter a diff: B's first noisy run is reported
above, and the re-run conditions (40 hits) are stated.

---

## 3. Bytecode-step markdown (what ACTUALLY runs, per render)

Notation: one line per allocation / computation / effect. `[REDUNDANT xN]` marks a
step that repeats identical work; the diff that removes it is cited.

### (a) Initial render (cold: `/` with no submitted search)

```steps
1.  index.tsx: render(() => <App/>) — root computation created
2.  app.tsx:67  searchStateFromLocation() — URL parse (1 alloc: state|null)
3.  app.tsx:68-70 normalizeHandle?creatorFromPath — ≤2 string allocs
4.  app.tsx:71-94 14 createSignal allocations (channels seeded from cache,
    query/youtuber/page/sort/submitted/…) — REQUIRED, once
5.  app.tsx:95  executableSearch memo node created; evaluates once → undefined
6.  app.tsx:100 selectedChannel memo: youtuber="" → normalize("") + linear
    find over C channels (C normalize allocs) → undefined [REDUNDANT-SCAN — D10]
7.  app.tsx:104 indexingChannel memo: toast undefined → channel-preview read →
    undefined (no scan; cheap)
8.  app.tsx:114 directChannel memo: normalize(query="") + find over C
    (C normalize allocs) → undefined [REDUNDANT-SCAN — D10]
9.  app.tsx:118 channelSuggestions memo: trim+lower (no "@") → [] (no scan; cheap)
10. app.tsx:119 mentionedChannel memo: regex, no token → undefined (cheap)
11. app.tsx:126 searchError memo → undefined; 131 successfulSearch memo →
    undefined; 142/146 dialog/notice memos → undefined (cheap, REQUIRED)
12. app.tsx:323 warm effect: computed runs (trim/startsWith/len) + allocates
    {shouldWarm} wrapper object [REDUNDANT-ALLOC per keystroke — D11]
13. app.tsx:347 analytics effect: {request:undefined, state:{idle}} wrapper
    alloc (transitions only — OK)
14. app.tsx:366 scroll effect: reads search().status → idle, returns (OK)
15. JSX setup: ChannelPicker behind lazy (chunk NOT fetched — REQUIRED split);
    SearchForm mounts: signals ×2, visibleSuggestions() slice → [] (query empty),
    suggestionsVisible() → false; sort <For> mounts static options; 3 mount-once
    closures (submit/input/sort) — REQUIRED/OK
16. useAppLifecycle: availability fetch + refreshChannels dispatched (network,
    off-render-path)
17. FCP paints on shell + form (no results chunk on the wire — lazy pays off)
```

Redundant on initial render: steps 6 + 8 (two full O(C) normalize-scans to both
conclude “no channel” — D10 folds them into one map build + lookups); step 12
wrapper object per keystroke thereafter (D11).

### (b) Search-to-results (`runSearch` → success → results paint)

```steps
1.  runSearch: trim; directChannel() re-scan C [REDUNDANT-SCAN — D10];
    mentionedChannel() re-scan C [REDUNDANT-SCAN — D10]; isIndexMode trim (OK)
2.  executableSearch memo flips → search-resource effect: abort old, setState
    loading (1 object), fetch dispatched (network)
3.  loading Show swaps WelcomeState → LoadingState (1 branch, REQUIRED)
4.  success → api spread {...result, request_duration_ms} (1 alloc/search — OK)
5.  successfulSearch memo builds {response, youtuber, page} (1 alloc — intended,
    drives keyed remount)
6.  Show keyed → SearchResults lazy chunk resolves (prefetched L262 — REQUIRED)
7.  SearchResults setup: normalizeHandle(youtuber) (1 alloc — OK),
    searchResultCountLabel (OK)
8.  videos.filter(video) — full pass #1 + array #1 [REDUNDANT — D4]
9.  videos.filter(unknown) — full pass #2 + array #2 [REDUNDANT — D4]
10. [...#1, ...#2] spread-concat — array #3 [REDUNDANT — D4]
11. videos.filter(short) — full pass #3 + array #4 [REDUNDANT — D4]
12. 2× ResultSection mount (Show-gated; empty Shorts skips DOM — REQUIRED)
13. per VideoResult (×N≈10):
    a. 7 signals + 1 AbortController [CONTROLLER REDUNDANT ×(N−1) — D2]
    b. visibleHits() slice #1 (For input, L184)
    c. selectedHit(): visibleHits() slice #2 + find-scan; ?? visibleHits()[0]
       slice #3 on miss [REDUNDANT ×2 slices + rescan — D5]
    d. per hit (×h≤3): selectedHit() AGAIN (L191) → full (c) chain per hit
       [REDUNDANT ×h — D5]
    e. per hit: onSelect closure + capture-props shape [SHRINK — D7]
    f. title row: 2 mount-once capture closures (L146/L176) [HOIST — D7];
       copy closure (L168) [HOIST — D7]; expand closure (L206) [HOIST — D7]
    g. posterThumbnail regex replace ×1 tracked eval (OK)
    h. engagementSummary filter/map/join ×1 setup-only (OK — not tracked)
    i. HighlightedText per title + per hit: memo node + regex parse + Show/For
       nodes per part [MEMO + SHOW-PER-PART REDUNDANT — D1/D3]
    j. playerUrl NOT evaluated (iframe inside Show when=playerActive — OK, lazy)
14. scroll effect: prepareResultsRuntime().then → rAF(scrollToLinkedResult)
    (REQUIRED, frame-gated)
15. LCP paints on first video cards (images loading=lazy — REQUIRED)
```

### (c) Expand-moments (click “Show all moments” on one row)

```steps
1.  click → runExpand stable ref (after D7; today L206 closure — same path)
2.  momentsLoading(true) → Show swaps chevron→spinner (surgical, REQUIRED)
3.  searchVideoMoments fetch (network, off-path)
4.  response.videos.find(predicate) — 1 scan over ≤page videos (REQUIRED, L52)
5.  setAllHits(newHits) → allHits signal flips
6.  DEPENDENTS RE-RUN (plain-fn fan-out, no memo dedupe):
    - For each={visibleHits()} → slice (full array passthrough when showAll)
    - selectedHit() → find-scan #1 (+ slice/find #2 on miss) [REDUNDANT — D5]
    - hasHiddenMoments() → 2 length reads (OK)
    - badge {selectedHit()?.timestamp}, aria-label, youtube_url reads → each
      re-invokes selectedHit chain [REDUNDANT — D5]
7.  setShowAllMoments(true) → second invalidation wave over the same readers
    (Solid batches signal sets in the same tick? No — await point separates
    them: two commits. Batching opportunity noted in D5: set hits + flag before
    the await resolves? They are sequential statements after await → same
    synchronous flush → ONE commit. OK as-is; no diff.)
8.  For reconciles: network hit objects are NEW identities → keyed reuse fails
    → full excerpt remount (TranscriptExcerpt ×H + HighlightedText memos ×H)
    [PROVEN-UNAVAILABLE unless API preserves identity — see D5 note]
9.  capture(result_moments_expanded, {...spread}) — 1 event-path object (OK)
10. momentsLoading(false) → spinner→list swap (REQUIRED)
```

Step 8 honesty note: no reactivity-shape diff can fix remount-by-new-identity;
the only fix is data-layer (merge incoming hits by `id` against `allHits` so
`For` reuses rows and appends). Proposed as an optional follow-up in D5, not
claimed as a reactivity win.

---

## 4. Proposed diffs (behavior-identical; src/ untouched — apply on review)

Each diff编号 maps to vitals: **FCP** first paint · **LCP** results paint ·
**INP** input/click latency · **CLS** layout shift (none of these change layout —
stated per diff) · **TBT** main-thread blocking · **LongTasks/animation-frames**.
No diff changes rendered DOM, copy, ordering, ARIA, analytics event names, or
fetch semantics — stated per diff where it matters.

### D1 — `highlighted-text.tsx`: drop memo, plain derivation (TBT-mount)

Why: single reader (`For each={parts()}`), cheap compute (regex over a snippet
≤ ~300 chars), static inputs per mount. Removes 1 memo node + equals-check per
evaluation per title/hit instance (instances ≈ N×(1+h) per page).
Vitals: TBT-mount ↓ (fewer reactive nodes at results mount). No FCP/LCP-layout
change; CLS none; DOM identical.

```tsx
// src/components/highlighted-text.tsx
import { For } from "solid-js";
import type { Component } from "solid-js";
import { highlightedParts } from "../utils";

export const HighlightedText: Component<{
  text: string;
  highlight?: string;
  query: string;
}> = (props) => {
  // Single tracked read below; a memo would only add a node + === check for a
  // ~µs regex over a short snippet (see §2 memo discussion + #1932).
  const parts = () => highlightedParts(props.text, props.highlight, props.query);

  return (
    <For each={parts()}>
      {(part) =>
        part.highlighted ? (
          <mark class="bg-vesper-yellow/20 px-1 font-semibold text-vesper-yellow">{part.text}</mark>
        ) : (
          <>{part.text}</>
        )
      }
    </For>
  );
};
```

(Includes D3: `Show`-per-part → ternary. Same DOM: `<mark>` vs text fragment.
`part` objects are static post-mount, so no update semantics are lost.)

### D4 — `search-results.tsx`: single-pass partition, no spreads (TBT/LCP)

Why: replaces 3 filter passes + 3 intermediate arrays + 1 spread-concat with 1
pass and 2 pre-sized arrays (§2 C: 37.7% on the shape). Ordering preserved:
`[...videos(kind=video), ...unknown]` then `shorts` — identical sequence.
Vitals: TBT ↓ on search-to-results commit; LCP ↓ (less work before paint). CLS
none (same sections/DOM).

```tsx
// src/components/search-results.tsx (replace sectionVideos/unknownVideos + call sites)
const partitionVideos = (videos: SearchVideoResult[]) => {
  const videosAndUnknown: SearchVideoResult[] = [];
  const shorts: SearchVideoResult[] = [];
  for (const video of videos) {
    if (video.content_kind === "short") shorts.push(video);
    else videosAndUnknown.push(video); // "video" first, "unknown" appended — same order as [...sectionVideos(videos,"video"), ...unknownVideos(videos)]
  }
  return { videosAndUnknown, shorts };
};

// inside SearchResults component body (runs once per response — setup-only):
// const { videosAndUnknown, shorts } = partitionVideos(props.response.videos);
// <ResultSection title="Videos" videos={videosAndUnknown} … />
// <ResultSection title="Shorts" videos={shorts} … />
```

Note: `partitionVideos(props.response.videos)` runs setup-once per response (not
tracked), so no memo is needed; do NOT wrap it in `createMemo` (would add a
node for one reader — same reasoning as D1).

### D5 — `video-result.tsx`: cache visible hits + selected id per evaluation (INP/TBT)

Why: kills the 3-slices-per-eval fan-out (§2 B: 83.9% collapsed / 38.5%
expanded on the shape; 3→1 slice allocs counted) and the per-hit `selectedHit()`
re-chain (L191). One `createMemo` here is justified: ≥5 readers
(For input, selected-hit find, badge, aria-label, youtube_url) — the multi-reader
case from the Solid docs where memos pay off.
Vitals: INP ↓ on moment-select + expand (the click path evaluations); TBT ↓ at
results mount (×N rows). CLS none.

```tsx
// src/components/video-result.tsx — replace L21–29 derivations:
import { For, Show, createMemo, createSignal, onCleanup } from "solid-js";

const visible = createMemo(() => (showAllMoments() ? allHits() : allHits().slice(0, 3)));
const selectedId = createMemo(
  () =>
    visible().find((hit) => hit.id === selectedHitId())?.id ??
    visible()[0]?.id ??
    props.video.hits[0]?.id,
);
const selectedHit = createMemo(() => visible().find((hit) => hit.id === selectedId()));
// …per-hit check becomes a string compare (no chain):
// selected={hit.id === selectedId()}
// …badge/aria/url read selectedHit()?.timestamp etc. (memo-shared, computed once)
```

Behavior notes: `selectedHit()` type stays `SearchHit | undefined` with the same
fallback chain (`visible[0] ?? props.video.hits[0]` → now via id lookup; if the
first visible hit differs from `props.video.hits[0]` the resolved object is the
same reference — `visible()[0]`). `playerUrl` (D6) reads this memo. New-object
remount on expand (bytecode step (c).8) is unchanged by this diff — data-layer
merge-by-id is the optional follow-up (append-only update preserving hit
identity so `For` reuses rows), deliberately out of scope here.

### D2 — `video-result.tsx`: lazy `AbortController` (TBT-mount / GC)

Why: N controllers per page mount, N−1 never signalled. Allocate on first expand
only. Vitals: TBT-mount ↓ (fewer allocs in the mount burst), minor GC. No
behavior change: abort-on-unmount preserved via the same `onCleanup`.

```tsx
// src/components/video-result.tsx — replace L19–20:
let momentsController: AbortController | undefined;
onCleanup(() => momentsController?.abort());
const getMomentsController = () => (momentsController ??= new AbortController());
// expandMoments: searchVideoMoments(props.query, props.video.video_id, getMomentsController().signal)
```

### D6 — `video-result.tsx`: memoize `playerUrl` on selection (INP)

Why: `new URL()` parse + serialize per read; after D5 the memo recomputes only
when the selected hit identity changes (activation / moment switch), not on
every badge/aria read. Vitals: INP ↓ on moment-switch while player open. CLS
none; `src` value identical (`autoplay=1` same serialization).

```tsx
// src/components/video-result.tsx — replace L29:
const playerUrl = createMemo(() =>
  autoplayEmbedUrl(selectedHit()?.embed_url ?? props.video.embed_url),
);
```

(Evaluated lazily inside the `Show when={playerActive()}` branch as today — no
URL parse happens before first activation, same as current.)

### D7 — `video-result.tsx`: hoist static per-row handlers (TBT-mount / GC)

Why: L146/L168/L176/L206 allocate fresh closures per row mount whose bodies are
row-independent except for one stable reference; L192's per-hit closure keeps
its (unavoidable — Rationale R3) single closure but shrinks its captured body.
§2 A proves the mechanism (82.7% on the raw shape; realized app delta is the
mount-burst fraction). Vitals: TBT-mount ↓, fewer short-lived functions for GC.
DOM, analytics payloads, and event names unchanged.

```tsx
// src/components/video-result.tsx — component-scope stable handlers:
const openResult = () => capture("result_opened", resultProperties(props));
const copyLink = () => void copyResultLink();
const runExpand = () => void expandMoments();
const selectHit = (hit: SearchHit) => {
  setSelectedHitId(hit.id);
  activatePlayer();
  capture("result_timestamp_selected", {
    ...resultProperties(props),
    match_kind: hit.match_kind,
  });
};
// JSX: onClick={openResult} (×2: title link L146 + Open-on-YouTube L176)
//      onClick={copyLink} (L168), onClick={runExpand} (L206)
//      onSelect={() => selectHit(hit)} (L192 — one minimal closure per hit)
```

### D9 — `search-form.tsx`: memoize suggestion derivations (INP per keystroke)

Why: `visibleSuggestions()` slices (up to 5×/keystroke across L26/L41/L45/L53/
L148) and `suggestionsVisible()` re-trims + re-slices per read (ARIA attrs L91–94
read it 3× per render). Two memos → 1 slice + 1 trim-chain per query change;
`For` input identity stabilizes so the reconciler diff is a no-op when the
8-window is unchanged. Multi-reader (≥4) → memo justified.
Vitals: INP ↓ per keystroke while typing `@…`; TBT ↓. Suggestions, ordering,
keyboard nav, ARIA wiring unchanged. CLS none.

```tsx
// src/components/search-form.tsx — replace L23–27:
import { createMemo } from "solid-js";
const visibleSuggestions = createMemo(() => props.channelSuggestions.slice(0, 8));
const suggestionsVisible = createMemo(
  () =>
    !suggestionsDismissed() &&
    props.query.trim().startsWith("@") &&
    visibleSuggestions().length > 0,
);
// handleSearchKeyDown + JSX keep calling visibleSuggestions()/suggestionsVisible()
// — now cached reads instead of recompute.
```

### D10 — `app.tsx` + `channel-query.ts`: one normalized channel-key map (INP per keystroke)

Why: the biggest per-keystroke term (§2 D: 98.6% on the shape). Three memos each
O(C)-scan with per-channel `normalizeHandle` string allocs on every keystroke
(`directChannel`, `mentionedChannel` incl. its second suggestions re-scan,
`channelSuggestions` filter). One `Map` built per channel-list change (rare:
fetch/cache only); all lookups O(1). Suggestion _filtering_ (substring match)
still scans, but over pre-lowercased keys with zero allocs per channel.
Vitals: INP ↓ per keystroke (the dominant term); TBT ↓. Resolution semantics
unchanged: exact-handle match first, then single-suggestion disambiguation, then
`[]` — same returns for every input (including the `suggestions.length === 1`
rule in `resolveChannelMention`).

```tsx
// src/app.tsx — one map memo replaces per-memo scans:
const channelByKey = createMemo(() => {
  const map = new Map<string, IndexedChannel>();
  for (const channel of channels() ?? [])
    map.set(normalizeHandle(channel.handle).toLowerCase(), channel);
  return map;
});
const selectedChannel = createMemo(() =>
  youtuber() ? channelByKey().get(normalizeHandle(youtuber()).toLowerCase()) : undefined,
);
const directChannel = createMemo(() => channelByKey().get(normalizeHandle(query()).toLowerCase()));
// channel-query.ts: add key-precomputed variants (keep old fns for other callers):
// channelSuggestionsByKeys(keys: {handle,key,nameKey}[], needle: string)
// resolveChannelMentionByKeys(...) — same single-suggestion rule, no per-channel normalize.
const channelSuggestions = createMemo(() => suggestionScan(channelKeys(), query()));
const mentionedChannel = createMemo(() => mentionResolve(channelKeys(), query()));
```

(Exact helper code follows the existing `channel-query.ts:4–32` logic line for
line, substituting precomputed `key`/`nameKey` fields for the inline
`normalizeHandle(...).toLowerCase()` calls; omitted here to avoid duplicating
the file, behavior delta is nil by construction.)

### D11 + D12 — `app.tsx`: effect/computed micro-allocs (TBT, minor)

```tsx
// D11 — warm effect computed (L324–333): return the boolean, not a wrapper:
createEffect(
  () => {
    const normalizedQuery = query().trim();
    const searchQuery = mentionedChannel() ? mentionQuery() : normalizedQuery;
    const searchMode = normalizedQuery.startsWith("@") && !mentionedChannel() ? "index" : "search";
    return searchMode === "search" && searchQuery.length >= 2 && availability().search_enabled;
  },
  (shouldWarm) => {
    if (!shouldWarm) return;
    const timeout = window.setTimeout(() => {
      void Promise.all([prepareSearchRuntime(), prepareResultsRuntime()]).catch(() => undefined);
    }, 900);
    return () => window.clearTimeout(timeout);
  },
);
// D12 — executableSearch (L95): trivial ternary, one reader → plain accessor:
// const executableSearch = () => (availability().search_enabled ? submitted() : undefined);
// (createSearchResource tracks it identically through its effect.)
```

Vitals: TBT-micro ↓ (no per-keystroke wrapper object). Behavior identical
(debounce, runtime-only warm, no speculative fetch — preserved).

---

## 5. Verification plan (Playwright / headless-Chrome)

Baseline (this candidate, no `src/` changes): `bun run build` passes
(`✓ built in 386ms`; entry `index-*.js` 32.10 kB / gzip 11.53 kB,
`search-results-*.js` 13.10 kB / gzip 4.72 kB — lazy split intact, so FCP-critical
bytes are unaffected by any diff above; none of D1–D12 add dependencies).

### 5a. Existing gates (run before + after applying diffs)

```bash
bun run build
bun scripts/check-bundle-performance.ts   # initial-JS ≤30kB / total-JS ≤55kB / CSS ≤8kB gzip
bun run frontend:check                    # typecheck + format + lint + build
# Live search-quality guard (behavior-identical proof for relevance-sensitive files):
curl -fsS 'https://ytsearch-api-typesense-fly-staging.fly.dev/api/search?q=fable%20on%20high&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:3] | map({title, hit_count:(.hits|length)})'
curl -fsS 'https://ytsearch-api-typesense-fly-staging.fly.dev/api/search?q=I&youtuber=%40t3dotgg&per_page=10' | jq '.videos[0:3] | map({title,published_at})'
```

`scripts/check-frontend-performance.ts` targets prod (`https://search.pronsh.dev`,
budgets: JS ≤55 kB gzip, FCP ≤1800 ms, search-to-results ≤2500 ms) — do NOT run
it against staging deploys as a gate; use the local variant below and compare
budgets relatively (before/after on the same machine).

### 5b. Local before/after measurement (headless Chromium, same machine)

Against `vite preview` (built `dist/`) with a stubbed `/api/search` fixture
(deterministic videos/hits, so render cost — not network — is compared):

```ts
// scripts/perf-candidate-1.local.ts (proposed; Playwright + CDP)
import { chromium } from "@playwright/test";
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
await page.goto(process.env.PREVIEW_URL ?? "http://127.0.0.1:4173/", { waitUntil: "networkidle" });
const fcp = await page.evaluate(
  () =>
    performance.getEntriesByType("paint").find((e) => e.name === "first-contentful-paint")
      ?.startTime ?? 0,
);
// search-to-results (render path b): fill + submit with stubbed API, time to first <article>
await page.getByRole("textbox").fill("fable on high");
const t0 = await page.evaluate(() => performance.now());
await page.getByRole("button", { name: "Search", exact: true }).click();
await page.locator("article").first().waitFor({ state: "visible" });
const searchToResults = (await page.evaluate(() => performance.now())) - t0;
// expand-moments INP proxy (render path c): click “Show all moments”, time to 4th excerpt
await page
  .getByRole("button", { name: /Show all moments/i })
  .first()
  .click();
const t1 = await page.evaluate(() => performance.now());
await page.locator("article").first().locator("button").nth(4).waitFor({ state: "visible" });
const expandToMoments = (await page.evaluate(() => performance.now())) - t1;
// typing INP proxy (D9/D10): time 8 keystrokes of "@t3dotgg" end-to-end via input latency
// + CDP heap: query `performance.measureUserAgentSpecificMemory?.()` or HeapProfiler allocation sampling
//   around search-to-results to count short-lived Function/Array allocations before/after.
console.log(
  JSON.stringify({
    fcpMs: Math.round(fcp),
    searchToResultsMs: Math.round(searchToResults),
    expandToMomentsMs: Math.round(expandToMoments),
  }),
);
await browser.close();
```

Pass criteria (same machine, 5 runs, median): search-to-results ↓ or neutral,
expand-to-moments ↓ or neutral, FCP neutral (±noise), zero console errors,
Playwright core-flows suite green (`scripts/run-playwright.sh
tests/e2e/core-flows.spec.ts`).

### 5c. Manual trace fallback (no Playwright)

1. `bun run build && bunx vite preview --port 4173`, open headless or headed
   Chromium with DevTools → Performance → record.
2. Type `@t3` (typing-INP frames; check LongTasks absent), submit, stop: main
   thread should show one commit task for results mount; after D4/D5/D7 it
   shortens, no new tasks appear.
3. Expand moments: single click task; flame should show 1 slice/find evaluation
   per signal (D5) instead of the fan-out.
4. Memory tab → allocation sampling during search-to-results: `JSFunction` count
   drops (D7), `(array)` count drops (D4/D5/D9).

---

## 6. Rationale — alternatives considered and rejected

- **R1 — Memoize everything (engagementSummary, posterThumbnail, executableSearch
  as-is, partitionVideos).** Rejected: per Solid docs + discussion #1932, a memo
  pays only with multiple readers or expensive compute. `engagementSummary`
  (video-result.tsx:161) runs setup-once (not in a tracked scope) — memoizing
  adds a node to save zero re-runs. Same for `partitionVideos` (one reader) and
  `posterThumbnail` (one tracked evaluation). Memos were added only where reader
  fan-out exists (D5: ≥5, D9: ≥4, D10: 3 scans → 1 map) and removed where it
  doesn't (D1, D12).
- **R2 — Convert cheap plain derivations (`hasHiddenMoments`, `mentionQuery`,
  `hasChannel`, `useful`) into memos.** Rejected: same break-even — booleans and
  string compares are ns-scale; memo nodes cost an allocation + `===` check and
  widen the reactive graph reviewers must hold in their heads.
- **R3 — Kill per-hit `onSelect` closures via delegation (`data-hit-id` +
  `onClick` on the list container).** Rejected: structurally saves ~1 closure per
  hit but breaks Solid idiom (per-item callbacks are the framework's unit of
  update), complicates keyboard-focus semantics of the excerpt `<button>`, and
  risks analytics regressions. D7 shrinks the closure instead — the V8-proven
  middle ground (§2 A: allocation is structural, so minimize body, keep shape).
- **R4 — List virtualization for results/moments.** Rejected: pages render
  ~10 videos × ≤3 excerpts (≤~40 post-expand); mount cost is a sub-frame burst,
  not a scroll-window problem. Virtualization would add an observer-driven
  reactivity layer for no measured need.
- **R5 — Global store / context for the normalized channel map.** Rejected: the
  map has exactly one consumer subtree (App's four memos); a local `createMemo`
  (D10) is the existing abstraction (`youtube_search`-style: prefer what exists)
  with no new concepts, no cache-invalidation surface beyond `channels()`.
- **R6 — Drop `keyed` on result/error/dialog Shows to avoid remounts.** Rejected:
  `keyed` is correctness, not overhead: without it, a slow first response
  resolving after a fast second would paint stale videos under a new query.
  Remount-on-new-response is intended (bytecode (b).5).
- **R7 — Manual index loops / `Array.from` mapping instead of `<For>`.**
  Rejected: loses Solid's keyed row reuse (which is what makes typing
  keystrokes cheap today — stable channel objects skip DOM work); D9/D10 make
  the _inputs_ stable instead of abandoning the primitive.
- **Left untouched deliberately:** `createEffect(request, handler)` two-fn form
  (search-resource.ts:17, search-form.tsx:29) — shared codebase idiom, presumably
  Solid 2 RC API; changing it is out of scope and behavior-risky. `Loading`/
  `lazy` boundaries — already optimal (entry 11.53 kB gzip; results chunk
  prefetched on submit, warmed on idle). View-transition `flush()`
  (utils.ts:31, search-form.tsx:81) — load-bearing for snapshot correctness.
  `runSearch` identical-submit dedupe (app.tsx:271) — prevents redundant
  fetch+remount; a perf candidate must not remove the cheapest win in the file.

## Net effect (no src/ edits made by this candidate)

| Diff                                       | Removes per…                                                | Vitals                            |
| ------------------------------------------ | ----------------------------------------------------------- | --------------------------------- |
| D1+D3 HighlightedText memo + per-part Show | 1 memo node + 1 Show/For-node-set per part × N×(1+h) mounts | TBT-mount ↓                       |
| D4 single-pass partition                   | 2 passes + 3 arrays + 1 spread per response                 | TBT ↓, LCP ↓                      |
| D5 cached visible/selected                 | 2 slices + O(h) re-chains per eval × N rows                 | INP ↓, TBT ↓                      |
| D2 lazy AbortController                    | N−1 controllers per page mount                              | TBT-mount ↓, GC ↓                 |
| D6 playerUrl memo                          | URL parse per redundant read                                | INP ↓ (player open)               |
| D7 hoisted handlers                        | ~5 closures/row mount + bodies                              | TBT-mount ↓, GC ↓                 |
| D9 suggestion memos                        | 4 slices + trim-chains per keystroke                        | INP ↓                             |
| D10 channel-key map                        | 3× O(C) normalize-scans per keystroke                       | INP ↓ (largest)                   |
| D11+D12 effect micros                      | 1–2 wrapper objects per keystroke/update                    | TBT-micro ↓                       |
| CLS impact of all diffs                    | —                                                           | none (no layout/DOM-order change) |
