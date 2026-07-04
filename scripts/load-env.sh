#!/usr/bin/env sh
# Source this file from project scripts to load local env files without printing secrets.
# Supports simple KEY=VALUE .env files.

load_env_file() {
  file="$1"
  if [ -f "$file" ]; then
    set -a
    case "$file" in
      */*) . "$file" ;;
      *) . "./$file" ;;
    esac
    set +a
  fi
}

load_env_file ".env"
load_env_file ".env.local"
