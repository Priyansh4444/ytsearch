#!/usr/bin/env sh
set -eu

vite_pid=""

cleanup() {
  if [ -n "$vite_pid" ]; then
    kill "$vite_pid" 2>/dev/null || true
    wait "$vite_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT HUP INT TERM

if [ -z "${PLAYWRIGHT_BASE_URL:-}" ]; then
  port="${PLAYWRIGHT_PORT:-4174}"
  PLAYWRIGHT_BASE_URL="http://127.0.0.1:$port"
  export PLAYWRIGHT_BASE_URL
  VITE_SEMANTIC_SEARCH_ENABLED=true \
    bun ./node_modules/vite/bin/vite.js --host 127.0.0.1 --port "$port" --strictPort \
    </dev/null >/dev/null 2>&1 &
  vite_pid="$!"

  attempt=0
  while ! curl --fail --silent --show-error --max-time 1 "$PLAYWRIGHT_BASE_URL" \
    >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 300 ] || ! kill -0 "$vite_pid" 2>/dev/null; then
      echo "Vite did not become ready at $PLAYWRIGHT_BASE_URL" >&2
      exit 1
    fi
    sleep 0.1
  done
fi

bun ./node_modules/playwright/cli.js test "$@"
