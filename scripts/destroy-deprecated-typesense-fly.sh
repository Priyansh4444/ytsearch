#!/usr/bin/env sh
set -eu

confirmation="${APPLY_TYPESENSE_RETIREMENT:-}"
case "$confirmation" in
  verify-exact-deprecated-typesense-resources) ;;
  *)
  cat >&2 <<'EOF'
Preservation mode only. This branch cannot destroy Typesense resources.

To verify the complete stopped rollback set without changing it, run:
  APPLY_TYPESENSE_RETIREMENT=verify-exact-deprecated-typesense-resources \
    scripts/destroy-deprecated-typesense-fly.sh

Any future destruction requires a new explicit user decision and a separately
reviewed implementation. The historical destructive implementation is retained
on the Typesense archive branch as trial evidence only.
EOF
  exit 0
  ;;
esac

flyctl auth whoami >/dev/null 2>&1 || {
  echo "Fly authentication is required; run 'flyctl auth login' first." >&2
  exit 1
}

verify_app() {
  app="$1"
  expected_ids="$2"
  actual_ids="$(flyctl machine list --app "$app" --json | jq -r '.[].id' | sort | tr '\n' ' ' | sed 's/ $//')"
  if [ "$actual_ids" != "$expected_ids" ]; then
    echo "$app Machine set changed; expected '$expected_ids', got '$actual_ids'" >&2
    exit 1
  fi
}

verify_volumes() {
  app="$1"
  expected_ids="$2"
  actual_ids="$(flyctl volumes list --app "$app" --json | jq -r '.[].id' | sort | tr '\n' ' ' | sed 's/ $//')"
  if [ "$actual_ids" != "$expected_ids" ]; then
    echo "$app volume set changed; expected '$expected_ids', got '$actual_ids'" >&2
    exit 1
  fi
}

verify_machine() {
  app="$1"
  machine="$2"
  volume="$3"
  successor="$4"
  machine_json="$(flyctl machine list --app "$app" --json | jq -c --arg id "$machine" '.[] | select(.id == $id)')"
  if ! printf '%s' "$machine_json" | jq -e \
    --arg volume "$volume" --arg successor "$successor" '
      .state == "stopped" and
      .config.metadata.lifecycle == "deprecated" and
      .config.metadata.deprecated_reason == "raw-search-cutover" and
      .config.metadata.successor == $successor and
      ([.config.mounts[]?.volume] == [$volume])
    ' >/dev/null; then
    echo "$app/$machine is not the exact stopped deprecated Machine expected for $volume" >&2
    exit 1
  fi
}

# Resolve the complete preservation set and fail if it drifts or restarts.
verify_app ytsearch-typesense-staging "8147deb94937e8 d8de466f330468"
verify_app ytsearch-api-typesense-fly-staging "1855eddf654298"
verify_app ytsearch-indexer-staging "68352d2be17728"
verify_volumes ytsearch-typesense-staging "vol_v8e6yy17jo63nplv vol_vdezd781q880qm34"
verify_volumes ytsearch-api-typesense-fly-staging "vol_vwnydd578odzzo8v"
verify_volumes ytsearch-indexer-staging "vol_458zq611m0yppz14 vol_4qlzgynglnz786gr vol_r68z3gl5wq7jkqd4 vol_v8e6wk3g1l6kj15v"
verify_machine ytsearch-typesense-staging 8147deb94937e8 vol_v8e6yy17jo63nplv ytsearch-raw-production
verify_machine ytsearch-typesense-staging d8de466f330468 vol_vdezd781q880qm34 ytsearch-raw-production
verify_machine ytsearch-api-typesense-fly-staging 1855eddf654298 vol_vwnydd578odzzo8v ytsearch-api-raw-production
verify_machine ytsearch-indexer-staging 68352d2be17728 vol_r68z3gl5wq7jkqd4 ytsearch-indexer-raw-production

echo "Verified the four exact stopped deprecated Machines and complete seven-volume preservation set. No resources were changed."
