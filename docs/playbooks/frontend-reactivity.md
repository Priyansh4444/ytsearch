# Frontend reactivity bugs

Use this when the UI flashes, refetches, remounts a component (a playing
video restarting is the usual symptom), or loses state for no visible reason.

## Fast path

1. **Find the periodic writers.** Most of these bugs are a timer writing a new
   object into a signal. Current writers, in `src/app-lifecycle.ts`:
   - availability every 60 s, plus on `visibilitychange`
   - index status every 15 s, which refreshes the channel list when
     indexing is running
2. **Follow the signal to keyed boundaries.** `<Show when={x()} keyed>`
   remounts its children whenever `x()` returns a *different object*, even an
   equal one. Any memo that reads a periodically rewritten object and returns
   an object literal will remount everything under it.
3. **Reproduce in Playwright with a fake clock,** and detect the remount by
   tagging a DOM node, which a remount replaces:

   ```ts
   await page.clock.install({ time: new Date("2026-07-26T16:00:00Z") });
   await page.goto("/?q=rust+testing");
   // ...start the thing that should survive...
   await page.locator("iframe").evaluate((element) => element.setAttribute("data-original", ""));
   await page.clock.runFor(61_000);
   await expect(page.locator("iframe[data-original]")).toHaveCount(1);
   ```

   See "a playing video survives the periodic availability refresh" in
   `tests/e2e/core-flows.spec.ts`.
4. **Fix with a memo barrier.** Derive the primitive you actually depend on
   (`const searchEnabled = createMemo(() => availability().search_enabled)`)
   and read that instead. A memo's `===` check stops unchanged values.

## Traps

- **`createEffect` has no equality check**: it re-runs on every write to its
  sources, even if the value is the same. Put a `createMemo` between a
  polled signal and any effect that does work (see `executableSearch` in
  `src/app.tsx`).
- **The provisional availability follows the schedule,** but production runs
  `SEARCH_SCHEDULE_MODE=always_on` (`wrangler.jsonc`). Before noon Pacific the
  client's guess says search is closed. Use it only as the first-paint value,
  never to reset a known server answer. Tests that care should set
  `timezoneId: "America/Los_Angeles"` (as `core-flows.spec.ts` does) and a
  morning clock.
- **Solid warnings fail the e2e suite.** `STRICT_READ_UNTRACKED` and
  `ASYNC_OUTSIDE_LOADING_BOUNDARY` are collected in `afterEach`; read the console
  output before assuming the test logic is wrong.
