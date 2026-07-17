#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

API_URL="${TYPESENSE_SELFHOST_API_URL:-${TYPESENSE_API_URL:-}}"
API_KEY="${TYPESENSE_SELFHOST_API_KEY:-${TYPESENSE_ADMIN_API_KEY:-}}"
BACKUP_ROOT="${BACKUP_ROOT:-backups}"
STAMP="${BACKUP_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
BACKUP_DIR="$BACKUP_ROOT/$STAMP"
COLLECTIONS="${TYPESENSE_BACKUP_COLLECTIONS:-youtube_transcript_chunks youtube_youtubers}"
GZIP_LEVEL="${GZIP_LEVEL:-6}"

if [ -z "$API_URL" ] || [ -z "$API_KEY" ]; then
  echo "Missing Typesense API URL or admin key" >&2
  exit 1
fi
if printf '%s' "$API_URL" | grep -q 'ytsearch-typesense-staging.fly.dev' && [ "${ALLOW_SERVING_EXPORT:-0}" != "1" ]; then
  echo "Refusing to export from serving Typesense; restore/fork a snapshot and point TYPESENSE_SELFHOST_API_URL at the isolated instance" >&2
  exit 1
fi

mkdir -p "$BACKUP_DIR"
: >"$BACKUP_DIR/COUNTS.tsv"
for collection in $COLLECTIONS; do
  output="$BACKUP_DIR/$collection.jsonl.gz"
  curl -fsS --retry 3 --retry-all-errors \
    -H "X-TYPESENSE-API-KEY: $API_KEY" \
    "${API_URL%/}/collections/$collection/documents/export" | gzip "-$GZIP_LEVEL" >"$output"
  gzip -t "$output"
  printf '%s\t%s\n' "$collection" "$(gzip -dc "$output" | wc -l)" >>"$BACKUP_DIR/COUNTS.tsv"
done

(
  cd "$BACKUP_DIR"
  sha256sum ./*.jsonl.gz >SHA256SUMS
)

cat >"$BACKUP_DIR/manifest.json" <<EOF
{"created_at":"$STAMP","format":"typesense-jsonl-gzip","collections":"$COLLECTIONS"}
EOF

echo "$BACKUP_DIR"
