#!/usr/bin/env bash
set -euo pipefail

# Export only from an isolated snapshot clone, never the serving machine.
: "${TYPESENSE_EXPORT_URL:?set TYPESENSE_EXPORT_URL to the snapshot clone URL}"
: "${TYPESENSE_API_KEY:?set TYPESENSE_API_KEY to the snapshot clone key}"

OUTPUT_DIR="${OUTPUT_DIR:-.search-experiment/corpus}"
HANDLES="${HANDLES:-@veritasium,@vsauce}"
TRANSCRIPT_COLLECTION="${TRANSCRIPT_COLLECTION:-youtube_transcript_chunks}"
CHANNEL_COLLECTION="${CHANNEL_COLLECTION:-youtube_youtubers}"

mkdir -p "$OUTPUT_DIR"
chunks_tmp="$(mktemp "$OUTPUT_DIR/chunks.XXXXXX")"
channels_tmp="$(mktemp "$OUTPUT_DIR/channels.XXXXXX")"
trap 'rm -f "$chunks_tmp" "$channels_tmp"' EXIT

include_fields="id,youtuber_id,video_id,title,text,prev_text,next_text,youtuber_handle,youtuber_name,thumbnail,published_at,published_at_timestamp,duration,t_start_ms,t_end_ms"
curl --fail --silent --show-error \
  -H "X-TYPESENSE-API-KEY: $TYPESENSE_API_KEY" \
  --get "${TYPESENSE_EXPORT_URL%/}/collections/$TRANSCRIPT_COLLECTION/documents/export" \
  --data-urlencode "filter_by=youtuber_handle:=[$HANDLES]" \
  --data-urlencode "include_fields=$include_fields" \
  --output "$chunks_tmp"

curl --fail --silent --show-error \
  -H "X-TYPESENSE-API-KEY: $TYPESENSE_API_KEY" \
  --get "${TYPESENSE_EXPORT_URL%/}/collections/$CHANNEL_COLLECTION/documents/export" \
  --data-urlencode "filter_by=handle:=[$HANDLES]" \
  --output "$channels_tmp"

mv "$chunks_tmp" "$OUTPUT_DIR/chunks.jsonl"
mv "$channels_tmp" "$OUTPUT_DIR/channels.jsonl"
trap - EXIT

printf 'chunks=%s videos=%s channels=%s bytes=%s\n' \
  "$(wc -l < "$OUTPUT_DIR/chunks.jsonl")" \
  "$(jq -r .video_id "$OUTPUT_DIR/chunks.jsonl" | sort -u | wc -l)" \
  "$(wc -l < "$OUTPUT_DIR/channels.jsonl")" \
  "$(wc -c < "$OUTPUT_DIR/chunks.jsonl")"
