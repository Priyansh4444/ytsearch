# Effect v4 migration

The browser and Cloudflare Worker use Effect v4 at asynchronous, cancellation, error, and external-data boundaries. Pure synchronous UI transformations, Solid signals, routing, and platform handler signatures remain native to their environments.

## Migration contract

- Preserve request, response, caching, retry, queue acknowledgement, scheduling, logging, and Fly API behavior.
- Decode every API response before application code reads it.
- Model transport, HTTP response, and decoding failures separately.
- Forward Effect interruption to browser, Fly, and health-check fetches through `AbortSignal`.
- Run Effects only at Promise/platform boundaries with `Effect.runPromise`.
- Keep the initial browser JavaScript entry below 30,000 gzip bytes.
- Pin the v4 beta so upstream changes cannot silently alter production behavior.

## Browser architecture

`src/effect-runtime.ts` owns network Effects, runtime schemas, retry policy, parallel dashboard loading, analytics transport, and clipboard effects. `src/effect-loader.ts` is the single lazy boundary. The public modules in `src/api.ts`, `src/analytics.ts`, and `src/utils.ts` remain small Promise adapters, so UI code does not import Effect directly.

The search lifecycle owns an `AbortController`; a replacement search or component cleanup interrupts the Effect and its fetch. Channel, availability, index-status, video-moment, and dashboard requests use the same cancellation route. Search retries once after 250 ms only for transport failures or HTTP 502/503/504, excluding explicit scheduled-offline and starting responses.

All search, availability, channel, queue, status, dashboard-health, and structured error payloads are decoded with Effect Schema. A syntactically valid but structurally invalid JSON response is an `ApiDecodeError`, not trusted application state.

The former generic analytics SDK was replaced by the narrow operation this product uses: anonymous batch event capture through native `fetch`. The adapter buffers at most 50 pre-initialization events, never sends search text, and intentionally treats telemetry delivery as best effort.

## Bundle decision

The first direct frontend migration passed semantic tests but put Effect and Schema in the entry graph. It was rejected. The accepted design removed the larger generic analytics dependency and lazy-loaded one shared Effect runtime.

| Production build | Initial JS gzip | Total JS gzip | Decision |
| --- | ---: | ---: | --- |
| Pre-migration baseline | 24,301 B | 46,947 B | Reference |
| Direct Effect/Schema import | 46,321 B | 68,965 B | Rejected |
| Lazy Effect runtime plus narrow analytics | 23,440 B | 51,353 B | Accepted |

The accepted build also produces 7,412 B gzip CSS. The enforced budgets are 30,000 B initial JavaScript, 55,000 B total JavaScript, and 8,000 B CSS. The Effect runtime is a 24,131 B gzip on-demand chunk; the initial entry is 861 B smaller than the pre-migration baseline.

## Worker architecture

- `worker/fly-machine.ts`: cancellation-aware Fly fetches, typed HTTP/transport failures, and Schema-validated responses.
- `worker/search-availability.ts`: Effect timeout, cancellation, and failure recovery for health checks.
- `worker/index.ts`: Effect programs for fetch, queue, cron, machine reconciliation, and sequential message processing.

Cloudflare's handlers still return Promises. Each handler runs one Effect and passes the request signal at that boundary. The Worker has no request-scoped mutable global state and continues to stream proxied response bodies.

## Beta constraints

The repository pins `effect` to `4.0.0-beta.102`.

The published package references an internal `SchemaAST.Sentinel` type that is present in Effect source but omitted from the distributed declaration file. `patches/effect@4.0.0-beta.102.patch` restores that declaration, allowing the browser TypeScript project to remain fully library-checked. Remove the patch after upgrading to a release that publishes the declaration.

The Worker project still enables `skipLibCheck` because its deliberately narrow `ES2023`/Workers library set does not declare several browser and ESNext globals referenced transitively by Effect's full declaration surface. Worker application code remains strictly checked; the workaround is scoped to dependency declarations.

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
