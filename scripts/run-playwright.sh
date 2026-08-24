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

  node_bin="$(
    old_ifs="$IFS"
    IFS=:
    for directory in $PATH; do
      case "$directory" in
        /tmp/bun-node-*) continue ;;
      esac
      if [ -x "$directory/node" ]; then
        printf '%s\n' "$directory/node"
        break
      fi
    done
    IFS="$old_ifs"
  )"
  if [ -z "$node_bin" ]; then
    echo "System Node executable not found" >&2
    exit 1
  fi

  start_attempt=1
  vite_ready=0
  while [ "$start_attempt" -le 2 ]; do
    : >"$vite_log"
    VITE_SEMANTIC_SEARCH_ENABLED=true \
      "$node_bin" ./node_modules/vite/bin/vite.js \
      --host 127.0.0.1 --port "$port" --strictPort \
      </dev/null >"$vite_log" 2>&1 &
    vite_pid="$!"

    poll=0
    while [ "$poll" -lt 150 ] && kill -0 "$vite_pid" 2>/dev/null; do
      if curl --fail --silent --show-error --max-time 1 "$PLAYWRIGHT_BASE_URL" \
        >/dev/null 2>&1; then
        vite_ready=1
        break
      fi
      poll=$((poll + 1))
      sleep 0.1
    done

    if [ "$vite_ready" -eq 1 ]; then
      break
    fi

    kill "$vite_pid" 2>/dev/null || true
    wait "$vite_pid" 2>/dev/null || true
    vite_pid=""
    start_attempt=$((start_attempt + 1))
  done

  if [ "$vite_ready" -ne 1 ]; then
    echo "Vite did not become ready at $PLAYWRIGHT_BASE_URL after 2 attempts" >&2
    tail -n 50 "$vite_log" >&2
    exit 1
  fi
fi

bun ./node_modules/playwright/cli.js test "$@"
