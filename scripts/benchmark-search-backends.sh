#!/usr/bin/env sh
set -eu

# Compare Meilisearch, Typesense Cloud, and self-hosted Typesense API endpoints.
# Local run:
#   scripts/benchmark-search-backends.sh
#
# Multi-region run through Fly SSH:
#   FLY_REGIONS="sjc iad lhr nrt syd" \
#   BENCH_FLY_APP=ytsearch-api-meili-staging \
#   scripts/benchmark-search-backends.sh

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

QUERY="${QUERY:-react}"
YOUTUBER="${YOUTUBER:-}"
RUNS="${RUNS:-20}"
PER_PAGE="${PER_PAGE:-10}"
MEILI_API_BASE_URL="${MEILI_API_BASE_URL:-}"
TYPESENSE_CLOUD_API_BASE_URL="${TYPESENSE_CLOUD_API_BASE_URL:-${TYPESENSE_API_BASE_URL:-}}"
TYPESENSE_FLY_API_BASE_URL="${TYPESENSE_FLY_API_BASE_URL:-}"
FLY_REGIONS="${FLY_REGIONS:-}"
BENCH_FLY_APP="${BENCH_FLY_APP:-ytsearch-api-meili-staging}"
BENCH_FLY_ORG="${BENCH_FLY_ORG:-pronsh-sandbox}"

required_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required environment variable: $name" >&2
    exit 1
  fi
}

required_env MEILI_API_BASE_URL
required_env TYPESENSE_CLOUD_API_BASE_URL
required_env TYPESENSE_FLY_API_BASE_URL

bench_one() {
  label="$1"
  base_url="$2"
  API_BASE_URL="$base_url" QUERY="$QUERY" YOUTUBER="$YOUTUBER" RUNS="$RUNS" PER_PAGE="$PER_PAGE" \
    sh "$(dirname "$0")/benchmark-search.sh" | sed "s/^/backend=$label /"
}

bench_local() {
  echo "location=local"
  bench_one meilisearch "$MEILI_API_BASE_URL"
  bench_one typesense-cloud "$TYPESENSE_CLOUD_API_BASE_URL"
  bench_one typesense-fly "$TYPESENSE_FLY_API_BASE_URL"
}

quote_remote() {
  printf "%s" "$1" | sed "s/'/'\\\\''/g"
}

remote_command() {
  cat <<EOF
QUERY='$(quote_remote "$QUERY")'
YOUTUBER='$(quote_remote "$YOUTUBER")'
RUNS='$RUNS'
PER_PAGE='$PER_PAGE'
MEILI_API_BASE_URL='$(quote_remote "$MEILI_API_BASE_URL")'
TYPESENSE_CLOUD_API_BASE_URL='$(quote_remote "$TYPESENSE_CLOUD_API_BASE_URL")'
TYPESENSE_FLY_API_BASE_URL='$(quote_remote "$TYPESENSE_FLY_API_BASE_URL")'
bench() {
  label="\$1"
  base_url="\$2"
  query=\$(printf '%s' "\$QUERY" | jq -sRr @uri)
  url="\$base_url/api/search?q=\$query&per_page=\$PER_PAGE"
  if [ -n "\$YOUTUBER" ]; then
    yt=\$(printf '%s' "\$YOUTUBER" | jq -sRr @uri)
    url="\$url&youtuber=\$yt"
  fi
  i=1
  while [ "\$i" -le "\$RUNS" ]; do
    curl -sS -o /tmp/ytsearch-benchmark-response.json \
      -w "backend=\$label run=\$i http=%{http_code} dns=%{time_namelookup}s connect=%{time_connect}s tls=%{time_appconnect}s ttfb=%{time_starttransfer}s total=%{time_total}s\n" \
      "\$url"
    i=\$((i + 1))
  done
}
bench meilisearch "\$MEILI_API_BASE_URL"
bench typesense-cloud "\$TYPESENSE_CLOUD_API_BASE_URL"
bench typesense-fly "\$TYPESENSE_FLY_API_BASE_URL"
EOF
}

if [ -z "$FLY_REGIONS" ]; then
  bench_local
  exit 0
fi

if [ -n "${FLYIO_TOKEN:-}" ]; then
  export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"
elif [ -z "${FLY_ACCESS_TOKEN:-}" ]; then
  echo "Set FLYIO_TOKEN or FLY_ACCESS_TOKEN for Fly multi-region benchmarks." >&2
  exit 1
fi

for region in $FLY_REGIONS; do
  echo "location=fly:$region"
  flyctl ssh console \
    --app "$BENCH_FLY_APP" \
    --org "$BENCH_FLY_ORG" \
    --region "$region" \
    --command "sh -lc $(printf '%s' "$(remote_command)" | jq -sRr @sh)"
done
