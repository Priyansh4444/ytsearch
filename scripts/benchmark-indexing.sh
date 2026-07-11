#!/usr/bin/env sh
set -eu

# Benchmark indexing phases for one creator. Dry-run by default: it fetches
# metadata/transcripts and chunks locally, but does not write to a search backend.

HANDLE="${1:-@t3dotgg}"
if [ "$#" -gt 0 ]; then
  shift
fi

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

BACKENDS="${BACKENDS:-typesense-fly}"
MAX_VIDEOS="${MAX_VIDEOS:-5}"
IMPORT="${IMPORT:-0}"

import_flag=""
if [ "$IMPORT" = "1" ]; then
  import_flag="--import"
fi

has_backend() {
  backend="$1"
  case " $BACKENDS " in
    *" $backend "*) return 0 ;;
    *) return 1 ;;
  esac
}

run_bench() {
  label="$1"
  backend="$2"
  shift 2

  echo "backend=$label"
  cargo run --release -p youtube_cli -- \
    benchmark-index-youtuber "$HANDLE" \
    --backend "$backend" \
    --max-videos "$MAX_VIDEOS" \
    $import_flag \
    "$@"
}

if has_backend meilisearch; then
  if [ "${ALLOW_LEGACY_MEILI:-0}" != "1" ]; then
    echo "Meilisearch is retired. Set ALLOW_LEGACY_MEILI=1 only for historical benchmarking." >&2
    exit 1
  fi
  run_bench meilisearch meilisearch "$@"
fi

if has_backend typesense-fly; then
  if [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ] || [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ]; then
    echo "Skipping typesense-fly: missing TYPESENSE_SELFHOST_API_KEY or TYPESENSE_SELFHOST_API_URL" >&2
  else
    TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
    TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
    TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
    run_bench typesense-fly typesense "$@"
  fi
fi
