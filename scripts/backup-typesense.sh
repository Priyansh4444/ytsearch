#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

# Dedicated overrides make it difficult for a migration export to be silently
# redirected to the serving URL when load-env.sh loads production defaults.
API_URL="${TYPESENSE_BACKUP_API_URL:-${TYPESENSE_SELFHOST_API_URL:-${TYPESENSE_API_URL:-}}}"
API_KEY="${TYPESENSE_BACKUP_API_KEY:-${TYPESENSE_SELFHOST_API_KEY:-${TYPESENSE_ADMIN_API_KEY:-}}}"
BACKUP_ROOT="${BACKUP_ROOT:-backups}"
STAMP="${BACKUP_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
BACKUP_DIR="$BACKUP_ROOT/$STAMP"
COLLECTIONS="${TYPESENSE_BACKUP_COLLECTIONS:-youtube_transcript_chunks youtube_youtubers}"

if [ -z "$API_URL" ] || [ -z "$API_KEY" ]; then
  echo "Missing Typesense API URL or admin key" >&2
  exit 1
fi
if printf '%s' "$API_URL" | grep -q 'ytsearch-typesense-staging.fly.dev' && [ "${ALLOW_SERVING_EXPORT:-0}" != "1" ]; then
  echo "Refusing to export from serving Typesense; restore/fork a snapshot and point TYPESENSE_SELFHOST_API_URL at the isolated instance" >&2
  exit 1
fi

# Keep the admin key out of process listings and diagnostic command output.
AUTH_HEADER_FILE="$(mktemp)"
chmod 600 "$AUTH_HEADER_FILE"
printf 'X-TYPESENSE-API-KEY: %s\n' "$API_KEY" >"$AUTH_HEADER_FILE"
cleanup() {
  rm -f "$AUTH_HEADER_FILE"
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

mkdir -p "$BACKUP_DIR"
: >"$BACKUP_DIR/COUNTS.tsv"
for collection in $COLLECTIONS; do
  curl -fsS --retry 3 --retry-all-errors \
    -H "@$AUTH_HEADER_FILE" \
    "${API_URL%/}/collections/$collection" |
    jq 'del(.created_at, .num_documents)' >"$BACKUP_DIR/$collection.schema.json"
  output="$BACKUP_DIR/$collection.jsonl.gz"
  curl -fsS --retry 3 --retry-all-errors \
    -H "@$AUTH_HEADER_FILE" \
    -H 'Accept-Encoding: gzip' \
    --output "$output" \
    "${API_URL%/}/collections/$collection/documents/export"
  gzip -t "$output"
  printf '%s\t%s\n' "$collection" "$(gzip -dc "$output" | wc -l)" >>"$BACKUP_DIR/COUNTS.tsv"
done

(
  cd "$BACKUP_DIR"
  sha256sum ./*.jsonl.gz ./*.schema.json COUNTS.tsv >SHA256SUMS
)

jq -n \
  --arg created_at "$STAMP" \
  --arg collections "$COLLECTIONS" \
  '{created_at:$created_at,format:"typesense-jsonl-gzip-v2",collections:($collections | split(" "))}' \
  >"$BACKUP_DIR/manifest.json"

echo "$BACKUP_DIR"
