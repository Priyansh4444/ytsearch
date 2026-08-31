#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
checker="$SCRIPT_DIR/check-staging-machines.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$tmp/bin" "$tmp/apps" "$tmp/volumes"

cat > "$tmp/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
action="$1 $2"
[[ "$action" == "machine list" || "$action" == "volumes list" ]]
app=""
while (( $# > 0 )); do
  case "$1" in
    --app) app="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [[ "$action" == "machine list" ]]; then
  cat "$FAKE_FLY_APPS/$app.json"
else
  cat "$FAKE_FLY_VOLUMES/$app.json"
fi
SH
cat > "$tmp/bin/curl" <<'SH'
#!/usr/bin/env sh
set -eu
printf '{"schedule_policy":"always_on","enforced":false}\n'
SH
chmod +x "$tmp/bin/flyctl" "$tmp/bin/curl"

jq -n '[{
  id:"13131313131313",state:"started",region:"sjc",
  image_ref:{registry:"registry.fly.io"},
  config:{guest:{cpu_kind:"shared",cpus:1,memory_mb:512},
    mounts:[{volume:"vol_1313131313131313",path:"/data"}]}
}]' > "$tmp/apps/ytsearch-api-raw-production.json"
jq -n '[
  {
    id:"10101010101010",state:"started",region:"sjc",
    image_ref:{registry:"registry.fly.io"},
    config:{guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
      mounts:[{volume:"vol_1010101010101010",path:"/data/mutable"}]}
  },
  {
    id:"22222222222222",state:"stopped",region:"sjc",
    image_ref:{registry:"registry.fly.io"},
    config:{guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
      mounts:[{volume:"vol_1111111111111111",path:"/data/mutable"}]}
  }
]' > "$tmp/apps/ytsearch-raw-production.json"
jq -n '[
  {
    id:"12121212121212",state:"stopped",region:"sjc",
    image_ref:{registry:"registry.fly.io"},
    config:{guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
      mounts:[{volume:"vol_1212121212121212",path:"/var/lib/ytsearch"}]}
  },
  {
    id:"33333333333333",state:"stopped",region:"ord",
    image_ref:{registry:"registry.fly.io"},
    config:{guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
      mounts:[{volume:"vol_2222222222222222",path:"/var/lib/ytsearch"}]}
  }
]' > "$tmp/apps/ytsearch-indexer-raw-production.json"
jq -n '[{id:"vol_1313131313131313"}]' \
  > "$tmp/volumes/ytsearch-api-raw-production.json"
jq -n '[{id:"vol_1010101010101010"},{id:"vol_1111111111111111"}]' \
  > "$tmp/volumes/ytsearch-raw-production.json"
jq -n '[{id:"vol_1212121212121212"},{id:"vol_2222222222222222"}]' \
  > "$tmp/volumes/ytsearch-indexer-raw-production.json"

common_env=(
  PATH="$tmp/bin:$PATH"
  FAKE_FLY_APPS="$tmp/apps"
  FAKE_FLY_VOLUMES="$tmp/volumes"
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
  SEARCH_AVAILABILITY_URL=https://fixture.invalid/availability
)
env "${common_env[@]}" DETAILS=1 "$checker" > "$tmp/valid.out"
grep -q 'exact_resources_ok=true ok=true$' "$tmp/valid.out"
[[ "$(grep -c '^ytsearch-' "$tmp/valid.out")" == 5 ]]

jq 'map(if .id == "33333333333333" then
  .config.mounts[0].path = "/data" else . end)' \
  "$tmp/apps/ytsearch-indexer-raw-production.json" > "$tmp/indexer-drift.json"
mv "$tmp/indexer-drift.json" "$tmp/apps/ytsearch-indexer-raw-production.json"
if env "${common_env[@]}" "$checker" > "$tmp/drift.out" 2>&1; then
  printf 'staging topology check must reject a mismatched builder mount\n' >&2
  exit 1
fi
grep -q 'exact_resources_ok=false ok=false$' "$tmp/drift.out"

jq 'map(if .id == "33333333333333" then
  .config.mounts = [
    {volume:"vol_2222222222222222",path:"/var/lib/ytsearch"},
    {volume:"vol_9999999999999999",path:"/unexpected"}
  ] else . end)' "$tmp/apps/ytsearch-indexer-raw-production.json" \
  > "$tmp/indexer-extra-mount.json"
mv "$tmp/indexer-extra-mount.json" "$tmp/apps/ytsearch-indexer-raw-production.json"
if env "${common_env[@]}" "$checker" >/dev/null 2>&1; then
  printf 'staging topology check must reject an extra Machine mount\n' >&2
  exit 1
fi

jq 'map(if .id == "33333333333333" then
  .config.mounts = [{volume:"vol_2222222222222222",path:"/var/lib/ytsearch"}]
  else . end) + [{
    id:"99999999999999",state:"stopped",config:{mounts:[]}
  }]' "$tmp/apps/ytsearch-indexer-raw-production.json" > "$tmp/indexer-extra-machine.json"
mv "$tmp/indexer-extra-machine.json" "$tmp/apps/ytsearch-indexer-raw-production.json"
if env "${common_env[@]}" "$checker" >/dev/null 2>&1; then
  printf 'staging topology check must reject an extra Machine\n' >&2
  exit 1
fi

jq 'map(select(.id != "99999999999999"))' \
  "$tmp/apps/ytsearch-indexer-raw-production.json" > "$tmp/indexer-reset.json"
mv "$tmp/indexer-reset.json" "$tmp/apps/ytsearch-indexer-raw-production.json"
jq '. + [{id:"vol_9999999999999999"}]' \
  "$tmp/volumes/ytsearch-indexer-raw-production.json" > "$tmp/indexer-extra-volume.json"
mv "$tmp/indexer-extra-volume.json" "$tmp/volumes/ytsearch-indexer-raw-production.json"
if env "${common_env[@]}" "$checker" >/dev/null 2>&1; then
  printf 'staging topology check must reject an extra app volume\n' >&2
  exit 1
fi

printf 'staging_machine_topology=ok\n'
