#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
TAG="${1:-}"

if [[ ! "$TAG" =~ ^[a-z0-9][a-z0-9-]*$ ]]; then
  echo "usage: scripts/autoresearch-init.sh <lowercase-run-tag>" >&2
  exit 2
fi

BRANCH="autoresearch/$TAG"
WORKTREE="${AUTORESEARCH_WORKTREE:-/tmp/ytsearch-autoresearch-$TAG}"
if git -C "$ROOT" show-ref --verify --quiet "refs/heads/$BRANCH"; then
  echo "error=branch_exists branch=$BRANCH" >&2
  exit 2
fi
if [ -e "$WORKTREE" ]; then
  echo "error=worktree_path_exists path=$WORKTREE" >&2
  exit 2
fi

git -C "$ROOT" worktree add -b "$BRANCH" "$WORKTREE" HEAD
printf 'branch=%s\nworktree=%s\nnext=cd %q\n' "$BRANCH" "$WORKTREE" "$WORKTREE"
