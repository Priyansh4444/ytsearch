#!/usr/bin/env sh
set -eu

# One bounded, secret-free staging snapshot intended for agents and monitoring.
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/load-env.sh"

API_URL="${RAW_SEARCH_API_BASE_URL:-https://ytsearch-api-raw-production.fly.dev}"
ENGINE_URL="${RAW_SEARCH_API_URL:-https://ytsearch-raw-production.fly.dev}"
FRONTEND_URL="${RAW_SEARCH_FRONTEND_URL:-https://search.pronsh.dev}"
INDEXER_APP="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
QUERY="${QUERY:-healthcheck}"
FORMAT="${FORMAT:-json}"
CURL_MAX_TIME="${CURL_MAX_TIME:-15}"

for command in curl flyctl jq; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "missing_command=$command" >&2
    exit 1
  }
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

http_get() {
  output="$1"
  url="$2"
  shift 2
  curl -sS --max-time "$CURL_MAX_TIME" -o "$output" -w '%{http_code}\t%{time_total}' "$@" "$url" 2>/dev/null || printf '000\t0'
}

frontend_result="$(http_get "$tmp/frontend" "$FRONTEND_URL")"
engine_result="$(http_get "$tmp/engine" "$ENGINE_URL/health")"
query_encoded="$(printf '%s' "$QUERY" | jq -sRr @uri)"
search_result="$(http_get "$tmp/search" "$API_URL/api/search?q=$query_encoded&per_page=1")"
status_result="$(http_get "$tmp/status" "$API_URL/api/index-status")"
channels_result="$(http_get "$tmp/channels" "$API_URL/api/youtubers")"

for file in search status channels; do
  if ! jq -e 'type == "object"' "$tmp/$file" >/dev/null 2>&1; then
    printf '{}\n' > "$tmp/$file"
  fi
done

metrics='{}'
if [ -n "$ENGINE_URL" ]; then
  metrics_result="$(http_get "$tmp/metrics" "$ENGINE_URL/metrics")"
  if [ "$(printf '%s' "$metrics_result" | cut -f1)" = "200" ]; then
    metrics="$(jq -c '{cpu_ticks:(.cpu_ticks // 0),resident_mb:(((.rss_bytes // 0)/1048576)|floor),index_gb:(((.index_bytes // 0)/1073741824*100|floor)/100),documents:(.documents // 0)}' "$tmp/metrics")"
  fi
fi

machines="$(flyctl machine list --app "$INDEXER_APP" --json 2>/dev/null || printf '[]')"
worker="$(printf '%s' "$machines" | jq -c '[.[] | select(.config.env.QUEUE_TARGET_BACKEND == "raw") | {id,name,state,restart:(.config.restart.policy // "")} ] | first // null')"

json="$(jq -nc \
  --argjson frontend_http "$(printf '%s' "$frontend_result" | cut -f1)" \
  --argjson engine_http "$(printf '%s' "$engine_result" | cut -f1)" \
  --argjson search_http "$(printf '%s' "$search_result" | cut -f1)" \
  --argjson search_seconds "$(printf '%s' "$search_result" | cut -f2)" \
  --argjson status_http "$(printf '%s' "$status_result" | cut -f1)" \
  --argjson channels_http "$(printf '%s' "$channels_result" | cut -f1)" \
  --argjson metrics "$metrics" \
  --argjson worker "$worker" \
  --slurpfile search "$tmp/search" \
  --slurpfile index_status "$tmp/status" \
  --slurpfile channels "$tmp/channels" \
  '{ok:($frontend_http==200 and $engine_http==200 and $search_http==200 and $status_http==200 and $channels_http==200 and (if ($index_status[0].status // "idle") == "idle" then ($worker.state == "started" or $worker.state == "stopped") else $worker.state == "started" end)),frontend_http:$frontend_http,engine_http:$engine_http,search:{http:$search_http,seconds:$search_seconds,total_videos:($search[0].total_videos // null),total_moments:($search[0].total_moments // null)},indexing:($index_status[0] | {status,handle,indexed:(.channel.indexed_video_count // null),public:(.channel.public_video_count // null)}),corpus:{channels_http:$channels_http,channels:(($channels[0].channels // [])|length),incomplete_channels:([($channels[0].channels // [])[] | select((.public_video_count // 0) > (.indexed_video_count // 0))]|length),reported_video_gap:([($channels[0].channels // [])[] | ((.public_video_count // 0)-(.indexed_video_count // 0)) | select(. > 0)]|add // 0)},worker:$worker,engine:$metrics}')"

case "$FORMAT" in
  json) printf '%s\n' "$json" ;;
  pretty) printf '%s\n' "$json" | jq . ;;
  tsv)
    printf '%s\n' "$json" | jq -r '[.ok,.frontend_http,.engine_http,.search.http,.search.seconds,.indexing.status,.indexing.handle,.indexing.indexed,.indexing.public,.corpus.incomplete_channels,.corpus.reported_video_gap,.worker.state,.engine.resident_mb,.engine.index_gb] | @tsv'
    ;;
  *) echo "FORMAT must be json, pretty, or tsv" >&2; exit 1 ;;
esac

printf '%s\n' "$json" | jq -e '.ok' >/dev/null
