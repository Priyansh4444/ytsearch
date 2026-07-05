#!/usr/bin/env sh
set -eu

if [ ! -d .git ]; then
  echo "This script must be run from the repository root." >&2
  exit 1
fi

cp scripts/hooks/pre-commit .git/hooks/pre-commit
cp scripts/hooks/post-commit .git/hooks/post-commit
chmod +x .git/hooks/pre-commit .git/hooks/post-commit

echo "Installed git hooks: pre-commit frontend checks, post-commit Cloudflare Pages deploy."
echo "Set YTSEARCH_SKIP_DEPLOY_ON_COMMIT=1 to skip deploys for a commit."
