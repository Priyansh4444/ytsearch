#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
source_commit="$(printf '%040d' 1)"
base_digest="sha256:$(printf '%064d' 2)"
builder_id=33333333333333
target_id=22222222222222
target_ip='fdaa::2'
target_url="http://[$target_ip]:8108"
source_prefix="$(printf '%.12s' "$source_commit")"
digest_value="${base_digest#sha256:}"
digest_prefix="$(printf '%.12s' "$digest_value")"
builder_image="registry.fly.io/ytsearch-indexer-raw-production:raw-indexer-${source_prefix}-${digest_prefix}"
builder_resolved_image="$builder_image@sha256:$(jq -nr '"6" * 64')"
export RAW_BUILDER_EXPECTED_IMAGE="$builder_resolved_image"
export FAKE_PROVENANCE_SOURCE="$source_commit"
export FAKE_PROVENANCE_BASE="registry.fly.io/ytsearch-raw-production:base@$base_digest"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp/bin"
export FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
export RAW_RELEASE_RECOVERY_ROOT="$tmp/recovery"
export FLY_API_TOKEN=test-secret-token

cat > "$tmp/bin/flyctl" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_FLY_LOG"
case "$1 $2" in
  "auth token") printf 'test-refreshed-token\n' ;;
  "machine list")
    case " $* " in
      *" --app ytsearch-raw-production "*) cat "$FAKE_TARGET_STATE" ;;
      *) cat "$FAKE_FLY_STATE" ;;
    esac
    ;;
  "machine wait")
    case " $* " in
      *" --state started "*) test "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = started ;;
      *" --state stopped "*) test "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = stopped ;;
      *) exit 1 ;;
    esac
    ;;
  "ssh console")
    case " $* " in
      *" --command true "*) ;;
      *"/usr/local/share/ytsearch-release/source-commit"*)
        printf '%s\n%s\n' "$FAKE_PROVENANCE_SOURCE" "$FAKE_PROVENANCE_BASE"
        ;;
      *" --command ytsearch init-backend --backend raw "*) ;;
      *"ytsearch bounded publication job"*)
        if [ "${FAKE_HANG_PUBLISH:-0}" = 1 ]; then
          sleep 30
        fi
        if [ "${FAKE_PUBLISH_FAILURE:-0}" = 1 ]; then
          printf 'failed:1\n'
        else
          printf 'complete\n'
        fi
        ;;
      *) echo "unexpected ssh command: $*" >&2; exit 1 ;;
    esac
    ;;
  *) echo "unexpected flyctl command: $*" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/flyctl"

cat > "$tmp/bin/curl" <<SH
#!/usr/bin/env bash
set -euo pipefail
source "$SCRIPT_DIR/lib/test-fly-fakes.sh"
SH
cat >> "$tmp/bin/curl" <<'SH'
fake_curl_parse "$@"
case "$method:$url" in
  POST:*/lease)
    printf '{"data":{"nonce":"test-lease-nonce","expires_at":4102444800}}\n' > "$output"
    printf '201'
    ;;
  GET:*/machines/33333333333333)
    if [ "${FAKE_BUILDER_DIGEST_DRIFT:-0}" = 1 ]; then
      jq '.[0] | .image_ref.digest = ("sha256:" + ("9" * 64))' \
        "$FAKE_FLY_STATE" > "$output"
    else
      jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    fi
    printf '200'
    ;;
  POST:*/machines/33333333333333/start)
    jq '.[0].state = "started"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    : > "$output"
    printf '200'
    ;;
  POST:*/machines/33333333333333/stop)
    if [ "${FAKE_STOP_FAILURE:-0}" = 1 ]; then
      printf '503'
      exit 0
    fi
    jq '.[0].state = "stopped"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    printf '200'
    ;;
  DELETE:*/machines/33333333333333/lease) printf '204' ;;
  *) echo "unexpected curl request: $method $url" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/curl"

