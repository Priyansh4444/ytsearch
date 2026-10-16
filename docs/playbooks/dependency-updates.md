# Dependency updates

## Fast path

```bash
bun run deps:outdated          # JS (every dist-tag) and Rust majors in one list
# edit package.json versions, then:
bun install
cargo update --verbose         # semver-compatible Rust bumps; lists held-back majors
# bump Rust majors in crates/*/Cargo.toml, then check for duplicates:
cargo tree -d | grep -E '^[a-z]'
bun run check:precommit        # everything the commit hook runs
bun run test:e2e
```

If the update can change reader output (anything in `youtube_raw_search`'s
tree, such as `zstd`), prove parity against the previous build:

```bash
scripts/perf/reader-build.sh master /tmp/reader-base
scripts/perf/reader-build.sh . /tmp/reader-head
scripts/perf/reader-ab.sh /tmp/reader-base /tmp/reader-head 1   # needs parity: mismatches=0
```

## Traps

- **`bun outdated` misses prerelease lines.** Solid 2 is published under the
  `next` dist-tag while `latest` is still 1.x, so `bun outdated` reports
  nothing for it. `bun run deps:outdated` checks every tag.
- **A Wrangler bump changes the generated types.** `check:precommit` fails
  with "Types at worker-configuration.d.ts are out of date" until you run
  `CLOUDFLARE_LOAD_DEV_VARS_FROM_DOT_ENV=false bunx wrangler types` and commit the result.
- **Bundle budgets fail after upstream growth.** Find the cause before touching a
  budget: `scripts/perf/bundle-diff.sh <previous-ref>` prints the change per
  frontend module and per Worker package. Raise a budget only when the
  growth is upstream, and record the reason next to the number (see
  `scripts/check-bundle-performance.ts`).
- **Docker CLI presence does not imply daemon access.** If `docker info` reports
  permission denied on `/var/run/docker.sock`, local image smoke tests did not
  run. The Images workflow builds and smoke-tests all standalone images.
- **A major bump can duplicate a crate.** If another dependency still needs the
  old major, both copies get compiled in. Check `cargo tree -d`, and hold the
  bump if it duplicates.

## Current holds

Re-check these on every update; delete an entry once it no longer applies.

| Held | Why | Revisit when |
| --- | --- | --- |
| `matchit` on 0.8.4 | `axum` 0.8.9 pins exactly `=0.8.4`; Cargo cannot select 0.8.6 | axum changes its pin |
| `tower-http` on 0.6 | `reqwest` 0.13.5 depends on 0.6; 0.7 would compile both | `reqwest` moves to 0.7 |
| `rust:1.97` in the overlay Dockerfiles | They build on a fixed production base image | the base image is rebuilt |
| Rust 1.97.1 for `scripts/check-rust-public-surface.sh` | `cargo-hawk` is built against that rustc (`HAWK_TOOLCHAIN` overrides) | hawk is rebuilt for a newer rustc |

## Registry audit (2026-10-15)

Follow-up on 2026-10-16: refreshed
[Wrangler 4.151.1](https://registry.npmjs.org/wrangler/4.151.1) and
[Cloudflare Vitest plugin 1.4.4](https://registry.npmjs.org/@cloudflare/vitest-plugin/1.4.4).
Both select Miniflare `5.20261012.1-beta`; the override follows that version.
Workerd remains `1.20261012.1`. The entries below record the initial upgrade.

- Queried every npm dist-tag and the latest stable version of every direct Rust
  dependency with `bun run deps:outdated`; `cargo update --verbose` refreshed
  semver-compatible Rust releases. The direct dependency audit leaves only tower-http 0.7
  held, as explained above.
- Migrated the deprecated `@cloudflare/vitest-pool-workers` package to
  `@cloudflare/vitest-plugin` 1.4.3 using the
  [official migration guide](https://developers.cloudflare.com/workers/testing/vitest-integration/migration-guides/migrate-to-vitest-plugin/).
  The new plugin supports Vitest 5, removing the old Vitest 4 hold; Vitest and
  its Istanbul coverage provider are both 5.0.5. A registry audit of the old
  package alone misses this replacement.
- The final registry refresh picked up patch releases
  [Cloudflare Vitest plugin 1.4.3](https://registry.npmjs.org/@cloudflare/vitest-plugin/1.4.3),
  [Vitest 5.0.5](https://registry.npmjs.org/vitest/5.0.5) and
  [Istanbul coverage 5.0.5](https://registry.npmjs.org/@vitest/coverage-istanbul/5.0.5).
  The coverage provider requires exactly Vitest 5.0.5; the Cloudflare plugin
  accepts Vitest `^4.1.0 || ^5.0.0` and still pins Wrangler 4.151.0 and
  Miniflare `5.20261012.0-beta`, so the runtime override remains unchanged.
- Vitest 5's default filesystem module cache returns host paths to workerd,
  whose virtual filesystem cannot read them (`readAll .../.vitest-cache/...`),
  so Worker tests set `fsModuleCache: false`. Frontend tests retain the cache.
  jest-dom 7.0.1's `/vitest` types also target the old `Assertion<T>` shape;
  the test setup registers its standalone matchers against Vitest 5's
  `Matchers<R, T>` interface, preserving type checks without skipping library
  checks or patching installed dependencies.
- **Build and Worker tests share `dist`.** Run them sequentially: Vite removes
  old assets during a build while the Worker runtime reads its asset manifest.
- Keep Solid 2's `next` line: `solid-js` and `@solidjs/web` are both rc.15,
  paired with `@solidjs/vite-plugin` next.50. That plugin requires rc.14 or
  newer. Do not replace Solid 2 with the Solid 1 `latest` tag.
- Keep Wrangler's selected Miniflare version aligned with its dependency
  metadata: Wrangler 4.151.0 uses `5.20261012.0-beta`. The Worker test plugin uses the same version. The override keeps
  any transitive usage aligned with that runtime. Leaving the old alpha override forces
  Wrangler to execute against an older runtime than it was released with.
  Run Worker tests and regenerate Worker types after changing this pair.
- [Bun 1.4.3](https://github.com/oven-sh/bun/releases/tag/bun-v1.4.3) is pinned
  by `packageManager` and the frontend Docker builder.
  [Deno 2.9.7](https://github.com/denoland/deno/releases/tag/v2.9.7) is the
  current stable EJS solver image. The frontend nginx image follows the
  current mainline 1.31 image, preserving the existing mainline choice.
- GitHub release metadata confirms checkout 7.0.1, setup-bun 2.2.0,
  rust-cache 2.9.2, setup-buildx 4.4.1, build-push 7.4.0 and install-action
  2.87.28; every workflow uses the same checkout pin. The dtolnay stable
  action pin is also refreshed. Full Rust Docker builders already follow
  `rust:1-bookworm` (currently 1.99); overlay and Hawk holds remain deliberate.
- [yt-dlp 2026.08.19](https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19)
  is the current stable release. Indexer Docker builds already install the
  newest `yt-dlp[default]`; running production images require a rebuild and
  deploy to pick up Docker dependency changes.

## Facts

As of 2026-10-03, `effect/Schema` is about 97 KB of the 227 KB Worker bundle. It
is used only for two Fly API response decoders in `worker/fly-machine.ts`.
