#!/usr/bin/env sh
set -eu

confirmation="${APPLY_TYPESENSE_RETIREMENT:-}"
if [ "$confirmation" != "destroy-exact-deprecated-typesense-resources" ]; then
  cat >&2 <<'EOF'
Dry run only. No Fly resources were changed.

After every Raw publication, restart, search, load, browser, and queue gate passes, run:
  APPLY_TYPESENSE_RETIREMENT=destroy-exact-deprecated-typesense-resources \
    scripts/destroy-deprecated-typesense-fly.sh
EOF
  exit 0
fi

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

# Resolve the full destructive target set before deleting the first resource.
verify_app ytsearch-typesense-staging "8147deb94937e8 d8de466f330468"
verify_app ytsearch-api-typesense-fly-staging "1855eddf654298"
verify_app ytsearch-indexer-staging "68352d2be17728"
verify_machine ytsearch-typesense-staging 8147deb94937e8 vol_v8e6yy17jo63nplv ytsearch-raw-production
verify_machine ytsearch-typesense-staging d8de466f330468 vol_vdezd781q880qm34 ytsearch-raw-production
verify_machine ytsearch-api-typesense-fly-staging 1855eddf654298 vol_vwnydd578odzzo8v ytsearch-api-raw-production
verify_machine ytsearch-indexer-staging 68352d2be17728 vol_r68z3gl5wq7jkqd4 ytsearch-indexer-raw-production

flyctl machine destroy --app ytsearch-typesense-staging 8147deb94937e8 d8de466f330468
flyctl machine destroy --app ytsearch-api-typesense-fly-staging 1855eddf654298
flyctl machine destroy --app ytsearch-indexer-staging 68352d2be17728

flyctl volumes destroy --app ytsearch-typesense-staging --yes \
  vol_v8e6yy17jo63nplv vol_vdezd781q880qm34
flyctl volumes destroy --app ytsearch-api-typesense-fly-staging --yes vol_vwnydd578odzzo8v
flyctl volumes destroy --app ytsearch-indexer-staging --yes vol_r68z3gl5wq7jkqd4

flyctl apps destroy --yes \
  ytsearch-typesense-staging \
  ytsearch-api-typesense-fly-staging \
  ytsearch-indexer-staging

echo "Destroyed the four exact deprecated Machines, four attached volumes, and three legacy apps."
