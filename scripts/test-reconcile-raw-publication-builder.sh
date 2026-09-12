#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$tmp/bin"

source_commit="$(git -C "$SCRIPT_DIR/.." rev-parse HEAD)"
base_digest="$(printf '1%.0s' {1..64})"
resolved_digest="$(printf '2%.0s' {1..64})"
builder_id=33333333333333
target_id=22222222222222
target_url='http://[fdaa::2]:8108'
image="registry.fly.io/ytsearch-indexer-raw-production:raw-indexer-${source_commit:0:12}-${base_digest:0:12}"

cat > "$tmp/state.json" <<JSON
[{
  "id":"$builder_id",
  "instance_id":"builder-version-1",
  "state":"stopped",
  "host_status":"ok",
  "incomplete_config":false,
  "image_ref":{"digest":"sha256:$resolved_digest"},
  "config":{
    "env":{"YTSEARCH_MACHINE_ROLE":"publication-builder","STALE_QUEUE_MODE":"1"},
    "init":{"entrypoint":["/bin/sh"],"cmd":["/usr/local/bin/cloud-index-all"]},
    "guest":{"cpu_kind":"shared","cpus":4,"memory_mb":4096},
    "metadata":{"lifecycle":"on-demand-publication-builder"},
    "mounts":[{"volume":"vol_2222222222222222","path":"/data"}],
    "restart":{"policy":"no"},
    "dns":{"skip_registration":true},
    "services":[],
    "checks":{},
    "files":[{"guest_path":"/old-worker","raw_value":"old"}],
    "image":"registry.invalid/old:one"
  }
}]
JSON

jq -n --arg source "$source_commit" --arg id "$target_id" '[{
  id:$id,
  state:"stopped",
  private_ip:"fdaa::2",
  config:{metadata:{source_commit:$source}}
}]' > "$tmp/target-state.json"

cat > "$tmp/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_FLY_LOG"
case "$1 $2" in
  "machine list")
    case " $* " in
      *" --app ytsearch-raw-production "*) cat "$FAKE_TARGET_STATE" ;;
      *) cat "$FAKE_FLY_STATE" ;;
    esac
    ;;
  "secrets sync")
    [[ "$*" == "secrets sync --stage --app ytsearch-indexer-raw-production" ]]
    ;;
  "auth token") printf 'fake-token\n' ;;
  *) printf 'unexpected flyctl command: %s\n' "$*" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/flyctl"

cat > "$tmp/bin/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
method=GET
output=""
request=""
url=""
auth_header=""
lease_header=""
printf '%s\n' "$*" >> "$FAKE_CURL_LOG"
while (( $# > 0 )); do
  case "$1" in
    --request) method="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    --data-binary) request="${2#@}"; shift 2 ;;
    --header)
      if [[ "$2" == @* ]]; then
        header_file="${2#@}"
        if grep -q '^Authorization:' "$header_file"; then
          auth_header="$header_file"
        elif grep -q '^fly-machine-lease-nonce:' "$header_file"; then
          lease_header="$header_file"
        fi
      fi
      shift 2
      ;;
    --write-out|--connect-timeout|--max-time) shift 2 ;;
    --silent|--show-error) shift ;;
    http://*|https://*) url="$1"; shift ;;
    *) printf 'unexpected curl argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done
[[ -n "$output" && -n "$auth_header" && -n "$url" ]]
grep -Fx 'Authorization: Bearer fake-token' "$auth_header" >/dev/null
if [[ "$url" != */lease || "$method" == DELETE ]]; then
  [[ -n "$lease_header" ]]
  grep -Fx 'fly-machine-lease-nonce: fake-nonce' "$lease_header" >/dev/null
fi
case "$method:$url" in
  POST:*/lease)
    jq -e '.ttl == 300' "$request" >/dev/null
    printf '{"data":{"nonce":"fake-nonce","expires_at":4102444800}}\n' > "$output"
    printf 201
    ;;
  GET:*/machines/33333333333333)
    jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    printf 200
    ;;
  POST:*/machines/33333333333333/stop)
    jq '.[0].state = "stopped"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    printf '{}\n' > "$output"
    printf 200
    ;;
  POST:*/machines/33333333333333)
    jq -e '
      .current_version == "builder-version-1" and
      .skip_launch == true and
      .min_secrets_version == 104052995 and
      (.config.env | keys | sort) == [
        "RAW_PUBLICATION_MIN_FREE_BYTES",
        "RAW_PUBLICATION_WORK_DIRECTORY",
        "RAW_SEARCH_API_URL",
        "YTSEARCH_MACHINE_ROLE"
      ] and
      .config.init.entrypoint == ["/usr/bin/sleep"] and
      .config.init.cmd == ["infinity"] and
      (.config.services | length) == 0 and
      (.config.checks | length) == 0 and
      (.config.files | length) == 0 and
      .config.guest == {cpu_kind:"performance",cpus:1,memory_mb:4096} and
      .config.metadata.builder_profile == "performance-1x-4gb"
    ' "$request" >/dev/null
    jq --slurpfile request "$request" --arg digest "$FAKE_RESOLVED_DIGEST" '
      .[0].config = $request[0].config |
      .[0].instance_id = "builder-version-2" |
      .[0].state = "created" |
      .[0].image_ref.digest = ("sha256:" + $digest)
    ' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    printf 200
    ;;
  DELETE:*/lease)
    printf '{}\n' > "$output"
    printf 204
    ;;
  *) printf 'unexpected curl request: %s %s\n' "$method" "$url" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/curl"

