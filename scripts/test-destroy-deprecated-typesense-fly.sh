#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp/bin" "$tmp/fixtures"

cat >"$tmp/bin/flyctl" <<'EOF'
#!/usr/bin/env sh
set -eu

case "$1 $2" in
  "auth whoami") exit 0 ;;
  "machine list")
    shift 2
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --app) app="$2"; shift 2 ;;
        --json) shift ;;
        *) shift ;;
      esac
    done
    cat "$FLY_FIXTURES/$app.json"
    ;;
  "volumes list")
    shift 2
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --app) app="$2"; shift 2 ;;
        --json) shift ;;
        *) shift ;;
      esac
    done
    cat "$FLY_FIXTURES/$app-volumes.json"
    ;;
  "machine destroy"|"volumes destroy"|"apps destroy")
    printf '%s\n' "$*" >>"$FLY_CALLS"
    ;;
  *)
    echo "unexpected flyctl command: $*" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$tmp/bin/flyctl"

inventory="$SCRIPT_DIR/../config/fly-production-resources.json"
reason="$(jq -r '.retirement_sets.typesense.reason' "$inventory")"
for app in $(jq -r '[
  .retirement_sets.typesense.machines[].app,
  .retirement_sets.typesense.unattached_volumes[].app
] | unique[]' "$inventory"); do
  jq --arg app "$app" --arg reason "$reason" '[
    .retirement_sets.typesense.machines[] | select(.app == $app) | {
      id:.machine_id,
      state:"stopped",
      config:{
        metadata:{lifecycle:"deprecated",deprecated_reason:$reason,successor:.successor_app},
        mounts:[{volume:.volume_id}]
      }
    }
  ]' "$inventory" > "$tmp/fixtures/$app.json"
  jq --arg app "$app" '[
    (.retirement_sets.typesense.machines[] | select(.app == $app) | {id:.volume_id}),
    (.retirement_sets.typesense.unattached_volumes[] | select(.app == $app) | {id:.volume_id})
  ]' "$inventory" > "$tmp/fixtures/$app-volumes.json"
done
engine_app="$(jq -r '.retirement_sets.typesense.machines[0].app' "$inventory")"
indexer_app="$(jq -r '.retirement_sets.typesense.unattached_volumes[0].app' "$inventory")"
first_indexer_volume="$(jq -r '.[0].id' "$tmp/fixtures/$indexer_app-volumes.json")"

PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null 2>&1
test ! -e "$tmp/calls"

PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  APPLY_TYPESENSE_RETIREMENT=verify-exact-deprecated-typesense-resources \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null
test ! -e "$tmp/calls"

jq '.[0].state = "started"' "$tmp/fixtures/$engine_app.json" \
  >"$tmp/invalid.json"
mv "$tmp/invalid.json" "$tmp/fixtures/$engine_app.json"
if PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  APPLY_TYPESENSE_RETIREMENT=verify-exact-deprecated-typesense-resources \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null 2>&1; then
  echo "preservation verification should reject a started Machine" >&2
  exit 1
fi
test ! -e "$tmp/calls"

jq '.[0].state = "stopped"' "$tmp/fixtures/$engine_app.json" \
  >"$tmp/valid.json"
mv "$tmp/valid.json" "$tmp/fixtures/$engine_app.json"

jq '.[0].id = "vol_unexpected"' \
  "$tmp/fixtures/$indexer_app-volumes.json" >"$tmp/invalid-volumes.json"
mv "$tmp/invalid-volumes.json" \
  "$tmp/fixtures/$indexer_app-volumes.json"
if PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  APPLY_TYPESENSE_RETIREMENT=verify-exact-deprecated-typesense-resources \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null 2>&1; then
  echo "preservation verification should reject an unexpected volume set" >&2
  exit 1
fi
test ! -e "$tmp/calls"

jq --arg volume "$first_indexer_volume" '.[0].id = $volume' \
  "$tmp/fixtures/$indexer_app-volumes.json" >"$tmp/valid-volumes.json"
mv "$tmp/valid-volumes.json" \
  "$tmp/fixtures/$indexer_app-volumes.json"

PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  APPLY_TYPESENSE_RETIREMENT=verify-exact-deprecated-typesense-resources \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null
test ! -e "$tmp/calls"

PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  APPLY_TYPESENSE_RETIREMENT=unsupported-destructive-request \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null 2>&1
test ! -e "$tmp/calls"

echo "typesense_retirement_checks=ok"
