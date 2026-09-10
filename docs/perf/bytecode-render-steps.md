# Bytecode render steps: what actually runs, before → after

Synthesized from the `docs/perf/arena/` audits (candidate 1 base + grafts).
Numbers below are the candidates' measured V8 microbenchmarks (Node v26.8.1,
`performance.now()`, `--print-bytecode` / `--allow-natives-syntax` /
`--trace-gc`); they prove direction and mechanism (allocation / pass /
scan counts), not wall-clock deltas on a results page. Absolute validation is
the Playwright plan in §5 of the candidate files.

One line per allocation / computation / effect. `[REDUNDANT]` = removed by
this change with identical output. `[KEPT]` = investigated, correctly cheap
or load-bearing. `→` = the diff that changed the step.

## (a) Initial render (cold `/`, no submitted search)

```steps
1.  index.tsx: render(() => <App/>) — root computation created [KEPT]
2.  app.tsx: searchStateFromLocation() — URL parse, 1 alloc [KEPT]
3.  creatorFromPath() — split/filter small array, once per route [KEPT — scan
    rewrite rejected: pathname-only, cold, diminishing]
4.  16 createSignal allocations (channels seeded from cache, query/youtuber/
    page/sort/submitted/…) [KEPT — once, distinct owners]
5.  executableSearch: memo kept [KEPT — load-bearing, NOT removed].
6.  selectedChannel memo: normalize + linear find over C channels → one map
    build per channel-list change + O(1) lookup [REDUNDANT-SCAN → app.tsx D10]

    (Why step 5 is kept: `createEffect` nodes are built with `_equals: false`
    (`@solidjs/signals` dist, effect-node constructor), so an effect sourced from
    a plain accessor re-runs on *every* write to its dependencies. `useAppLifecycle`
    writes a fresh `provisionalAvailability()` object on mount, on
    `visibilitychange`, and every 60 s; without the memo's `===` barrier each write
    re-runs the search resource effect, aborting the in-flight request and
    refetching the identical one. The memo is the equality barrier, not overhead.)
7.  indexingChannel memo: same scan → same map [REDUNDANT-SCAN → app.tsx D10]
8.  directChannel memo per keystroke: O(C) scan → O(1) map get [REDUNDANT-SCAN → app.tsx D10]
9.  channelSuggestions memo per keystroke: full filter + 2x toLowerCase per
    channel → keyed index scan, zero allocs per channel [REDUNDANT-SCAN → channel-query D10/D13]
10. mentionedChannel memo per keystroke: exact scan + second full filter
    re-scan → map exact + same index, one pass [REDUNDANT-SCAN → app.tsx D10]
11. searchError / successfulSearch / dialog / notice memos → undefined [KEPT —
    subtree gates; successfulSearch identity intentionally drives keyed remount]
12. warm effect computed: {shouldWarm} wrapper object per keystroke → plain
    boolean [REDUNDANT-ALLOC → app.tsx D11]
13. analytics/scroll effects: transition-only wrappers [KEPT]
14. SearchForm mounts: visibleSuggestions slice + suggestionsVisible trim-chain
    recomputed up to 5x per keystroke → 2 memos, 1 slice per query change
    [REDUNDANT → search-form D9]
15. suggestion listbox inserts/removes in flow, shifting form + help text
    [KEPT — an e2e test pins "suggestions push the schedule down instead of
    overlaying it"; the overlay draft was reverted because it contradicts that
    product invariant]
16. preconnect: none → i.ytimg.com handshake starts before poster fetch
    [FCP/LCP → index.html D1; no `crossorigin`, because the poster <img> is a
    no-cors request and a CORS-mode connection would not be reused; API/avatars
    are same-origin, so they need no hint]
17. scroll effect: preload + rAF scheduled even with no location.hash →
    guarded [REDUNDANT microtask+frame → app.tsx D9-guard]
18. magnet: Math.hypot per item per active frame → squared early-out, same
    outputs [TBT → swarm-layout D9]
```

## (b) Search-to-results (submit → first paint of cards)

