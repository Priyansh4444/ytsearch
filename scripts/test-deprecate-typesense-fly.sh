#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$tmp/bin"

cat > "$tmp/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
  "auth whoami") exit 0 ;;
  "machine list")
    app=""
    while (( $# > 0 )); do
      case "$1" in
        --app) app="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    jq --arg app "$app" --rawfile updated "$FLY_UPDATED" \
      --argjson extra "${FAKE_EXTRA_MACHINE:-false}" '
      [.retirement_sets.typesense.machines[] | select(.app == $app) | . as $resource | {
        id:.machine_id,
        state:(if env.FAKE_MACHINE_STARTED == .machine_id then "started" else "stopped" end),
        config:{
          mounts:[{volume:.volume_id}],
          metadata:(if ($updated | split("\n") | index($resource.machine_id)) then {
            lifecycle:"deprecated",
            deprecated_reason:"raw-search-cutover",
            successor:$resource.successor_app
          } else {} end)
        }
      }] + (if $extra then [{id:"99999999999999",state:"stopped",config:{mounts:[],metadata:{}}}] else [] end)
    ' "$FLY_INVENTORY"
    ;;
  "volumes list")
    app=""
    while (( $# > 0 )); do
      case "$1" in
        --app) app="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    jq --arg app "$app" '[
      (.retirement_sets.typesense.machines[] | select(.app == $app) | {id:.volume_id}),
      (.retirement_sets.typesense.unattached_volumes[] | select(.app == $app) | {id:.volume_id})
    ]' "$FLY_INVENTORY"
    ;;
  "machine update")
    machine="$3"
    printf '%s\n' "$*" >> "$FLY_CALLS"
    if [[ "${FAKE_SKIP_UPDATE:-0}" != 1 ]]; then
      printf '%s\n' "$machine" >> "$FLY_UPDATED"
    fi
    ;;
  *) printf 'unexpected flyctl command: %s\n' "$*" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/flyctl"
: > "$tmp/updated"

common_env=(
  PATH="$tmp/bin:$PATH"
  FLY_CALLS="$tmp/calls"
  FLY_UPDATED="$tmp/updated"
  FLY_INVENTORY="$SCRIPT_DIR/testdata/fly-production-resources.json"
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
)

env "${common_env[@]}" "$SCRIPT_DIR/deprecate-typesense-fly.sh" >/dev/null 2>&1
[[ ! -e "$tmp/calls" ]]

env "${common_env[@]}" APPLY_TYPESENSE_DEPRECATION=mark-known-typesense-machines \
  "$SCRIPT_DIR/deprecate-typesense-fly.sh" >/dev/null
[[ "$(wc -l < "$tmp/calls")" == 4 ]]
while IFS=$'\t' read -r app machine successor; do
  grep -F -- "machine update $machine --app $app" "$tmp/calls" >/dev/null
  grep -F -- "--metadata successor=$successor" "$tmp/calls" >/dev/null
done < <(jq -r '.retirement_sets.typesense.machines[] |
  [.app, .machine_id, .successor_app] | @tsv' \
  "$SCRIPT_DIR/testdata/fly-production-resources.json")
if grep -Ev -- '--skip-start --yes$' "$tmp/calls" >/dev/null; then
  printf 'every deprecation update must remain stopped\n' >&2
  exit 1
fi

: > "$tmp/updated"
rm -f "$tmp/calls"
if env "${common_env[@]}" FAKE_EXTRA_MACHINE=true \
  APPLY_TYPESENSE_DEPRECATION=mark-known-typesense-machines \
  "$SCRIPT_DIR/deprecate-typesense-fly.sh" >/dev/null 2>&1; then
  printf 'deprecation must reject a drifted Machine set\n' >&2
  exit 1
fi
[[ ! -e "$tmp/calls" ]]

: > "$tmp/updated"
rm -f "$tmp/calls"
started_machine="$(jq -r '.retirement_sets.typesense.machines[0].machine_id' \
  "$SCRIPT_DIR/testdata/fly-production-resources.json")"
if env "${common_env[@]}" FAKE_MACHINE_STARTED="$started_machine" \
  APPLY_TYPESENSE_DEPRECATION=mark-known-typesense-machines \
  "$SCRIPT_DIR/deprecate-typesense-fly.sh" >/dev/null 2>&1; then
  printf 'deprecation must reject a started Machine before mutation\n' >&2
  exit 1
fi
[[ ! -e "$tmp/calls" ]]

: > "$tmp/updated"
rm -f "$tmp/calls"
if env "${common_env[@]}" FAKE_SKIP_UPDATE=1 \
  APPLY_TYPESENSE_DEPRECATION=mark-known-typesense-machines \
  "$SCRIPT_DIR/deprecate-typesense-fly.sh" >/dev/null 2>&1; then
  printf 'deprecation must verify metadata after mutation\n' >&2
  exit 1
fi
[[ "$(wc -l < "$tmp/calls")" == 4 ]]

printf 'typesense_deprecation=ok\n'
