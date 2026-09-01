#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/bin"
cat > "$tmp/bin/flyctl" <<'EOF'
#!/usr/bin/env sh
printf '%s\n' "$*" >> "$FLYCTL_LOG"
EOF
chmod +x "$tmp/bin/flyctl"

(
  cd "$tmp"
  PATH="$tmp/bin:$PATH" \
  FLYCTL_LOG="$tmp/flyctl.log" \
  FLY_ORG=test-org \
  INDEXER_APP_NAME=test-indexer \
  RAW_SEARCH_ADMIN_API_KEY=test-admin-key \
  YTSEARCH_INDEX_QUEUE_TOKEN=test-queue-token \
  INDEX_QUEUE_API_BASE_URL=https://queue.invalid \
  YOUTUBE_API_FALLBACK=0 \
    "$SCRIPT_DIR/fly-set-indexer-secrets.sh"
)

grep -F 'secrets unset --stage --app test-indexer RAW_SEARCH_API_URL' \
  "$tmp/flyctl.log" >/dev/null
set_call="$(grep -F 'secrets set --stage --app test-indexer' "$tmp/flyctl.log")"
case "$set_call" in
  *RAW_SEARCH_API_URL=*)
    printf 'Raw URL leaked into app-wide secrets\n' >&2
    exit 1
    ;;
esac
grep -F 'RAW_SEARCH_ADMIN_API_KEY=test-admin-key' <<<"$set_call" >/dev/null
