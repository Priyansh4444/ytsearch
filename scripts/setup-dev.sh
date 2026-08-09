#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
ENV_EXAMPLE="${YTSEARCH_SETUP_ENV_EXAMPLE:-$ROOT/.env.example}"
ENV_FILE="${YTSEARCH_SETUP_ENV_FILE:-$ROOT/.env}"
SKIP_INSTALL="${YTSEARCH_SETUP_SKIP_INSTALL:-0}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Unable to set up ytsearch: install $1 and run this command again." >&2
    exit 1
  fi
}

require_command openssl
if [ "$SKIP_INSTALL" != "1" ]; then
  require_command bun
  require_command cargo
fi

if [ ! -f "$ENV_FILE" ]; then
  if [ ! -f "$ENV_EXAMPLE" ]; then
    echo "Unable to set up ytsearch: $ENV_EXAMPLE is missing." >&2
    exit 1
  fi
  raw_search_key="$(openssl rand -hex 24)"
  queue_token="$(openssl rand -hex 24)"
  previous_umask="$(umask)"
  umask 077
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      RAW_SEARCH_API_URL=) line="RAW_SEARCH_API_URL=http://127.0.0.1:8108" ;;
      RAW_SEARCH_ADMIN_API_KEY=) line="RAW_SEARCH_ADMIN_API_KEY=$raw_search_key" ;;
      RAW_SEARCH_API_KEY=) line="RAW_SEARCH_API_KEY=$raw_search_key" ;;
      YTSEARCH_INDEX_QUEUE_TOKEN=) line="YTSEARCH_INDEX_QUEUE_TOKEN=$queue_token" ;;
    esac
    printf '%s\n' "$line"
  done < "$ENV_EXAMPLE" > "$ENV_FILE"
  umask "$previous_umask"
  echo "Created $ENV_FILE with local-only development credentials."
else
  echo "Kept existing $ENV_FILE unchanged."
fi

if [ "$SKIP_INSTALL" != "1" ]; then
  cd "$ROOT"
  bun install --frozen-lockfile
  cargo fetch
fi

cat <<'EOF'

Setup complete.

Run the complete local stack:
  docker compose up -d --build

Then open http://localhost:3000.

Or develop the API and frontend in separate terminals:
  docker compose up -d raw-search init-backend
  bun run api
  bun run dev

Run the fast contribution checks with:
  bun run check:precommit
EOF
