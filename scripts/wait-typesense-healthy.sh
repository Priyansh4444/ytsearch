#!/usr/bin/env sh
set -eu

APP_NAME="${TYPESENSE_APP_NAME:-ytsearch-typesense-staging}"
HEALTH_URL="${TYPESENSE_HEALTH_URL:-https://ytsearch-typesense-staging.fly.dev/health}"
INTERVAL_SECONDS="${INTERVAL_SECONDS:-10}"
REQUIRED_HEALTHY_CHECKS="${REQUIRED_HEALTHY_CHECKS:-3}"
MAX_CHECKS="${MAX_CHECKS:-0}"

healthy=0
checks=0

echo "Waiting for $HEALTH_URL to pass $REQUIRED_HEALTHY_CHECKS consecutive checks"
while :; do
  checks=$((checks + 1))
  body_file="/tmp/ytsearch-typesense-health.$$"
  code="$(curl -sS -o "$body_file" -w '%{http_code}' "$HEALTH_URL" || printf '000')"
  body="$(cat "$body_file" 2>/dev/null || true)"
  rm -f "$body_file"

  if [ "$code" = "200" ] && [ "$body" = '{"ok":true}' ]; then
    healthy=$((healthy + 1))
  else
    healthy=0
  fi

  printf 'check=%s http=%s consecutive=%s/%s body=%s\n' \
    "$checks" "$code" "$healthy" "$REQUIRED_HEALTHY_CHECKS" "$body"

  if [ "$healthy" -ge "$REQUIRED_HEALTHY_CHECKS" ]; then
    echo "Typesense is stable and ready."
    exit 0
  fi
  if [ "$MAX_CHECKS" -gt 0 ] && [ "$checks" -ge "$MAX_CHECKS" ]; then
    echo "Typesense did not become ready within $MAX_CHECKS checks." >&2
    flyctl machines list --app "$APP_NAME" 2>/dev/null || true
    exit 1
  fi
  sleep "$INTERVAL_SECONDS"
done
