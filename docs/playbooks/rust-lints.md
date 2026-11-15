# Rust lint baseline

## Fast path

```sh
bun run lint:rust
python3 scripts/test-check-rust-lints.py
```

Workspace Clippy rules remain in `Cargo.toml`, with test exceptions in
`clippy.toml`. Every workspace crate must inherit them with `[lints] workspace = true`.
The baseline check runs in `check:precommit` and CI through `bun run check`.

Existing diagnostics are recorded in `config/rust-lint-baseline.json`. The check
rejects new findings by rule, file, message and source span, including extra
occurrences of an existing finding. Moving code to another line does not create
a finding. Removing one finding does not buy permission to add a different one.
Compiler errors and unrelated warnings always fail.

Fix findings incrementally and rerun the check. After reviewing the fixes, refresh
the baseline explicitly with `bun run lint:rust:baseline` and review its diff.
Do not regenerate the baseline to accept new violations. Direct `cargo clippy`
continues to enforce the strict rules without the baseline.

## Traps

- Do not use `--cap-lints`: that also hides unrelated compiler warnings and errors.
  The wrapper uses `--force-warn` for configured Clippy rules to collect the full
  diagnostic set without stopping at the first crate. It compares that set with
  the baseline before returning success.
- Rust is pinned in `rust-toolchain.toml` and `.github/workflows/ci.yml` so local
  and CI diagnostics agree. Update both together.
- Clippy upgrades may change diagnostics. Review the baseline diff when updating
  the Rust toolchain; the baseline is not refreshed during normal checks.

## Facts

- On 2026-11-15 the strict lint configuration was merged with the current
  `master` code. The existing debt was baselined at the user's request so new
  violations can be rejected while existing code is improved incrementally.