```steps
1.  runSearch: trim; directChannel + mentionedChannel re-scan C → map/index
    lookups [REDUNDANT-SCAN → app.tsx D10]
2.  request literals: {...current, page} / {...recent, page:1} second objects
    → explicit 4-field literals, stable maps [REDUNDANT → app.tsx D4]
3.  resource effect: abort old, setState loading, fetch [KEPT — lifecycle]
4.  api timing wrapper {...result, request_duration_ms} [KEPT — must not
    mutate the shared inflight response other subscribers read; the spread is
    1 alloc per completed search, CloneObject 14 B]
5.  JSON.stringify(request) cache key [KEPT — manual concat rejected: \u0000
    collisions would corrupt dedupe; 5-8x of ~0.1 us is not worth the risk]
6.  successfulSearch {response, youtuber, page} → keyed remount [KEPT —
    correctness: prevents stale paint on out-of-order responses]
7.  SearchResults: filter(video) + filter(unknown) + spread-concat + third
    filter(short) = 3 passes + 4 arrays → single-pass partition, 0 spreads
    [REDUNDANT → search-results D4; 37.7% on shape, 4.5x + ~14x heap]
8.  header searchResultCountLabel reduce [KEPT — once per response; the
    loop rewrite was reverted as churn]
9.  per VideoResult (xN): AbortController xN (N-1 never used) → lazy create
    [REDUNDANT → video-result D2]
10. per row: visibleHits() slice x3 per eval (For + selectedHit x2) → one
    `visible` memo [REDUNDANT x2 slices → video-result D5; 83.9%/38.5%]
11. per row: selectedHit slice-then-find re-chained per hit (O(h^2) predicate
    runs) → one `selection` memo + string compare per hit [REDUNDANT → D5/D14]
12. per row: 5 mount-once capture/copy/expand closures → stable hoisted refs;
    per-hit onSelect body shrunk to selectHit(hit) [REDUNDANT → D7; 82.7%]
13. per row: engagementSummary filter/map/join (2 closures + 2 arrays + join)
    → one loop, same strings [REDUNDANT → video-result D9; 1.36x]
14. per row: playerUrl new URL parse + serialize per read → string fast path
    fast path only for well-formed absolute embed URLs with no fragment and no
    existing `autoplay=` (the parser still covers both, keeping `set` semantics);
    memo rejected — it would parse eagerly on mount, while the plain accessor
    stays lazy until the player opens [REDUNDANT → video-result D10]
15. per row: posterThumbnail regex, engagementSummary setup-once, formatDate
    [KEPT — one tracked eval / setup-only; memo would add a node for zero re-runs]
16. per title/hit: HighlightedText memo node + Show-per-part nodes → plain
    derivation + ternary, same DOM [REDUNDANT → highlighted-text D1/D3]
17. per hit: fresh /<mark>/ regex + per-part slices [KEPT — shared /g is
    slower and bleeds lastIndex; slice beats manual 1.5x]; per-part
    textarea decode retained [KEPT — entity-map rewrite needs a corpus gate]
18. sort validation [...BASE, ...ENG].includes per route parse → module Set
    [REDUNDANT → search-sorts D6]
19. recent save [recent, ...filter].slice [KEPT — once per submitted search,
    not per render; the loop rewrite was reverted as churn]
20. LCP: first poster loading=lazy, no priority [KEPT as-is — eager-first
    needs field LCP data; dims (1280x720) + aspect-ratio already reserve space]
```

## (c) Expand-moments ("Show all moments" → full list)

```steps
1.  click → runExpand stable ref; momentsLoading(true) swaps chevron→spinner
    in the same task [KEPT — instant feedback, small task]
2.  searchVideoMoments fetch, async; input free [KEPT]
3.  response.videos.find(predicate) over <=page videos [KEPT — tiny N]
4.  setAllHits + setShowAllMoments → dependents re-run once per memo
    (For input, selection, badge, aria, url share the memo) [SHRUNK → D5]
5.  For reconciles network hit objects (new identities) → remount excerpts
    [KEPT — only a data-layer merge-by-id would reuse rows; reactivity cannot]
6.  capture(result_moments_expanded, {...spread}) — 1 event-path object
    [KEPT — per-click, diminishing; single-literal rewrite deferred to keep
    analytics payloads in one shape]
7.  Chunked/idle moment reveal [REJECTED — 78x slower wall-clock on the
    shape; dozens of rows commit in one task within budget]
8.  Delegated list-level onSelect [REJECTED — saves ~1 closure/hit, breaks
    Solid per-item idiom + focus semantics; D7 shrinks instead]
```

