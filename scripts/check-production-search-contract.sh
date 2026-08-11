#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${PRODUCTION_FRONTEND_URL:-https://search.pronsh.dev}"
QUERY="${QUERY:-blackhole}"
PER_PAGE="${PER_PAGE:-10}"
CURL_MAX_TIME="${CURL_MAX_TIME:-30}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

search() {
  local output="$1"
  shift
  curl --fail --silent --show-error --max-time "$CURL_MAX_TIME" --get \
    "${BASE_URL%/}/api/search" \
    --data-urlencode "q=$QUERY" \
    --data-urlencode "per_page=$PER_PAGE" \
    "$@" \
    --output "$output"
}

search "$tmp/relevance.json"
jq -e '
  .sort == "relevance" and
  .total_videos > 0 and
  .total_moments >= .total_videos and
  (.totals_approximate | not) and
  (.videos | length) > 0 and
  (.has_next_page | type) == "boolean"
' "$tmp/relevance.json" >/dev/null

for creator in veritasium vsauce; do
  search "$tmp/$creator.json" --data-urlencode "youtuber=@$creator"
  jq -e --arg handle "@$creator" '
    .total_videos > 0 and
    all(.videos[].youtuber_handle; ascii_downcase == $handle)
  ' "$tmp/$creator.json" >/dev/null
done

for sort in relevance_engagement views_desc likes_desc comments_desc published_desc published_asc; do
  search "$tmp/$sort.json" --data-urlencode "sort=$sort"
  jq -e --arg sort "$sort" '.sort == $sort and (.videos | length) > 0' \
    "$tmp/$sort.json" >/dev/null
done

for pair in 'views_desc:view_count' 'likes_desc:like_count' 'comments_desc:comment_count'; do
  sort="${pair%%:*}"
  metric="${pair#*:}"
  jq -e --arg metric "$metric" '
    [.videos[][$metric]] as $values |
    all($values[]; type == "number") and
    all(range(1; $values | length); $values[. - 1] >= $values[.])
  ' "$tmp/$sort.json" >/dev/null
done

jq -e '
  [.videos[].published_at | fromdateiso8601] as $values |
  all(range(1; $values | length); $values[. - 1] >= $values[.])
' "$tmp/published_desc.json" >/dev/null
jq -e '
  [.videos[].published_at | fromdateiso8601] as $values |
  all(range(1; $values | length); $values[. - 1] <= $values[.])
' "$tmp/published_asc.json" >/dev/null

total_videos="$(jq -r '.total_videos' "$tmp/relevance.json")"
last_page="$(( (total_videos + PER_PAGE - 1) / PER_PAGE ))"
search "$tmp/last-page.json" --data-urlencode "page=$last_page"
jq -e --argjson page "$last_page" '
  .page == $page and
  (.videos | length) > 0 and
  (.videos | length) <= .per_page and
  .has_next_page == false
' "$tmp/last-page.json" >/dev/null

invalid_sort_status="$(curl --silent --show-error --max-time "$CURL_MAX_TIME" \
  --output "$tmp/invalid-sort.txt" --write-out '%{http_code}' --get \
  "${BASE_URL%/}/api/search" \
  --data-urlencode "q=$QUERY" \
  --data-urlencode 'sort=unsupported_sort')"
[[ "$invalid_sort_status" == "400" ]]

invalid_video_status="$(curl --silent --show-error --max-time "$CURL_MAX_TIME" \
  --output "$tmp/invalid-video.txt" --write-out '%{http_code}' --get \
  "${BASE_URL%/}/api/search" \
  --data-urlencode "q=$QUERY" \
  --data-urlencode 'video_id=../../etc/passwd')"
[[ "$invalid_video_status" == "400" ]]

jq -n \
  --slurpfile relevance "$tmp/relevance.json" \
  --slurpfile veritasium "$tmp/veritasium.json" \
  --slurpfile vsauce "$tmp/vsauce.json" \
  --argjson last_page "$last_page" \
  '{
    query:$relevance[0].query,
    total_videos:$relevance[0].total_videos,
    total_moments:$relevance[0].total_moments,
    veritasium_videos:$veritasium[0].total_videos,
    vsauce_videos:$vsauce[0].total_videos,
    last_page:$last_page,
    checks:{creator_scope:true,sorts:true,exact_totals:true,last_page:true,fail_closed:true}
  }'
