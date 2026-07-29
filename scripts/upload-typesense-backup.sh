#!/usr/bin/env sh
set -eu

# Upload one verified backup to both private S3-compatible destinations.
# Requires aws-cli. Credentials are passed only to the child process.
# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

BACKUP_DIR="${1:-}"
BACKUP_PREFIX="${BACKUP_PREFIX:-ytsearch/typesense}"

if [ -z "$BACKUP_DIR" ] || [ ! -d "$BACKUP_DIR" ]; then
  echo "Usage: $0 <backup-directory>" >&2
  exit 1
fi
command -v aws >/dev/null 2>&1 || { echo "aws-cli is required" >&2; exit 1; }
(cd "$BACKUP_DIR" && sha256sum -c SHA256SUMS)
stamp="$(basename "$BACKUP_DIR")"

upload() {
  label="$1"
  endpoint="$2"
  bucket="$3"
  access_key="$4"
  secret_key="$5"
  region="$6"
  if [ -z "$endpoint" ] || [ -z "$bucket" ] || [ -z "$access_key" ] || [ -z "$secret_key" ]; then
    echo "Missing $label endpoint, bucket, or credentials" >&2
    exit 1
  fi
  echo "Uploading $stamp to $label" >&2
  AWS_ACCESS_KEY_ID="$access_key" AWS_SECRET_ACCESS_KEY="$secret_key" \
    AWS_DEFAULT_REGION="$region" aws s3 sync "$BACKUP_DIR/" \
    "s3://$bucket/$BACKUP_PREFIX/$stamp/" --endpoint-url "$endpoint" \
    --only-show-errors
}

upload R2 "${R2_S3_ENDPOINT:-}" "${R2_BACKUP_BUCKET:-}" \
  "${R2_ACCESS_KEY_ID:-}" "${R2_SECRET_ACCESS_KEY:-}" "${R2_REGION:-auto}"
upload B2 "${B2_S3_ENDPOINT:-}" "${B2_BACKUP_BUCKET:-}" \
  "${B2_ACCESS_KEY_ID:-}" "${B2_SECRET_ACCESS_KEY:-}" "${B2_REGION:-us-west-004}"

echo "Uploaded $stamp to R2 and B2"