## Net effect

| Area                                | Before                                     | After                                               |
| ----------------------------------- | ------------------------------------------ | --------------------------------------------------- |
| Result partition per response       | 3 passes, 4 arrays, 1 spread               | 1 pass, 2 arrays, 0 spreads                         |
| Slice allocs per row eval           | 3                                          | 1                                                   |
| Channel scans per keystroke         | 3x O(C) with C normalize allocs each       | 2 allocs + keyed scan                               |
| Suggestion slice/trim per keystroke | up to 5x                                   | 1x (memo)                                           |
| Row-mount closures                  | ~5 + 3-statement hit closures              | 4 stable refs + 1-call hit closures                 |
| Controllers per page mount          | N                                          | <= expanded rows                                    |
| URL parses before activation        | 0 (lazy)                                   | 0 (lazy; app-shaped embeds skip the parse entirely) |
| Sort validation                     | rebuilds a 7-element array per route parse | module Set                                          |
| Suggestion listbox layout           | in-flow shift (CLS)                        | in-flow [KEPT — e2e pins push-down]                 |
| Head                                | no hints                                   | i.ytimg.com preconnect (no `crossorigin`)           |

## Synthesis record (arena)

- Base: candidate 1 (Solid reactivity). Tie on points with candidate 2
  (10/10, candidate 3: 8); smallest behavior-identical surface — memo only on
  multi-reader fan-out (and kept where the equality barrier gates an effect:
  `executableSearch`); lazy controller; hoisted handlers;
  one channel-key map; no new infra/props/timing changes.
- Grafts from candidate 2: autoplayEmbedUrl fast path, restricted to the
  app-shaped embed URL (D10), sorts module Set (D6), explicit request literals
  (D4), single-eval selection loop (D14, folded into one `selection` memo).
- Grafts from candidate 3: i.ytimg.com preconnect without `crossorigin` (D1 safe
  subset), magnet squared early-out (D9), mount-once handler hoists (D4 subset).
- Reverted after adversarial review: the suggestion-listbox overlay (D8,
  contradicts an e2e test that pins push-down) and the cold-path loop rewrites
  in `recent-searches`/`result-counts` (churn without an INP win).
- Rejected (with reason): api spread removal (mutates shared inflight —
  candidate 1 proved the ownership); manual cache key (separator collisions);
  for-in clone (54 B megamorphic); shared /g (lastIndex bleed); hand top-3
  (slice intrinsic wins); pre-lowercased catalog (invalidation surface);
  entity-map (corpus gate needed); swarm literal (fragile to new fields);
  analytics single-literals (event-path, one shape); suggestion debounce
  (timing/focus change); chunked reveal (78x slower); delegation
  (idiom/focus); virtualization (no measured need); store (one consumer);
  de-keying (stale-paint correctness); eager-first poster (needs field LCP);
  per-card memoization (contradicts break-even discipline); speculative
  prefetch (duplicate origin work); scheduler.yield (non-baseline);
  matchMedia caching (test-mock + live-preference semantics); API
  preconnect (same-origin — would be an unused preconnect).
- Dropouts: none (3/3 candidates + cross-judge completed).
- Verification: `bun run frontend:check` (typecheck + format + lint + build),
  `vitest run` (22 files / 112 tests), `playwright test` (16 e2e flows against a
  local dev server, including the suggestion push-down invariant), bundle + worker
  budgets, `cargo fmt --check`, and `check:docs`. The results-page INP/TBT script
  in §5 of the candidate files was not run in this environment.
