#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/ignored" "$fixture/nested"
cp "$root/scripts/check-markdown-links.ts" "$fixture/scripts/"
git -C "$fixture" init -q
git -C "$fixture/nested" init -q
printf 'ignored/\n' > "$fixture/.gitignore"
printf '[missing](missing.md)\n' > "$fixture/ignored/broken.md"
printf '[missing](missing.md)\n' > "$fixture/nested/broken.md"
printf '# Target\n' > "$fixture/target.md"
printf '[target](target.md)\n' > "$fixture/tracked.md"
git -C "$fixture" add tracked.md target.md
printf '[target](target.md)\n' > "$fixture/untracked.md"
bun "$fixture/scripts/check-markdown-links.ts" > "$fixture/result"
grep -q 'markdown_links=ok files=3' "$fixture/result"
printf '[missing](missing.md)\n' > "$fixture/untracked.md"
if bun "$fixture/scripts/check-markdown-links.ts" > "$fixture/result" 2>&1; then
  echo 'FAIL: untracked broken link accepted' >&2
  exit 1
fi
grep -q 'untracked.md: missing missing.md' "$fixture/result"
rm "$fixture/tracked.md" "$fixture/untracked.md"
bun "$fixture/scripts/check-markdown-links.ts" > "$fixture/result"
grep -q 'markdown_links=ok files=1' "$fixture/result"

# Code spans must name existing files and package scripts.
printf '{"scripts":{"known":"true"}}\n' > "$fixture/package.json"
printf 'Run `scripts/check-markdown-links.ts`, then `bun run known`.\n' > "$fixture/code.md"
bun "$fixture/scripts/check-markdown-links.ts" > "$fixture/result"
printf 'Run `scripts/gone.sh`, then:\n\n```bash\nbun run unknown\n```\n' > "$fixture/code.md"
if bun "$fixture/scripts/check-markdown-links.ts" > "$fixture/result" 2>&1; then
  echo 'FAIL: stale code reference accepted' >&2
  exit 1
fi
grep -q 'code.md: missing scripts/gone.sh' "$fixture/result"
grep -q 'code.md: no package script unknown' "$fixture/result"
echo 'markdown link scope tests passed'
