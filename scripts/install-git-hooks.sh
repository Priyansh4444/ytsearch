#!/usr/bin/env sh
set -eu

if [ ! -d .git ]; then
  echo "This script must be run from the repository root." >&2
  exit 1
fi

cp scripts/hooks/pre-commit .git/hooks/pre-commit
chmod +x .git/hooks/pre-commit

echo "Installed pre-commit frontend checks."
