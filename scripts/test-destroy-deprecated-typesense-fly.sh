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

machine() {
  id="$1"
  state="$2"
  volume="$3"
  successor="$4"
  jq -n --arg id "$id" --arg state "$state" --arg volume "$volume" \
    --arg successor "$successor" '[{
      id:$id,
      state:$state,
      config:{
        metadata:{
          lifecycle:"deprecated",
          deprecated_reason:"raw-search-cutover",
          successor:$successor
        },
        mounts:[{volume:$volume}]
      }
    }]'
}

machine 8147deb94937e8 stopped vol_v8e6yy17jo63nplv ytsearch-raw-production \
  >"$tmp/engine.json"
machine d8de466f330468 stopped vol_vdezd781q880qm34 ytsearch-raw-production \
  >"$tmp/bench.json"
jq -s '.[0] + .[1]' "$tmp/engine.json" "$tmp/bench.json" \
  >"$tmp/fixtures/ytsearch-typesense-staging.json"
machine 1855eddf654298 stopped vol_vwnydd578odzzo8v ytsearch-api-raw-production \
  >"$tmp/fixtures/ytsearch-api-typesense-fly-staging.json"
machine 68352d2be17728 stopped vol_r68z3gl5wq7jkqd4 ytsearch-indexer-raw-production \
  >"$tmp/fixtures/ytsearch-indexer-staging.json"

PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null 2>&1
test ! -e "$tmp/calls"

jq '.[0].state = "started"' "$tmp/fixtures/ytsearch-typesense-staging.json" \
  >"$tmp/invalid.json"
mv "$tmp/invalid.json" "$tmp/fixtures/ytsearch-typesense-staging.json"
if PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  APPLY_TYPESENSE_RETIREMENT=destroy-exact-deprecated-typesense-resources \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null 2>&1; then
  echo "retirement should fail before deleting a started Machine" >&2
  exit 1
fi
test ! -e "$tmp/calls"

jq '.[0].state = "stopped"' "$tmp/fixtures/ytsearch-typesense-staging.json" \
  >"$tmp/valid.json"
mv "$tmp/valid.json" "$tmp/fixtures/ytsearch-typesense-staging.json"
PATH="$tmp/bin:$PATH" FLY_FIXTURES="$tmp/fixtures" FLY_CALLS="$tmp/calls" \
  APPLY_TYPESENSE_RETIREMENT=destroy-exact-deprecated-typesense-resources \
  "$SCRIPT_DIR/destroy-deprecated-typesense-fly.sh" >/dev/null

test "$(wc -l <"$tmp/calls" | tr -d ' ')" -eq 7
rg -q '^machine destroy --app ytsearch-typesense-staging 8147deb94937e8 d8de466f330468$' \
  "$tmp/calls"
rg -q '^volumes destroy --app ytsearch-indexer-staging --yes vol_r68z3gl5wq7jkqd4$' \
  "$tmp/calls"
rg -q '^apps destroy --yes ytsearch-typesense-staging ytsearch-api-typesense-fly-staging ytsearch-indexer-staging$' \
  "$tmp/calls"

echo "typesense_retirement_checks=ok"
