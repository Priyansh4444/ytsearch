#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

BACKUP_DIR="${1:-}"
API_URL="${TYPESENSE_SELFHOST_API_URL:-${TYPESENSE_API_URL:-}}"
API_KEY="${TYPESENSE_SELFHOST_API_KEY:-${TYPESENSE_ADMIN_API_KEY:-}}"

if [ -z "$BACKUP_DIR" ] || [ ! -d "$BACKUP_DIR" ]; then
  echo "Usage: $0 <backup-directory>" >&2
  exit 1
fi
if [ -z "$API_URL" ] || [ -z "$API_KEY" ]; then
  echo "Missing Typesense API URL or admin key" >&2
  exit 1
fi

(cd "$BACKUP_DIR" && sha256sum -c SHA256SUMS)

for archive in "$BACKUP_DIR"/*.jsonl.gz; do
  collection="$(basename "$archive" .jsonl.gz)"
  echo "Restoring $collection" >&2
  gzip -dc "$archive" | curl -fsS --retry 3 --retry-all-errors \
    -H "X-TYPESENSE-API-KEY: $API_KEY" \
    -H "Content-Type: text/plain" \
    --data-binary @- \
    "${API_URL%/}/collections/$collection/documents/import?action=upsert" |
    awk 'BEGIN {failed=0} /"success":false/ {failed++} END {if (failed) {print failed " rows failed" > "/dev/stderr"; exit 1}}'
done

echo "Restore completed from $BACKUP_DIR"
