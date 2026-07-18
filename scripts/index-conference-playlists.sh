#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
MANIFEST="${1:-$SCRIPT_DIR/conference-playlists.tsv}"
STATE_DIR="${CONFERENCE_STATE_DIR:-$ROOT_DIR/data/conference-index-state}"
BACKEND="${BACKEND:-typesense}"
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"
TRANSCRIPT_CACHE_DIR="${TRANSCRIPT_CACHE_DIR:-$ROOT_DIR/data/transcript-cache}"

. "$SCRIPT_DIR/load-env.sh"

TYPESENSE_API_URL="${TYPESENSE_API_URL:-${TYPESENSE_SELFHOST_API_URL:-}}"
TYPESENSE_ADMIN_API_KEY="${TYPESENSE_ADMIN_API_KEY:-${TYPESENSE_SELFHOST_API_KEY:-}}"
TYPESENSE_SEARCH_API_KEY="${TYPESENSE_SEARCH_API_KEY:-$TYPESENSE_ADMIN_API_KEY}"
export TYPESENSE_API_URL TYPESENSE_ADMIN_API_KEY TYPESENSE_SEARCH_API_KEY

if [ "$BACKEND" = "typesense" ] && { [ -z "$TYPESENSE_API_URL" ] || [ -z "$TYPESENSE_ADMIN_API_KEY" ]; }; then
  echo "conference-index: Typesense admin URL and key are required" >&2
  exit 1
fi

command -v yt-dlp >/dev/null 2>&1 || {
  echo "yt-dlp is required" >&2
  exit 1
}

mkdir -p "$STATE_DIR" "$TRANSCRIPT_CACHE_DIR"

while IFS="$(printf '\t')" read -r target playlist_url; do
  case "$target" in ''|'#'*) continue ;; esac
  completed="$STATE_DIR/$target.completed"
  touch "$completed"
  echo "conference-index: discovering target=$target playlist=$playlist_url"
  yt-dlp --flat-playlist --print '%(id)s' "$playlist_url" |
    while IFS= read -r video_id; do
      [ -n "$video_id" ] || continue
      if grep -Fxq -- "$video_id" "$completed"; then
        continue
      fi
      echo "conference-index: indexing target=$target video=$video_id"
      if cargo run --release -p youtube_cli -- index-video \
        --backend "$BACKEND" \
        --collection "$COLLECTION" \
        --no-wipe \
        --transcript-cache-dir "$TRANSCRIPT_CACHE_DIR" \
        -- "$video_id"; then
        printf '%s\n' "$video_id" >> "$completed"
      else
        echo "conference-index: failed target=$target video=$video_id; continuing" >&2
      fi
    done
done < "$MANIFEST"