cat > "$tmp/bin/resource-probe" <<'SH'
#!/usr/bin/env sh
set -eu
test "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = started
printf '{"state":"started","memory_peak_bytes":1024,"swap_peak_bytes":0,"disk_percent":1,"oom":0,"oom_kill":0,"cgroup_version":1,"memory_peak_source":"sampled_current","swap_peak_source":"sampled_current"}\n'
SH
chmod +x "$tmp/bin/resource-probe"

write_machine() {
  role="$1"
  profile="${2:-shared-2x}"
  case "$profile" in
    shared-2x) cpu_kind=shared; cpus=2 ;;
    performance-1x-4gb) cpu_kind=performance; cpus=1 ;;
    *) return 1 ;;
  esac
  jq -n --arg role "$role" --arg source "$source_commit" \
    --arg base "$base_digest" --arg target "$target_url" \
    --arg target_id "$target_id" --arg image "$builder_image" \
    --arg profile "$profile" --arg cpu_kind "$cpu_kind" --argjson cpus "$cpus" '[{
    id:"33333333333333",
    instance_id:"builder-version-1",
    state:"stopped",
    image_ref:{digest:("sha256:" + ("6" * 64))},
    config:{
      env:{
        YTSEARCH_MACHINE_ROLE:$role,
        RAW_SEARCH_API_URL:$target,
        RAW_PUBLICATION_WORK_DIRECTORY:"/var/lib/ytsearch/raw-generation-work",
        RAW_PUBLICATION_MIN_FREE_BYTES:"8000000000"
      },
      image:$image,
      init:{entrypoint:["/usr/bin/sleep"],cmd:["infinity"],swap_size_mb:512},
      guest:{cpu_kind:$cpu_kind,cpus:$cpus,memory_mb:(if $profile == "performance-1x-4gb" then 4096 else 2048 end)},
      metadata:{
        lifecycle:"on-demand-publication-builder",
        source_commit:$source,
        base_image_digest:$base,
        builder_profile:$profile,
        target_machine:$target_id
      },
      mounts:[{volume:"vol_2222222222222222",path:"/var/lib/ytsearch"}],
      restart:{policy:"no"},
      dns:{skip_registration:true},
      services:[],
      checks:{},
      files:[]
    }
  }]'
}

jq -n --arg id "$target_id" --arg ip "$target_ip" --arg source "$source_commit" '[{
  id:$id,
  state:"started",
  private_ip:$ip,
  config:{metadata:{source_commit:$source}}
}]' > "$tmp/target-state.json"

write_machine queue-indexer > "$tmp/state.json"
: > "$tmp/fly.log"
: > "$tmp/curl.log"
if PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
  FAKE_CURL_LOG="$tmp/curl.log" \
  RAW_PUBLICATION_TARGET_URL="$target_url" \
RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$tmp/rejected-final.json" \
  "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null 2>&1; then
  echo "publication wrapper must reject a queue worker" >&2
  exit 1
fi
! grep -q '/start$' "$tmp/curl.log"

run_successful_publication() {
  profile="$1"
  final_output="$2"
  write_machine publication-builder "$profile" > "$tmp/state.json"
  : > "$tmp/fly.log"
  : > "$tmp/curl.log"
  PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" \
    FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
    FAKE_CURL_LOG="$tmp/curl.log" RAW_PUBLICATION_TARGET_URL="$target_url" \
    RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
    RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
    RAW_PUBLICATION_BUILDER_PROFILE="$profile" \
    RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
    RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$final_output" \
    "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null
  test "$(jq -r '.[0].state' "$tmp/state.json")" = stopped
  jq -e '.state == "started" and .memory_peak_bytes == 1024' "$final_output" >/dev/null
  grep -q "POST .*/machines/$builder_id/start$" "$tmp/curl.log"
  grep -q -- '--command ytsearch init-backend --backend raw' "$tmp/fly.log"
  grep -q -- 'ytsearch bounded publication job' "$tmp/fly.log"
  grep -q "POST .*/machines/$builder_id/stop$" "$tmp/curl.log"
  grep -q "DELETE .*/machines/$builder_id/lease$" "$tmp/curl.log"
}

