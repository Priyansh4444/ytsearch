#!/usr/bin/env bash
# usage: raw-schema-differential.sh <replay.tsv> <index-a> <index-b> <out-dir>
#
# Serves two raw generations behind two real API processes and compares the
# complete API response for every replay row (from axiom-export-search-replay.sh)
# plus relevance, newest, and views sorts on pages 1-2 for each unique query and
# creator. Timing fields are the only ones allowed to differ. Exits non-zero on
# any difference or non-200 response.
#
# Binaries default to target/release; override with RAW_SEARCH_BIN and API_BIN.
# The readers use a local-only key and bind to loopback.
set -euo pipefail

replay=${1:?usage: raw-schema-differential.sh <replay.tsv> <index-a> <index-b> <out-dir>}
index_a=${2:?missing index-a}
index_b=${3:?missing index-b}
out=${4:?missing out-dir}
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
reader_bin=${RAW_SEARCH_BIN:-$root/target/release/youtube-raw-search}
api_bin=${API_BIN:-$root/target/release/youtube-api}
key=local-schema-differential
mkdir -p "$out"

units=(rawdiffreadera rawdiffreaderb rawdiffapia rawdiffapib)
stop_units() {
  systemctl --user stop "${units[@]}" 2>/dev/null || true
  systemctl --user reset-failed "${units[@]}" 2>/dev/null || true
}
trap stop_units EXIT
stop_units

start_pair() { # reader-unit reader-bind index api-unit api-addr
  systemd-run --user --unit="$1" --setenv=RAW_SEARCH_BIND="$2" \
    --setenv=RAW_SEARCH_API_KEY="$key" \
    -p StandardOutput=file:"$out/$1.log" -p StandardError=file:"$out/$1.log" \
    "$reader_bin" serve "$3" >/dev/null
  until curl -fsS "http://$2/health" >/dev/null 2>&1; do sleep 1; done
  systemd-run --user --unit="$4" --setenv=RAW_SEARCH_API_URL="http://$2" \
    --setenv=RAW_SEARCH_API_KEY="$key" --setenv=YTSEARCH_API_ADDR="$5" \
    --setenv=YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE=0 \
    --setenv=YTSEARCH_ENGAGEMENT_SORTS=true \
    -p StandardOutput=file:"$out/$4.log" -p StandardError=file:"$out/$4.log" \
    "$api_bin" >/dev/null
  until curl -fsS "http://$5/api/health" >/dev/null 2>&1; do sleep 0.5; done
}
start_pair rawdiffreadera '[::1]:18118' "$index_a" rawdiffapia 127.0.0.1:18210
start_pair rawdiffreaderb '[::1]:18119' "$index_b" rawdiffapib 127.0.0.1:18211

# Unit separators keep empty creator fields; tab IFS would collapse them.
{
  tr '\t' '\037' <"$replay" | while IFS=$'\x1f' read -r q creator sort page _; do
    printf '%s\037%s\037%s\037%s\n' "$q" "$creator" "$sort" "$page"
  done
  tr '\t' '\037' <"$replay" | cut -d $'\x1f' -f1,2 | sort -u |
    while IFS=$'\x1f' read -r q creator; do
      for sort in relevance published_desc views_desc; do
        for page in 1 2; do
          printf '%s\037%s\037%s\037%s\n' "$q" "$creator" "$sort" "$page"
        done
      done
    done
} | sort -u >"$out/cases"

normalize='del(.. | .time_to_first_result_ms?, .search_time_ms?)'
cases=0 identical=0 different=0 failed=0
: >"$out/differences.jsonl"
while IFS=$'\x1f' read -r q creator sort page; do
  cases=$((cases + 1))
  args=(-sS -G --data-urlencode "q=$q" --data-urlencode "sort=$sort" --data-urlencode "page=$page")
  [ -n "$creator" ] && args+=(--data-urlencode "youtuber=$creator")
  response_a=$(curl "${args[@]}" -w '\n%{http_code}' http://127.0.0.1:18210/api/search)
  response_b=$(curl "${args[@]}" -w '\n%{http_code}' http://127.0.0.1:18211/api/search)
  status_a=${response_a##*$'\n'} status_b=${response_b##*$'\n'}
  body_a=$(printf '%s\n' "${response_a%$'\n'*}" | jq -cS "$normalize" 2>/dev/null || printf '%s' "$response_a")
  body_b=$(printf '%s\n' "${response_b%$'\n'*}" | jq -cS "$normalize" 2>/dev/null || printf '%s' "$response_b")
  [ "$status_a" = 200 ] && [ "$status_b" = 200 ] || failed=$((failed + 1))
  if [ "$status_a" = "$status_b" ] && [ "$body_a" = "$body_b" ]; then
    identical=$((identical + 1))
  else
    different=$((different + 1))
    if [ "$different" -le 5 ]; then
      printf '%s\n' "$body_a" | jq -S . >"$out/a-$different.json" 2>/dev/null || true
      printf '%s\n' "$body_b" | jq -S . >"$out/b-$different.json" 2>/dev/null || true
    fi
    jq -nc --arg q "$q" --arg creator "$creator" --arg sort "$sort" --arg page "$page" \
      '{q:$q,creator:$creator,sort:$sort,page:$page}' >>"$out/differences.jsonl"
  fi
done <"$out/cases"

echo "cases=$cases identical=$identical different=$different non200=$failed" | tee "$out/summary"
[ "$different" -eq 0 ] && [ "$failed" -eq 0 ]
