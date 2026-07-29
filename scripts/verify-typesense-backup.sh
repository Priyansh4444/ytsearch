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
failed=0
while IFS="$(printf '\t')" read -r collection expected; do
  actual="$(curl -fsS --retry 3 --retry-all-errors \
    -H "X-TYPESENSE-API-KEY: $API_KEY" \
    "${API_URL%/}/collections/$collection" | jq -r '.num_documents')"
  printf '%s\texpected=%s\tactual=%s\n' "$collection" "$expected" "$actual"
  if [ "$actual" != "$expected" ]; then
    failed=1
  fi
done <"$BACKUP_DIR/COUNTS.tsv"

if [ "$failed" -ne 0 ]; then
  echo "Typesense collection count verification failed" >&2
  exit 1
fi
