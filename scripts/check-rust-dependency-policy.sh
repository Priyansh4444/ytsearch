#!/usr/bin/env sh
set -eu

tree="$(cargo tree --workspace --edges normal --prefix none)"

for forbidden in native-tls openssl hyper-tls tokio-native-tls; do
  if printf '%s\n' "$tree" | grep -Eq "^${forbidden} v"; then
    echo "dependency_policy=failed forbidden=${forbidden}" >&2
    exit 1
  fi
done

if rg -n 'reqwest\s*=\s*\{' crates --glob Cargo.toml | grep -v 'workspace = true' >/dev/null; then
  echo "dependency_policy=failed reason=reqwest_must_use_workspace_configuration" >&2
  exit 1
fi

echo "dependency_policy=ok tls=rustls"
