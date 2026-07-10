#!/usr/bin/env sh
set -eu

# Compare active search API endpoints.
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
TYPESENSE_FLY_API_BASE_URL="${TYPESENSE_FLY_API_BASE_URL:-}"
BACKENDS="${BACKENDS:-meilisearch typesense-fly}"
FLY_REGIONS="${FLY_REGIONS:-}"
BENCH_FLY_APP="${BENCH_FLY_APP:-ytsearch-api-meili-staging}"
BENCH_FLY_ORG="${BENCH_FLY_ORG:-pronsh-sandbox}"
SUMMARY="${SUMMARY:-1}"
RESULTS_FILE="${RESULTS_FILE:-/tmp/ytsearch-benchmark-backends.$$.log}"

required_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required environment variable: $name" >&2
    exit 1
  fi
}

has_backend() {
  case " $BACKENDS " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}

if has_backend meilisearch; then
  required_env MEILI_API_BASE_URL
fi
if has_backend typesense-fly; then
  required_env TYPESENSE_FLY_API_BASE_URL
fi

bench_one() {
  label="$1"
  base_url="$2"
  API_BASE_URL="$base_url" QUERY="$QUERY" YOUTUBER="$YOUTUBER" RUNS="$RUNS" PER_PAGE="$PER_PAGE" \
    sh "$(dirname "$0")/benchmark-search.sh" | sed "s/^/backend=$label /"
}

record() {
  tee -a "$RESULTS_FILE"
}

summarize() {
  if [ "$SUMMARY" = "1" ]; then
    echo "summary_file=$RESULTS_FILE"
    "$(dirname "$0")/summarize-benchmark-results.sh" "$RESULTS_FILE"
  fi
}

bench_local() {
  echo "location=local"
  if has_backend meilisearch; then
    bench_one meilisearch "$MEILI_API_BASE_URL" | record
  fi
  if has_backend typesense-fly; then
    bench_one typesense-fly "$TYPESENSE_FLY_API_BASE_URL" | record
  fi
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
CURL_MAX_TIME='${CURL_MAX_TIME:-15}'
MEILI_API_BASE_URL='$(quote_remote "$MEILI_API_BASE_URL")'
TYPESENSE_FLY_API_BASE_URL='$(quote_remote "$TYPESENSE_FLY_API_BASE_URL")'
BACKENDS='$(quote_remote "$BACKENDS")'
has_backend() {
  case " \$BACKENDS " in
    *" \$1 "*) return 0 ;;
    *) return 1 ;;
  esac
}
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
      --max-time "\$CURL_MAX_TIME" \
      -w "backend=\$label run=\$i http=%{http_code} dns=%{time_namelookup}s connect=%{time_connect}s tls=%{time_appconnect}s ttfb=%{time_starttransfer}s total=%{time_total}s\n" \
      "\$url" || true
    i=\$((i + 1))
  done
}
if has_backend meilisearch; then
  bench meilisearch "\$MEILI_API_BASE_URL"
fi
if has_backend typesense-fly; then
  bench typesense-fly "\$TYPESENSE_FLY_API_BASE_URL"
fi
EOF
}

if [ -z "$FLY_REGIONS" ]; then
  bench_local
  summarize
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
    --command "sh -lc $(printf '%s' "$(remote_command)" | jq -sRr @sh)" | record
done

summarize
