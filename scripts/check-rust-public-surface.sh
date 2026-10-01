#!/usr/bin/env bash
set -euo pipefail

# Fails when a Rust item is more visible than its compiled uses need. Narrowing
# visibility is what lets rustc's dead_code lint see unused items. Hawk is
# built against one rustc; HAWK_TOOLCHAIN must match it.
toolchain="${HAWK_TOOLCHAIN:-1.97.1}"
if ! command -v cargo-hawk >/dev/null 2>&1; then
  echo "cargo-hawk is not installed; skipping the public-surface check" >&2
  exit 0
fi
cargo "+$toolchain" hawk check -D warnings
