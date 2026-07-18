#!/usr/bin/env sh
set -eu

# Remove display-only fields from Typesense's in-memory indexes while retaining
# their stored values. Reads remain available; Typesense blocks writes during
# each synchronous field alteration.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/load-env.sh"

base="${TYPESENSE_SELFHOST_API_URL:-}"
key="${TYPESENSE_SELFHOST_API_KEY:-}"
collection="${COLLECTION:-youtube_transcript_chunks}"
if [ -z "$base" ] || [ -z "$key" ]; then
  echo "TYPESENSE_SELFHOST_API_URL and TYPESENSE_SELFHOST_API_KEY are required." >&2
  exit 1
fi
if [ "${ALLOW_SCHEMA_MEMORY_OPTIMIZATION:-0}" != "1" ]; then
  echo "Refusing schema alteration without ALLOW_SCHEMA_MEMORY_OPTIMIZATION=1." >&2
  exit 1
fi
if pgrep -f 'ytsearch (index-video|index-youtuber|index-youtubers)' >/dev/null 2>&1; then
  echo "Refusing schema alteration while a local ytsearch index process is active." >&2
  exit 1
fi

api() {
  path="$1"
  shift
  curl -fsS --max-time 3600 -H "X-TYPESENSE-API-KEY: $key" "$@" "${base%/}/$path"
}

if [ "$(api operations/schema_changes | jq 'length')" -ne 0 ]; then
  echo "A Typesense schema change is already active." >&2
  exit 1
fi

memory_mb() {
  api metrics.json | jq '(.typesense_memory_resident_bytes | tonumber) / 1048576 | floor'
}

before_mb="$(memory_mb)"
printf 'resident_before_mb=%s\n' "$before_mb"
changed=0
max_fields="${MAX_FIELDS:-0}"

while IFS=' ' read -r name type; do
  current="$(api "collections/$collection" | jq -r --arg name "$name" '.fields[] | select(.name == $name) | .index')"
  if [ "$current" = "false" ]; then
    printf 'field=%s status=already_disk_only\n' "$name"
    continue
  fi
  body="$(jq -nc --arg name "$name" --arg type "$type" '{fields:[{name:$name,drop:true},{name:$name,type:$type,index:false,optional:true}]}')"
  api "collections/$collection" -X PATCH -H 'Content-Type: application/json' -d "$body" >/dev/null
  printf 'field=%s status=disk_only\n' "$name"
  curl -fsS --retry 20 --retry-delay 2 --retry-all-errors "${base%/}/health" >/dev/null
  changed=$((changed + 1))
  if [ "$max_fields" -gt 0 ] && [ "$changed" -ge "$max_fields" ]; then
    break
  fi
done <<'EOF'
prev_text string
next_text string
channel_id string
sequence int32
chapter string
chapter_index int32
t_start_ms int32
t_end_ms int32
thumbnail string
published_at string
upload_date string
uploader_id string
EOF

after_mb="$(memory_mb)"
printf 'resident_after_mb=%s\n' "$after_mb"
printf 'resident_reduction_mb=%s\n' "$((before_mb - after_mb))"
