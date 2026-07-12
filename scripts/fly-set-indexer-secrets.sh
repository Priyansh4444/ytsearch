#!/usr/bin/env sh
set -eu

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"
. "$(dirname "$0")/queue-api-url.sh"

INDEX_QUEUE_API_BASE_URL="$(normalize_queue_api_url "${INDEX_QUEUE_API_BASE_URL:-}")"

flyctl apps create "$APP_NAME" --org "$FLY_ORG" >/dev/null 2>&1 || true

require_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required env var: $name" >&2
    exit 1
  fi
}

require_env TYPESENSE_SELFHOST_API_URL
require_env TYPESENSE_SELFHOST_API_KEY
require_env YTSEARCH_INDEX_QUEUE_TOKEN

set_optional_ytdlp_secrets() {
  if [ -n "${YT_DLP_COOKIES_B64:-}" ]; then
    flyctl secrets set --stage --app "$APP_NAME" YT_DLP_COOKIES_FILE_B64="$YT_DLP_COOKIES_B64"
  elif [ -n "${YT_DLP_COOKIES_FILE:-}" ] && [ -f "$YT_DLP_COOKIES_FILE" ]; then
    flyctl secrets set --stage --app "$APP_NAME" \
      YT_DLP_COOKIES_FILE_B64="$(base64 < "$YT_DLP_COOKIES_FILE" | tr -d '\n')"
  fi

  if [ -n "${YT_DLP_EXTRACTOR_ARGS:-}" ]; then
    flyctl secrets set --stage --app "$APP_NAME" YT_DLP_EXTRACTOR_ARGS="$YT_DLP_EXTRACTOR_ARGS"
  fi

  if [ -n "${YT_DLP_USER_AGENT:-}" ]; then
    flyctl secrets set --stage --app "$APP_NAME" YT_DLP_USER_AGENT="$YT_DLP_USER_AGENT"
  fi
}

configure_youtube_api_fallback() {
  # yt-dlp remains the primary discovery path. Opt in explicitly when the
  # Data API should be available as a fallback, or use 0 to remove an old key.
  case "${YOUTUBE_API_FALLBACK:-auto}" in
    1|true|yes)
      require_env YOUTUBE_API_KEY
      flyctl secrets set --stage --app "$APP_NAME" \
        YOUTUBE_API_KEY_FILE_B64="$(printf '%s' "$YOUTUBE_API_KEY" | base64 | tr -d '\n')"
      ;;
    0|false|no)
      flyctl secrets unset --stage --app "$APP_NAME" \
        YOUTUBE_API_KEY YOUTUBE_API_KEY_FILE_B64 >/dev/null 2>&1 || true
      ;;
    auto)
      if [ -n "${YOUTUBE_API_KEY:-}" ]; then
        flyctl secrets set --stage --app "$APP_NAME" \
          YOUTUBE_API_KEY_FILE_B64="$(printf '%s' "$YOUTUBE_API_KEY" | base64 | tr -d '\n')"
      fi
      ;;
    *)
      echo "YOUTUBE_API_FALLBACK must be auto, 0, or 1." >&2
      exit 1
      ;;
  esac
}

# Values are loaded from local env files and passed directly to Fly; they are not printed.
flyctl secrets set --stage \
  --app "$APP_NAME" \
  INDEX_QUEUE_API_BASE_URL="$INDEX_QUEUE_API_BASE_URL" \
  YTSEARCH_INDEX_QUEUE_TOKEN="$YTSEARCH_INDEX_QUEUE_TOKEN" \
  TYPESENSE_SELFHOST_API_URL="$TYPESENSE_SELFHOST_API_URL" \
  TYPESENSE_SELFHOST_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
  TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY"

set_optional_ytdlp_secrets
configure_youtube_api_fallback
