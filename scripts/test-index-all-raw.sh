#!/usr/bin/env bash
set -euo pipefail

# Runs the local bulk-indexing scripts from a copy of scripts/ (so no real .env
# is loaded) against a fake cargo that records the CLI arguments.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$tmp/bin" "$tmp/repository"
cp -R "$SCRIPT_DIR" "$tmp/repository/scripts"
cd "$tmp/repository"
printf '@one\n@two\n' > "$tmp/list.txt"

cat > "$tmp/bin/cargo" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$@" > "$FAKE_CARGO_ARGS"
printf 'fake cargo output\n'
exit "${FAKE_CARGO_STATUS:-0}"
SH
chmod +x "$tmp/bin/cargo"
export PATH="$tmp/bin:$PATH" FAKE_CARGO_ARGS="$tmp/cargo-args"
unset BACKENDS MODE JOBS CHANNEL_JOBS EXISTING_TAIL STOP_AFTER_EXISTING MAX_VIDEOS \
  COLLECTION RAW_SEARCH_COLLECTION YTSEARCH_TRANSCRIPTS_INDEX ALLOW_DESTRUCTIVE_REWIPE
credentials=(RAW_SEARCH_ADMIN_API_KEY=test-admin RAW_SEARCH_API_URL=http://raw.test)

expect_cargo() {
  printf '%s\n' run --release -p youtube_cli -- index-youtubers "$@" > "$tmp/expected"
  diff "$tmp/expected" "$tmp/cargo-args"
  rm "$tmp/cargo-args"
}

expect_refusal() {
  local message="$1"
  shift
  if env "$@" >/dev/null 2>&1; then
    printf '%s\n' "$message" >&2
    exit 1
  fi
  [[ ! -e "$tmp/cargo-args" ]]
}

env "${credentials[@]}" scripts/index-all-raw.sh "$tmp/list.txt" --verbose-extra >/dev/null
expect_cargo "$tmp/list.txt" --backend raw --mode update \
  --collection youtube_transcript_chunks --jobs 1 --channel-jobs 1 \
  --stop-after-existing 0 --verbose-extra

env "${credentials[@]}" MODE=rewipe ALLOW_DESTRUCTIVE_REWIPE=1 BACKENDS=other JOBS=2 \
  CHANNEL_JOBS=3 EXISTING_TAIL=5 RAW_SEARCH_COLLECTION=other-collection \
  scripts/index-all-raw.sh "$tmp/list.txt" >/dev/null
expect_cargo "$tmp/list.txt" --backend raw --mode update \
  --collection other-collection --jobs 2 --channel-jobs 3 --stop-after-existing 5

expect_refusal 'missing Raw credentials must be refused' \
  RAW_SEARCH_API_URL=http://raw.test scripts/index-all-raw.sh "$tmp/list.txt"
expect_refusal 'a missing list file must be refused' \
  "${credentials[@]}" scripts/index-all-raw.sh "$tmp/missing.txt"

log="$tmp/logs/quiet.log"
env "${credentials[@]}" LOG_FILE="$log" EXISTING_TAIL=5 STOP_AFTER_EXISTING=7 MAX_VIDEOS=9 \
  scripts/run-index-all-quiet.sh "$tmp/list.txt" > "$tmp/quiet.out"
grep -Fx "Indexing completed successfully. Log: $log" "$tmp/quiet.out" >/dev/null
grep -Fx 'fake cargo output' "$log" >/dev/null
sort "$tmp/cargo-args" > "$tmp/sorted-args"
printf '%s\n' run --release -p youtube_cli -- index-youtubers "$tmp/list.txt" \
  --backend raw --mode update --collection youtube_transcript_chunks --jobs 1 \
  --channel-jobs 1 --stop-after-existing 7 --max-videos 9 | sort > "$tmp/expected"
diff "$tmp/expected" "$tmp/sorted-args"
rm "$tmp/cargo-args"

if env "${credentials[@]}" LOG_FILE="$log" FAKE_CARGO_STATUS=3 \
  scripts/run-index-all-quiet.sh "$tmp/list.txt" > "$tmp/quiet.out" 2> "$tmp/quiet.err"; then
  printf 'a failed index run must fail the quiet wrapper\n' >&2
  exit 1
else
  [[ "$?" == 3 ]]
fi
grep -F 'Indexing failed with exit code 3' "$tmp/quiet.err" >/dev/null
grep -Fx 'fake cargo output' "$tmp/quiet.err" >/dev/null

printf 'index_all_raw=ok\n'