run_successful_publication shared-2x "$tmp/final.json"
run_successful_publication performance-1x-4gb "$tmp/performance-final.json"

# shellcheck source=scripts/lib/fly-publication-job.sh
. "$SCRIPT_DIR/lib/fly-publication-job.sh"
handshake_command="$(fly_publication_job_command \
  /tmp/ytsearch-publication-handshake 60 5 1)"
printf '%s' "$handshake_command" | grep -q 'RAW_PUBLICATION_ACTIVATION_READY_FILE'
printf '%s' "$handshake_command" | grep -q 'activation-ready'
printf '%s' "$handshake_command" | grep -q 'activation-finished'

write_machine publication-builder > "$tmp/state.json"
: > "$tmp/fly.log"
: > "$tmp/curl.log"
if PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" \
  FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
  FAKE_CURL_LOG="$tmp/curl.log" FAKE_BUILDER_DIGEST_DRIFT=1 \
  RAW_PUBLICATION_TARGET_URL="$target_url" \
  RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
  RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
  RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
  RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$tmp/digest-drift-final.json" \
  "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null 2>&1; then
  echo "builder digest drift after lease must fail" >&2
  exit 1
fi
! grep -q "POST .*/machines/$builder_id/start$" "$tmp/curl.log"
grep -q "DELETE .*/machines/$builder_id/lease$" "$tmp/curl.log"

write_machine publication-builder > "$tmp/state.json"
: > "$tmp/fly.log"
: > "$tmp/curl.log"
if PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" \
  FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
  FAKE_CURL_LOG="$tmp/curl.log" FAKE_HANG_PUBLISH=1 \
  RAW_PUBLICATION_TARGET_URL="$target_url" \
  RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
  RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
  RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
  RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$tmp/hung-final.json" \
  RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS=1 \
  RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS=1 \
  "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null 2>&1; then
  echo "hung publication ssh must be killed and fail" >&2
  exit 1
fi
test "$(jq -r '.[0].state' "$tmp/state.json")" = stopped

write_machine publication-builder > "$tmp/state.json"
: > "$tmp/fly.log"
: > "$tmp/curl.log"
if PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" \
  FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
  FAKE_CURL_LOG="$tmp/curl.log" FAKE_PROVENANCE_SOURCE="$(printf '%040d' 9)" \
  RAW_PUBLICATION_TARGET_URL="$target_url" \
  RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
  RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
  RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
  RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$tmp/provenance-final.json" \
  "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null 2>&1; then
  echo "mismatched builder provenance must fail" >&2
  exit 1
fi
test "$(jq -r '.[0].state' "$tmp/state.json")" = stopped

write_machine publication-builder > "$tmp/state.json"
: > "$tmp/fly.log"
: > "$tmp/curl.log"
if PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
  FAKE_CURL_LOG="$tmp/curl.log" FAKE_PUBLISH_FAILURE=1 \
  RAW_PUBLICATION_TARGET_URL="$target_url" \
  RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
  RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
  RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
  RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$tmp/failed-final.json" \
  "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null 2>&1; then
  echo "publication failure must propagate" >&2
  exit 1
fi
test "$(jq -r '.[0].state' "$tmp/state.json")" = stopped

write_machine publication-builder > "$tmp/state.json"
: > "$tmp/fly.log"
: > "$tmp/curl.log"
if PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
  FAKE_CURL_LOG="$tmp/curl.log" FAKE_STOP_FAILURE=1 \
  RAW_PUBLICATION_TARGET_URL="$target_url" \
  RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
  RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
  RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
  RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$tmp/unconfirmed-stop-final.json" \
  RAW_BUILDER_STOP_RETRIES=1 RAW_BUILDER_STOP_POLL_ATTEMPTS=1 \
  RAW_BUILDER_STOP_POLL_INTERVAL_SECONDS=0 \
  "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null 2>&1; then
  echo "unconfirmed builder stop must fail the publication wrapper" >&2
  exit 1