cat > "$tmp/fly-config.yml" <<'YAML'
app_secrets_minvers:
  ytsearch-indexer-raw-production: 104052995
YAML

common_env=(
  PATH="$tmp/bin:$PATH"
  FLY_CONFIG_FILE="$tmp/fly-config.yml"
  FLY_API_TOKEN=fake-token
  FLY_API_HOSTNAME=https://api.test
  FAKE_FLY_STATE="$tmp/state.json"
  FAKE_TARGET_STATE="$tmp/target-state.json"
  FAKE_FLY_LOG="$tmp/fly.log"
  FAKE_CURL_LOG="$tmp/curl.log"
  FAKE_RESOLVED_DIGEST="$resolved_digest"
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
  BASE_IMAGE="registry.fly.io/ytsearch-raw-production:base@sha256:$base_digest"
  INDEXER_IMAGE="$image"
  RAW_PUBLICATION_BUILDER_PROFILE=performance-1x-4gb
)

: > "$tmp/fly.log"
: > "$tmp/curl.log"
env "${common_env[@]}" "$SCRIPT_DIR/reconcile-raw-publication-builder.sh" \
  > "$tmp/dry-run.json"
jq -e --arg machine "$builder_id" '
  .apply == false and .machine == $machine and
  .builder_profile == "performance-1x-4gb"
' \
  "$tmp/dry-run.json" >/dev/null
[[ ! -s "$tmp/curl.log" ]]

env "${common_env[@]}" \
  APPLY_RAW_PUBLICATION_BUILDER=configure-exact-inert-builder \
  "$SCRIPT_DIR/reconcile-raw-publication-builder.sh" > "$tmp/applied.json"
jq -e --arg image "$image@sha256:$resolved_digest" '
  .kind == "raw-publication-builder" and .resolved_image == $image and
  .state == "stopped" and .memory_mb == 4096 and .cpus == 1 and
  .builder_profile == "performance-1x-4gb"
' "$tmp/applied.json" >/dev/null
jq -e '
  .[0].state == "stopped" and
  (.[0].config.files | length) == 0 and
  (.[0].config.services | length) == 0 and
  (.[0].config.env | has("STALE_QUEUE_MODE") | not) and
  (.[0].config.env | keys | length) == 4 and
  .[0].config.guest == {cpu_kind:"performance",cpus:1,memory_mb:4096} and
  .[0].config.metadata.builder_profile == "performance-1x-4gb" and
  .[0].config.dns.skip_registration == true
' "$tmp/state.json" >/dev/null
if grep -Eq 'fake-token|fake-nonce' "$tmp/curl.log"; then
  printf 'builder staging credentials must not appear in curl arguments\n' >&2
  exit 1
fi

jq '.[0].config.services = [{internal_port:8108}]' "$tmp/state.json" > "$tmp/unsafe.json"
if env "${common_env[@]}" FAKE_FLY_STATE="$tmp/unsafe.json" \
  "$SCRIPT_DIR/reconcile-raw-publication-builder.sh" >/dev/null 2>&1; then
  printf 'builder reconciliation must reject public services\n' >&2
  exit 1
fi

if env "${common_env[@]}" RAW_PUBLICATION_TARGET_URL=https://search.pronsh.dev \
  "$SCRIPT_DIR/reconcile-raw-publication-builder.sh" >/dev/null 2>&1; then
  printf 'builder reconciliation must reject public target URLs\n' >&2
  exit 1
fi

if env "${common_env[@]}" RAW_PUBLICATION_BUILDER_PROFILE=performance-2x \
  "$SCRIPT_DIR/reconcile-raw-publication-builder.sh" >/dev/null 2>&1; then
  printf 'builder reconciliation must reject unknown resource profiles\n' >&2
  exit 1
fi

RAW_PUBLICATION_BUILDER_PROFILE=performance-1x-4gb
source "$SCRIPT_DIR/lib/raw-publication-builder-profile.sh"
raw_publication_builder_profile_load
raw_publication_builder_profile_apply_production_budgets
[[ "$RAW_CANARY_LEASE_TTL_SECONDS" == 43200 ]]
[[ "$PUBLICATION_TIMEOUT_SECONDS" == 10800 ]]

RAW_PUBLICATION_BUILDER_PROFILE=shared-2x
raw_publication_builder_profile_load
raw_publication_builder_profile_apply_production_budgets
[[ "$RAW_CANARY_LEASE_TTL_SECONDS" == 21600 ]]
[[ "$PUBLICATION_TIMEOUT_SECONDS" == 2400 ]]

printf 'raw_publication_builder_reconciliation=ok\n'
