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
- **A major bump can duplicate a crate.** If another dependency still needs the
  old major, both copies get compiled in. Check `cargo tree -d`, and hold the
  bump if it duplicates.

## Current holds

Re-check these on every update; delete an entry once it no longer applies.

| Held | Why | Revisit when |
| --- | --- | --- |
| `vitest`, `@vitest/coverage-istanbul` on 4.x | `@cloudflare/vitest-pool-workers` 0.22.0 requires `vitest ^4.1.0` | the pool supports vitest 5 |
| `tower-http` on 0.6 | `reqwest` 0.13.5 depends on 0.6; 0.7 would compile both | `reqwest` moves to 0.7 |
| `rust:1.97` in the overlay Dockerfiles | They build on a fixed production base image | the base image is rebuilt |
| Rust 1.97.1 for `scripts/check-rust-public-surface.sh` | `cargo-hawk` is built against that rustc (`HAWK_TOOLCHAIN` overrides) | hawk is rebuilt for a newer rustc |

## Facts

As of 2026-10-03, `effect/Schema` is about 97 KB of the 227 KB Worker bundle. It
is used only for two Fly API response decoders in `worker/fly-machine.ts`.