fi
test "$(jq -r '.[0].state' "$tmp/state.json")" = started
if grep -q "DELETE .*/machines/$builder_id/lease$" "$tmp/curl.log"; then
  echo "the builder lease must remain held when stop cannot be confirmed" >&2
  exit 1
fi
test -f "$tmp"/recovery/builder-publish-*/lease.header

write_machine publication-builder \
  | jq '.[0].config.env.STALE_QUEUE_MODE = "1"' > "$tmp/state.json"
: > "$tmp/curl.log"
if PATH="$tmp/bin:$PATH" FAKE_FLY_STATE="$tmp/state.json" \
  FAKE_TARGET_STATE="$tmp/target-state.json" FAKE_FLY_LOG="$tmp/fly.log" \
  FAKE_CURL_LOG="$tmp/curl.log" RAW_PUBLICATION_TARGET_URL="$target_url" \
  RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
  RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_digest" \
  RAW_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe" \
  RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$tmp/stale-env-final.json" \
  "$SCRIPT_DIR/fly-publish-raw-generation.sh" publish --backend raw >/dev/null 2>&1; then
  echo "publication wrapper must reject stale builder environment" >&2
  exit 1
fi
if grep -q '/lease$' "$tmp/curl.log"; then
  echo "unsafe builder config must be rejected before lease acquisition" >&2
  exit 1
fi

cat > "$tmp/bin/ytsearch" <<'SH'
#!/usr/bin/env sh
set -eu
[ "$*" = 'publish --backend raw' ]
printf 'ready\n' > "$RAW_PUBLICATION_ACTIVATION_READY_FILE"
while [ ! -f "$RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE" ]; do sleep 0.01; done
date +%s%3N > "$RAW_PUBLICATION_ACTIVATION_STARTED_FILE"
sleep 0.1
date +%s%3N > "$RAW_PUBLICATION_ACTIVATION_FINISHED_FILE"
SH
chmod +x "$tmp/bin/ytsearch"
bounded_flyctl() {
  shift
  while [ "$#" -gt 0 ]; do
    if [ "$1" = --command ]; then
      PATH="$tmp/bin:$PATH" sh -c "$2"
      return
    fi
    shift
  done
  return 1
}
RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS=30
RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS=2
RAW_BUILDER_COMMAND_TIMEOUT_SECONDS=5
RAW_BUILDER_APP=fixture-builder
RAW_BUILDER_MACHINE_ID=33333333333333
RAW_PUBLICATION_ACTIVATION_READY_FILE="$tmp/local-ready"
RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE="$tmp/local-continue"
RAW_PUBLICATION_ACTIVATION_STARTED_FILE="$tmp/local-started"
RAW_PUBLICATION_ACTIVATION_FINISHED_FILE="$tmp/local-finished"
export RAW_PUBLICATION_ACTIVATION_READY_FILE RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE
export RAW_PUBLICATION_ACTIVATION_STARTED_FILE RAW_PUBLICATION_ACTIVATION_FINISHED_FILE
handshake_job="/tmp/ytsearch-publication-${tmp##*.}"
fly_publication_job_wait "$handshake_job" "$tmp/handshake-diagnostics" &
handshake_waiter=$!
for _ in $(seq 1 100); do
  [ -f "$RAW_PUBLICATION_ACTIVATION_READY_FILE" ] && break
  sleep 0.1
done
test -f "$RAW_PUBLICATION_ACTIVATION_READY_FILE"
touch "$RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE"
wait "$handshake_waiter"
test -s "$RAW_PUBLICATION_ACTIVATION_STARTED_FILE"
test -s "$RAW_PUBLICATION_ACTIVATION_FINISHED_FILE"
rm -rf "$handshake_job"

echo "raw_publication_builder_wrapper=ok"
