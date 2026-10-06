#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' HUP INT TERM

mkdir -p "$tmp/bin" "$tmp/state" "$tmp/cache"
printf 'fixture\thttps://example.invalid/playlist\n' >"$tmp/manifest.tsv"

printf '%s\n' '#!/usr/bin/env sh' "printf '%s\\n' -fixture-video" >"$tmp/bin/yt-dlp"
printf '%s\n' \
  '#!/usr/bin/env sh' \
  'test -n "${RAW_SEARCH_API_URL:-}"' \
  'test -n "${RAW_SEARCH_ADMIN_API_KEY:-}"' \
  'case " $* " in *" --no-wipe "*" -- -fixture-video "*) exit 0;; *) exit 1;; esac' >"$tmp/bin/cargo"
chmod +x "$tmp/bin/yt-dlp" "$tmp/bin/cargo"

PATH="$tmp/bin:$PATH" \
RAW_SEARCH_API_URL=http://raw.invalid \
RAW_SEARCH_ADMIN_API_KEY=fixture-admin-key \
CONFERENCE_STATE_DIR="$tmp/state" \
TRANSCRIPT_CACHE_DIR="$tmp/cache" \
"$SCRIPT_DIR/index-conference-playlists.sh" "$tmp/manifest.tsv" >/dev/null

grep -Fxq -- -fixture-video "$tmp/state/fixture.completed"
echo "conference_playlist_checks=ok"
