#!/usr/bin/env bash
set -euo pipefail

: "${RAW_URL:?set RAW_URL to the Raw engine base URL}"
RAW_API_KEY="${RAW_API_KEY:-}"
OUTPUT="${OUTPUT:-.search-experiment/results/raw-contract.json}"
mkdir -p "$(dirname "$OUTPUT")"

RAW_CHECK_TMP="$(mktemp -d)"
trap 'rm -rf "$RAW_CHECK_TMP"' EXIT

search() {
  local route="$1"
  local query="$2"
  local handle="$3"
  local sort_by="$4"
  local offset="$5"
  local limit="$6"
  local output="$7"
  local args=(
    --fail --silent --show-error --get
    "${RAW_URL%/}${route}"
    --data-urlencode "q=$query"
    --data-urlencode 'query_by=text,title'
    --data-urlencode 'group_by=video_id'
    --data-urlencode 'group_limit=5'
    --data-urlencode "offset=$offset"
    --data-urlencode "limit=$limit"
    --output "$output"
  )
  if [[ -n "$handle" ]]; then
    args+=(--data-urlencode "filter_by=youtuber_handle:=$handle")
  fi
  if [[ -n "$sort_by" ]]; then
    args+=(--data-urlencode "sort_by=$sort_by")
  fi
  if [[ -n "$RAW_API_KEY" ]]; then
    args+=(-H "X-TYPESENSE-API-KEY: $RAW_API_KEY")
  fi
  curl "${args[@]}"
}

TRANSCRIPT_ROUTE='/collections/youtube_transcript_chunks/documents/search'
RAW_TRANSCRIPT_ROUTE='/raw/collections/youtube_transcript_chunks/documents/search'
CHANNEL_ROUTE='/collections/youtube_youtubers/documents/search'

search "$TRANSCRIPT_ROUTE" blackhole '' '' 0 10 "$RAW_CHECK_TMP/global.json"
search "$RAW_TRANSCRIPT_ROUTE" blackhole '' '' 0 10 "$RAW_CHECK_TMP/raw-alias.json"
cmp "$RAW_CHECK_TMP/global.json" "$RAW_CHECK_TMP/raw-alias.json"
jq -e '.found > 0 and (.grouped_hits | length) > 0' "$RAW_CHECK_TMP/global.json" >/dev/null

exact_blackhole_top="$(jq -r '.grouped_hits[0].group_key[0]' "$RAW_CHECK_TMP/global.json")"
for typo in blackhloe blakhole blackhle; do
  search "$TRANSCRIPT_ROUTE" "$typo" '' '' 0 10 "$RAW_CHECK_TMP/$typo.json"
  jq -e '.found > 0 and (.grouped_hits | length) > 0' "$RAW_CHECK_TMP/$typo.json" >/dev/null
  typo_top="$(jq -r '.grouped_hits[0].group_key[0]' "$RAW_CHECK_TMP/$typo.json")"
  [[ "$typo_top" == "$exact_blackhole_top" ]]
done

for creator in veritasium vsauce; do
  search "$TRANSCRIPT_ROUTE" blackhole "@$creator" '' 0 10 "$RAW_CHECK_TMP/$creator.json"
  jq -e --arg handle "@$creator" '
    .found > 0 and
    all(.grouped_hits[].hits[].document.youtuber_handle; ascii_downcase == $handle)
  ' "$RAW_CHECK_TMP/$creator.json" >/dev/null
done

search "$TRANSCRIPT_ROUTE" blackhole '' 'published_at_timestamp:desc' 0 10 "$RAW_CHECK_TMP/newest.json"
search "$TRANSCRIPT_ROUTE" blackhole '' 'published_at_timestamp:asc' 0 10 "$RAW_CHECK_TMP/oldest.json"
search "$TRANSCRIPT_ROUTE" blackhole '' 'engagement_score:desc' 0 10 "$RAW_CHECK_TMP/engagement.json"
search "$TRANSCRIPT_ROUTE" blackhole '' 'view_count:desc' 0 10 "$RAW_CHECK_TMP/views.json"
search "$TRANSCRIPT_ROUTE" blackhole '' 'like_count:desc' 0 10 "$RAW_CHECK_TMP/likes.json"
search "$TRANSCRIPT_ROUTE" blackhole '' 'comment_count:desc' 0 10 "$RAW_CHECK_TMP/comments.json"
jq -e '
  def timestamp:
    if .published_at_timestamp != null then .published_at_timestamp
    elif .published_at != null then (.published_at | fromdateiso8601)
    else 0 end;
  [.grouped_hits[].hits[0].document | timestamp] as $timestamps |
  all(range(1; $timestamps | length); $timestamps[. - 1] >= $timestamps[.])
