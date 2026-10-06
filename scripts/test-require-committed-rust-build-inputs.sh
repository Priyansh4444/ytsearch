#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
test_root="$(mktemp -d)"
trap 'rm -rf "$test_root"' EXIT
trap 'exit 130' HUP INT TERM

mkdir -p "$test_root/repository/scripts" "$test_root/repository/crates/example/src"
cp "$SCRIPT_DIR/require-committed-rust-build-inputs.sh" "$test_root/repository/scripts/"
printf '[workspace]\nmembers = []\n' > "$test_root/repository/Cargo.toml"
printf '# lock\n' > "$test_root/repository/Cargo.lock"
printf 'pub fn committed() {}\n' > "$test_root/repository/crates/example/src/lib.rs"
git -C "$test_root/repository" init --quiet
git -C "$test_root/repository" config user.name test
git -C "$test_root/repository" config user.email test@example.invalid
git -C "$test_root/repository" add .
git -C "$test_root/repository" commit --quiet -m initial

commit="$(git -C "$test_root/repository" rev-parse HEAD)"
"$test_root/repository/scripts/require-committed-rust-build-inputs.sh" "$commit"

printf 'pub fn dirty() {}\n' >> "$test_root/repository/crates/example/src/lib.rs"
if "$test_root/repository/scripts/require-committed-rust-build-inputs.sh" "$commit" >/dev/null 2>&1; then
  echo "the release guard must reject modified Rust inputs" >&2
  exit 1
fi
git -C "$test_root/repository" add .
git -C "$test_root/repository" commit --quiet -m tracked
previous_commit="$commit"
commit="$(git -C "$test_root/repository" rev-parse HEAD)"
if "$test_root/repository/scripts/require-committed-rust-build-inputs.sh" "$previous_commit" >/dev/null 2>&1; then
  echo "the release guard must reject a commit captured before HEAD changed" >&2
  exit 1
fi

printf 'pub fn untracked() {}\n' > "$test_root/repository/crates/example/src/untracked.rs"
if "$test_root/repository/scripts/require-committed-rust-build-inputs.sh" "$commit" >/dev/null 2>&1; then
  echo "the release guard must reject untracked Rust inputs" >&2
  exit 1
fi

echo "committed_rust_build_inputs=ok"
