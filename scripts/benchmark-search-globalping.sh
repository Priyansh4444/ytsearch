#!/usr/bin/env sh
set -eu

# Run one HTTP benchmark measurement per backend through Globalping probes.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

QUERY="${QUERY:-react}"
YOUTUBER="${YOUTUBER:-}"
PER_PAGE="${PER_PAGE:-10}"
MEILI_API_HOST="${MEILI_API_HOST:-}"
TYPESENSE_CLOUD_API_HOST="${TYPESENSE_CLOUD_API_HOST:-${TYPESENSE_API_HOST:-}}"
TYPESENSE_FLY_API_HOST="${TYPESENSE_FLY_API_HOST:-}"
MEILI_API_BASE_URL="${MEILI_API_BASE_URL:-}"
TYPESENSE_CLOUD_API_BASE_URL="${TYPESENSE_CLOUD_API_BASE_URL:-${TYPESENSE_API_BASE_URL:-}}"
TYPESENSE_FLY_API_BASE_URL="${TYPESENSE_FLY_API_BASE_URL:-}"
GLOBALPING_COUNTRIES="${GLOBALPING_COUNTRIES:-US GB JP AU BR}"
GLOBALPING_USER_AGENT="${GLOBALPING_USER_AGENT:-ytsearch-benchmark/0.1}"

host_from_url() {
  value="$1"
  value="${value#http://}"
  value="${value#https://}"
  printf '%s' "${value%%/*}"
}

required_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required environment variable: $name" >&2
    exit 1
  fi
}

if [ -z "$MEILI_API_HOST" ] && [ -n "$MEILI_API_BASE_URL" ]; then
  MEILI_API_HOST="$(host_from_url "$MEILI_API_BASE_URL")"
fi
if [ -z "$TYPESENSE_CLOUD_API_HOST" ] && [ -n "$TYPESENSE_CLOUD_API_BASE_URL" ]; then
  TYPESENSE_CLOUD_API_HOST="$(host_from_url "$TYPESENSE_CLOUD_API_BASE_URL")"
fi
if [ -z "$TYPESENSE_FLY_API_HOST" ] && [ -n "$TYPESENSE_FLY_API_BASE_URL" ]; then
  TYPESENSE_FLY_API_HOST="$(host_from_url "$TYPESENSE_FLY_API_BASE_URL")"
fi

required_env MEILI_API_HOST
required_env TYPESENSE_CLOUD_API_HOST
required_env TYPESENSE_FLY_API_HOST

query_string="q=$(printf '%s' "$QUERY" | jq -sRr @uri)&per_page=$PER_PAGE"
if [ -n "$YOUTUBER" ]; then
  query_string="$query_string&youtuber=$(printf '%s' "$YOUTUBER" | jq -sRr @uri)"
fi

locations_json() {
  printf '%s\n' "$GLOBALPING_COUNTRIES" |
    jq -R 'split(" ") | map(select(length > 0) | {country: ., limit: 1})'
}

create_measurement() {
  host="$1"
  locations="$(locations_json)"
  jq -n \
    --arg host "$host" \
    --arg query "$query_string" \
    --argjson locations "$locations" \
    '{
      type: "http",
      target: $host,
      locations: $locations,
      measurementOptions: {
        request: {
          path: "/api/search",
          query: $query,
          method: "GET"
        },
        protocol: "HTTPS"
      }
    }' |
    curl -sS \
      -H "Content-Type: application/json" \
      -H "User-Agent: $GLOBALPING_USER_AGENT" \
      -X POST \
      --data-binary @- \
      https://api.globalping.io/v1/measurements |
    jq -r '.id'
}

poll_measurement() {
  id="$1"
  output="$2"
  i=1
  while [ "$i" -le 20 ]; do
    curl -sS -H "User-Agent: $GLOBALPING_USER_AGENT" \
      "https://api.globalping.io/v1/measurements/$id" > "$output"
    status="$(jq -r '.status' "$output")"
    [ "$status" != "in-progress" ] && return 0
    sleep 1
    i=$((i + 1))
  done
  echo "Timed out waiting for Globalping measurement $id" >&2
  return 1
}

print_results() {
  backend="$1"
  file="$2"
  jq -r --arg backend "$backend" '
    .results[] |
    [
      $backend,
      (.probe.city // ""),
      (.probe.country // ""),
      (.probe.continent // ""),
      (.probe.network // ""),
      (.result.status // ""),
      (.result.statusCode // ""),
      (.result.timings.dns // ""),
      (.result.timings.tcp // ""),
      (.result.timings.tls // ""),
      (.result.timings.firstByte // ""),
      (.result.timings.total // ""),
      (if .result.status == "failed" then (.result.rawOutput // "") else "" end)
    ] | @tsv' "$file"
}

meili_id="$(create_measurement "$MEILI_API_HOST")"
typesense_cloud_id="$(create_measurement "$TYPESENSE_CLOUD_API_HOST")"
typesense_fly_id="$(create_measurement "$TYPESENSE_FLY_API_HOST")"

meili_file="/tmp/ytsearch-globalping-meili-$meili_id.json"
typesense_cloud_file="/tmp/ytsearch-globalping-typesense-cloud-$typesense_cloud_id.json"
typesense_fly_file="/tmp/ytsearch-globalping-typesense-fly-$typesense_fly_id.json"

poll_measurement "$meili_id" "$meili_file"
poll_measurement "$typesense_cloud_id" "$typesense_cloud_file"
poll_measurement "$typesense_fly_id" "$typesense_fly_file"

printf 'backend\tcity\tcountry\tcontinent\tnetwork\tstatus\thttp\tdns_ms\ttcp_ms\ttls_ms\tttfb_ms\ttotal_ms\traw\n'
print_results meilisearch "$meili_file"
print_results typesense-cloud "$typesense_cloud_file"
print_results typesense-fly "$typesense_fly_file"