' "$RAW_CHECK_TMP/newest.json" >/dev/null
jq -e '
  def timestamp:
    if .published_at_timestamp != null then .published_at_timestamp
    elif .published_at != null then (.published_at | fromdateiso8601)
    else 0 end;
  [.grouped_hits[].hits[0].document | timestamp] as $timestamps |
  all(range(1; $timestamps | length); $timestamps[. - 1] <= $timestamps[.])
' "$RAW_CHECK_TMP/oldest.json" >/dev/null
jq -e --slurpfile oldest "$RAW_CHECK_TMP/oldest.json" '
  [.grouped_hits[].group_key[0]] != [$oldest[0].grouped_hits[].group_key[0]]
' "$RAW_CHECK_TMP/newest.json" >/dev/null

for metric in view_count like_count comment_count; do
  result_file="$RAW_CHECK_TMP/${metric%_count}s.json"
  [[ "$metric" != comment_count ]] || result_file="$RAW_CHECK_TMP/comments.json"
  jq -e --arg metric "$metric" '
    [.grouped_hits[].hits[0].document[$metric]] as $values |
    ($values | length) > 0 and
    all($values[]; type == "number") and
    all(range(1; $values | length); $values[. - 1] >= $values[.])
  ' "$result_file" >/dev/null
done
jq -e '
  (.grouped_hits | length) > 0 and
  all(.grouped_hits[].hits[0].document;
    (.view_count | type) == "number" and
    (.like_count | type) == "number" and
    (.comment_count | type) == "number")
' "$RAW_CHECK_TMP/engagement.json" >/dev/null

search "$TRANSCRIPT_ROUTE" blackhole '' '' 0 1 "$RAW_CHECK_TMP/page-one.json"
search "$TRANSCRIPT_ROUTE" blackhole '' '' 1 1 "$RAW_CHECK_TMP/page-two.json"
if jq -e '.found > 1' "$RAW_CHECK_TMP/global.json" >/dev/null; then
  first_id="$(jq -r '.grouped_hits[0].group_key[0]' "$RAW_CHECK_TMP/page-one.json")"
  second_id="$(jq -r '.grouped_hits[0].group_key[0]' "$RAW_CHECK_TMP/page-two.json")"
  [[ "$first_id" != "$second_id" ]]
fi

search "$CHANNEL_ROUTE" veritasium '' '' 0 10 "$RAW_CHECK_TMP/channel.json"
jq -e '.found > 0 and any(.hits[].document.handle; ascii_downcase == "@veritasium")' \
  "$RAW_CHECK_TMP/channel.json" >/dev/null

if [[ -n "$RAW_API_KEY" ]]; then
  unauthorized_status="$(curl --silent --output /dev/null --write-out '%{http_code}' --get \
    "${RAW_URL%/}${TRANSCRIPT_ROUTE}" --data-urlencode 'q=blackhole')"
  [[ "$unauthorized_status" == 401 ]]
fi

jq -n \
  --slurpfile global "$RAW_CHECK_TMP/global.json" \
  --slurpfile veritasium "$RAW_CHECK_TMP/veritasium.json" \
  --slurpfile vsauce "$RAW_CHECK_TMP/vsauce.json" \
  --slurpfile blackhloe "$RAW_CHECK_TMP/blackhloe.json" \
  --slurpfile blakhole "$RAW_CHECK_TMP/blakhole.json" \
  --slurpfile blackhle "$RAW_CHECK_TMP/blackhle.json" \
  --slurpfile newest "$RAW_CHECK_TMP/newest.json" \
  --slurpfile oldest "$RAW_CHECK_TMP/oldest.json" \
  --slurpfile engagement "$RAW_CHECK_TMP/engagement.json" \
  --slurpfile views "$RAW_CHECK_TMP/views.json" \
  --slurpfile likes "$RAW_CHECK_TMP/likes.json" \
  --slurpfile comments "$RAW_CHECK_TMP/comments.json" '
  {
    query:"blackhole",
    global_found:$global[0].found,
    veritasium_found:$veritasium[0].found,
    vsauce_found:$vsauce[0].found,
    typo_recovery:{
      blackhloe:$blackhloe[0].found,
      blakhole:$blakhole[0].found,
      blackhle:$blackhle[0].found
    },
    relevance_top_10:[$global[0].grouped_hits[].group_key[0]],
    newest_top_10:[$newest[0].grouped_hits[].group_key[0]],
    oldest_top_10:[$oldest[0].grouped_hits[].group_key[0]],
    engagement_top_10:[$engagement[0].grouped_hits[].group_key[0]],
    views_top_10:[$views[0].grouped_hits[].group_key[0]],
    likes_top_10:[$likes[0].grouped_hits[].group_key[0]],
    comments_top_10:[$comments[0].grouped_hits[].group_key[0]],
    checks:{raw_alias:true,typo_recovery:true,creator_scope:true,date_sort:true,engagement_sort:true,pagination:true,channel_search:true,authentication:true}
  }
' | tee "$OUTPUT"
