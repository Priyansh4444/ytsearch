#!/usr/bin/env bash
set -euo pipefail

# Replay exported production searches through a running API and record the
# Raw reader's server-side time for each request.
#
# usage: replay-production-searches.sh <replay.tsv> <output.jsonl>
#   API_URL      API base, e.g. http://127.0.0.1:18200 (required)
#   READER_URL   Raw reader base whose /metrics the API's backend reports to
#   RAW_SEARCH_API_KEY  reader search key for /metrics
#
# Timing is the delta of the reader's cumulative search_nanos around each
# request, so nothing else may query that reader during the run. The API asks
# the reader for a 60-second response cache: restart the reader (or wait out
# the TTL) between passes that should measure uncached searches. Output rows
# contain the query text; keep them outside Git.

input="${1:?usage: replay-production-searches.sh <replay.tsv> <output.jsonl>}"
output="${2:?usage: replay-production-searches.sh <replay.tsv> <output.jsonl>}"
: "${API_URL:?set API_URL}"
: "${READER_URL:?set READER_URL}"
: "${RAW_SEARCH_API_KEY:?set RAW_SEARCH_API_KEY}"

search_nanos() {
  curl -fsS -H "x-raw-search-key: $RAW_SEARCH_API_KEY" "$READER_URL/metrics" \
    | jq -r .search_nanos
}

umask 077
: > "$output"
row=0
# Unit-separator splitting keeps empty youtuber fields in place.
while IFS=$'\x1f' read -r query youtuber sort page _searches _p50 production_max _found _last; do
  row=$((row + 1))
  request=(-sS -o /dev/null -w '%{http_code}' -G "$API_URL/api/search"
    --data-urlencode "q=$query" --data-urlencode "sort=$sort" --data-urlencode "page=$page")
  [ -n "$youtuber" ] && request+=(--data-urlencode "youtuber=$youtuber")
  before=$(search_nanos)
  status=$(curl "${request[@]}")
  after=$(search_nanos)
  jq -nc --argjson row "$row" --arg q "$query" --arg youtuber "$youtuber" --arg sort "$sort" \
    --argjson page "$page" --argjson http "$status" \
    --argjson reader_ms "$(( (after - before) / 1000000 ))" \
    --argjson production_max_ms "${production_max:-0}" \
    '{row:$row,q:$q,youtuber:$youtuber,sort:$sort,page:$page,http:$http,reader_ms:$reader_ms,production_max_ms:$production_max_ms}' \
    >> "$output"
done < <(tr '\t' '\037' < "$input")

jq -s -r '
  def pct(p): sort | .[((length - 1) * p | floor)];
  map(.reader_ms) as $ms
  | "rows=\(length) http=\(map(.http) | group_by(.) | map("\(.[0]):\(length)") | join(",")) p50_ms=\($ms | pct(0.5)) p99_ms=\($ms | pct(0.99)) max_ms=\($ms | max) over_200ms=\($ms | map(select(. > 200)) | length)"
' "$output"
