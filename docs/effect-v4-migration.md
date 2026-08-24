# Effect v4 migration

The Cloudflare Worker uses Effect v4 at asynchronous, cancellation, error, and
external-data boundaries. The browser keeps the same typed boundary semantics
with a narrow native implementation because its entire initial JavaScript graph
has a hard 30,000-byte gzip ceiling. Pure UI transformations, Solid signals,
routing, and platform handler signatures remain native to their environments.

## Migration contract

- Preserve request, response, caching, retry, queue acknowledgement, scheduling, logging, and Fly API behavior.
- Decode every API response before application code reads it.
- Model transport, HTTP response, and decoding failures separately.
- Forward Effect interruption to browser, Fly, and health-check fetches through `AbortSignal`.
- Run Worker Effects only at Promise/platform boundaries with
  `Effect.runPromise`.
- Keep the initial browser JavaScript entry below 30,000 gzip bytes.
- Pin the v4 release candidate so upstream changes cannot silently alter
  production behavior.

## Browser architecture

`src/bootstrap-api.ts` owns startup fetches and small runtime wire decoders.
`src/client-runtime.ts` owns search, queue, dashboard, retry, and clipboard work;
`src/client-runtime-loader.ts` is its single lazy boundary. `src/api.ts` exposes
the Promise interface, coalesces identical in-flight searches, gives each caller
an accurate request timer, and aborts shared work only after its final subscriber
leaves.

The search lifecycle owns an `AbortController`; a replacement search or
component cleanup interrupts its fetch. Search retries once after 250 ms only
for transport failures or HTTP 502/503/504, excluding explicit
scheduled-offline and starting responses.

Every search, availability, channel, queue, status, dashboard-health, and
structured error payload is decoded before application code reads it. A
syntactically valid but structurally invalid JSON response becomes an
`ApiDecodeError`, not trusted application state. The decoder intentionally
covers only this application's wire contract; it is not a second schema
language.

The former generic analytics SDK was replaced by the narrow operation this product uses: anonymous batch event capture through native `fetch`. The adapter buffers at most 50 pre-initialization events, never sends search text, and intentionally treats telemetry delivery as best effort.

## Bundle decision

The first browser migration passed semantic tests but a loaded Effect/Schema
chunk alone cost about 25.4 KB gzip. Keeping that runtime while meeting a 30 KB
initial ceiling left too little room for Solid and the product. The accepted
design keeps Effect on the Worker and uses the narrow browser boundary above.
The bundle gate now traverses every static import from the HTML entry; the old
single-file check could under-report startup cost.

| Candidate build | Initial static graph | Total JS gzip | Decision |
| --- | ---: | ---: | --- |
| Browser Effect/Schema runtime | Over the 30,000 B ceiling when loaded | 52,217 B | Rejected |
| Solid 2 plus native typed wire boundary | 29,108 B | 46,111 B | Accepted |

The accepted build produces 7,661 B gzip CSS. Enforced budgets are 30,000 B
initial JavaScript, 55,000 B total JavaScript, and 8,000 B CSS.

## Worker architecture

- `worker/fly-machine.ts`: cancellation-aware Fly fetches, typed HTTP/transport failures, and Schema-validated responses.
- `worker/search-availability.ts`: Effect timeout, cancellation, and failure recovery for health checks.
- `worker/index.ts`: Effect programs for fetch, queue, cron, machine reconciliation, and sequential message processing.

Cloudflare's handlers still return Promises. Each handler runs one Effect and passes the request signal at that boundary. The Worker has no request-scoped mutable global state and continues to stream proxied response bodies.

## Version constraints

The repository pins `effect` to `4.0.0-rc.111`, `solid-js` and `@solidjs/web`
to `2.0.0-rc.1`, and the Solid Vite plugin to `3.0.0-next.32`. The former Effect
beta declaration patch is gone.

Solid 2 lifecycle changes are explicit: `Loading` replaces the former suspense
boundary, settled work uses `onSettled`, and event-to-submit signal visibility
uses narrowly placed `flush()` calls. The search resource is a discriminated
state instead of independent loading/error booleans.

## Validation

Run:

```sh
bun run typecheck
bun run test:frontend
bun run test:worker
bun run build
bun run perf:bundle
bun run perf:worker
bunx wrangler types --check
bunx wrangler deploy --dry-run
```

Do not claim a latency or throughput improvement from the Effect migration without a production-shaped benchmark. Its demonstrated outcomes are typed failure composition, cancellation propagation, runtime response validation, centralized lifecycle control, and a production bundle that stays below the browser budgets.
