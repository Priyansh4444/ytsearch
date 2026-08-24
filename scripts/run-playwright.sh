#!/usr/bin/env sh
set -eu

vite_pid=""
vite_log=""

cleanup() {
  if [ -n "$vite_pid" ]; then
    kill "$vite_pid" 2>/dev/null || true
    wait "$vite_pid" 2>/dev/null || true
  fi
  if [ -n "$vite_log" ]; then
    rm -f "$vite_log"
  fi
}
trap cleanup EXIT HUP INT TERM

if [ -z "${PLAYWRIGHT_BASE_URL:-}" ]; then
  port="${PLAYWRIGHT_PORT:-4174}"
  PLAYWRIGHT_BASE_URL="http://127.0.0.1:$port"
  export PLAYWRIGHT_BASE_URL
  vite_log="$(mktemp)"
  node_path="$PATH"
  case "$node_path" in
    "${NODE:-__no_bun_node_shim__}":*) node_path="${node_path#*:}" ;;
  esac
  VITE_SEMANTIC_SEARCH_ENABLED=true \
    env PATH="$node_path" node ./node_modules/vite/bin/vite.js \
    --host 127.0.0.1 --port "$port" --strictPort \
    </dev/null >"$vite_log" 2>&1 &
  vite_pid="$!"

  attempt=0
  while ! curl --fail --silent --show-error --max-time 1 "$PLAYWRIGHT_BASE_URL" \
    >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 300 ] || ! kill -0 "$vite_pid" 2>/dev/null; then
      echo "Vite did not become ready at $PLAYWRIGHT_BASE_URL" >&2
      tail -n 50 "$vite_log" >&2
      exit 1
    fi
    sleep 0.1
  done
fi

bun ./node_modules/playwright/cli.js test "$@"
