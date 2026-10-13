#!/usr/bin/env sh
set -eu

# Offline checks for scripts/refresh-query-log tabular parsing, log format, and
# time window selection. Does not call Axiom.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
trap 'exit 130' HUP INT TERM
# The query helper loads cwd-local env files. Never read the developer's env.
cd "$tmpdir"
. "$SCRIPT_DIR/axiom-query-lib.sh"
failed=0

assert_eq() {
  label="$1"
  expected="$2"
  actual="$3"
  if [ "$expected" = "$actual" ]; then
    printf 'ok  %s\n' "$label"
  else
    printf 'FAIL %s\n  expected: %s\n  actual:   %s\n' "$label" "$expected" "$actual" >&2
    failed=$((failed + 1))
  fi
}

# --- tabular column-oriented response (current Axiom format) ---
cat >"$tmpdir/tabular.json" <<'EOF'
{
  "format": "tabular",
  "tables": [
    {
      "name": "0",
      "fields": [
        {"name": "_time", "type": "datetime"},
        {"name": "query", "type": "string"}
      ],
      "columns": [
        ["2026-07-28T10:00:00Z", "2026-07-28T11:00:00.123456789Z", "2026-07-29T09:30:00Z"],
        ["react", "fable on high", "I"]
      ]
    }
  ]
}
EOF

parsed="$(axiom_query_log_lines "$tmpdir/tabular.json")"

assert_eq "tabular row count" "3" "$(printf '%s\n' "$parsed" | wc -l | tr -d ' ')"
assert_eq "tabular first row" "[2026-07-28T10:00:00Z]: react" "$(printf '%s\n' "$parsed" | sed -n '1p')"
assert_eq "tabular second row" "[2026-07-28T11:00:00.123456789Z]: fable on high" "$(printf '%s\n' "$parsed" | sed -n '2p')"
assert_eq "tabular third row" "[2026-07-29T09:30:00Z]: I" "$(printf '%s\n' "$parsed" | sed -n '3p')"

# Old/broken top-level .columns/.rows shape must yield nothing from the new parser
# when tables are absent — and the old parser yields nothing on real tabular.
old_parser_on_tabular="$(jq -r '
  (.columns // [] | map(.name // .)) as $columns
  | ($columns | index("_time")) as $time
  | ($columns | index("query")) as $query
  | (.rows // .data // [])[] | [.[ $time ], .[ $query ]] | @tsv
' "$tmpdir/tabular.json" || true)"
assert_eq "legacy parser misses tabular data" "" "$old_parser_on_tabular"

# --- legacy matches[] format still works ---
cat >"$tmpdir/legacy.json" <<'EOF'
{
  "format": "legacy",
  "matches": [
    {"_time": "2026-07-20T01:02:03Z", "data": {"query": "hello", "event": "search_query"}},
    {"_time": "2026-07-20T01:02:04Z", "data": {"query": "world", "event": "search_query"}}
  ]
}
EOF

legacy_parsed="$(axiom_query_log_lines "$tmpdir/legacy.json")"

assert_eq "legacy row count" "2" "$(printf '%s\n' "$legacy_parsed" | wc -l | tr -d ' ')"
assert_eq "legacy first row" "[2026-07-20T01:02:03Z]: hello" "$(printf '%s\n' "$legacy_parsed" | sed -n '1p')"

# Newlines inside query text are flattened.
cat >"$tmpdir/multiline.json" <<'EOF'
{
  "tables": [
    {
      "fields": [{"name": "_time"}, {"name": "query"}],
      "columns": [
        ["2026-07-28T12:00:00Z"],
        ["line1\nline2"]
      ]
    }
  ]
}
EOF
assert_eq "multiline query flattened" "[2026-07-28T12:00:00Z]: line1 line2" \
  "$(axiom_query_log_lines "$tmpdir/multiline.json")"

# --- last_seen selection uses last append-order timestamped line ---
cat >"$tmpdir/log.txt" <<'EOF'
# header
react
linux
2026-07-10T12:00:00Z	older-tab-format
[2026-07-28T15:30:00.5Z]: mid-fractional
bare-query-after-timestamps
[2026-07-28T15:30:00Z]: later-whole-second-same-clock
[2026-07-29T01:00:00Z]: newest
EOF

last_seen="$(axiom_last_logged_time "$tmpdir/log.txt")"
assert_eq "last_seen uses final timestamped line" "2026-07-29T01:00:00Z" "$last_seen"

# Bracket form is preferred when mixed with tab form earlier in the file.
cat >"$tmpdir/mixed.txt" <<'EOF'
2026-07-30T10:00:00Z	tab-newer-but-wrong-if-sorted
[2026-07-29T09:00:00Z]: bracket-last-appended
EOF
assert_eq "last_seen prefers append order over format" "2026-07-29T09:00:00Z" \
  "$(axiom_last_logged_time "$tmpdir/mixed.txt")"

# Lexicographic sort would wrongly rank …00.5Z before …00Z; ensure we do not.
sorted_wrong="$(printf '%s\n' "2026-07-28T15:30:00.5Z" "2026-07-28T15:30:00Z" | sort | tail -n 1)"
assert_eq "lexicographic sort is unsafe for mixed precision" "2026-07-28T15:30:00Z" "$sorted_wrong"

# bare-only log has no last_seen
printf 'react\nlinux\n' > "$tmpdir/bare.txt"
assert_eq "legacy query-log has no last_seen" "" "$(axiom_last_logged_time "$tmpdir/bare.txt")"

# --- request body includes startTime/endTime ---
body_incremental="$(jq -n \
  --arg apl "['ytsearch'] | where _time > datetime(\"2026-07-28T15:30:00Z\") | project _time, query" \
  --arg start "2026-07-28T15:30:00Z" \
  '{apl:$apl, startTime:$start, endTime:"now"}')"
assert_eq "incremental body startTime" "2026-07-28T15:30:00Z" "$(printf '%s' "$body_incremental" | jq -r .startTime)"
assert_eq "incremental body endTime" "now" "$(printf '%s' "$body_incremental" | jq -r .endTime)"

body_initial="$(jq -n \
  --arg apl "['ytsearch'] | project _time, query" \
  --arg start "now-365d" \
  '{apl:$apl, startTime:$start, endTime:"now"}')"
assert_eq "initial body startTime" "now-365d" "$(printf '%s' "$body_initial" | jq -r .startTime)"

# --- report-style tabular header/rows ---
report="$(axiom_print_first_table "$tmpdir/tabular.json")"
assert_eq "report header" "_time	query" "$(printf '%s\n' "$report" | sed -n '1p')"
assert_eq "report data rows" "3" "$(printf '%s\n' "$report" | sed -n '2,$p' | wc -l | tr -d ' ')"

# Execute the actual command with no inherited credentials and a fake transport.
mkdir "$tmpdir/bin"
cat > "$tmpdir/bin/curl" <<'EOF'
#!/usr/bin/env sh
set -eu
output=""
authorization=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -sS) shift ;;
    --max-time|-w) shift 2 ;;
    -o) output="$2"; shift 2 ;;
    -H)
      case "$2" in Authorization:*) authorization="$2" ;; esac
      shift 2
      ;;
    -d) printf '%s' "$2" > "$FAKE_REQUEST_BODY"; shift 2 ;;
    https://*) url="$1"; shift ;;
    *) echo "unexpected curl argument" >&2; exit 91 ;;
  esac
done
[ "$authorization" = 'Authorization: Bearer fixture-query-token' ]
[ "$url" = 'https://axiom.invalid/v1/datasets/_apl?format=tabular' ]
[ -n "$output" ]
cp "$FAKE_RESPONSE_BODY" "$output"
printf '%s' "$FAKE_HTTP_STATUS"
EOF
chmod +x "$tmpdir/bin/curl"

run_refresh() {
  env -i PATH="$tmpdir/bin:/usr/bin:/bin" \
    AXIOM_API_QUERY=fixture-query-token AXIOM_API_KEY=fixture-ingest-token \
    AXIOM_DATASET=fixture AXIOM_URL=https://axiom.invalid \
    QUERY_LOG_FILE="$tmpdir/command-log.txt" \
    FAKE_REQUEST_BODY="$tmpdir/request.json" \
    FAKE_RESPONSE_BODY="$1" FAKE_HTTP_STATUS="$2" \
    "$SCRIPT_DIR/refresh-query-log"
}

printf '[2026-07-20T00:00:00Z]: existing\n' > "$tmpdir/command-log.txt"
cp "$tmpdir/command-log.txt" "$tmpdir/prior-log.txt"
# jq can emit valid rows before encountering a malformed trailing JSON value.
# Neither those rows nor a success message may survive a failed complete parse.
cat "$tmpdir/tabular.json" > "$tmpdir/malformed.json"
printf '\n{broken json\n' >> "$tmpdir/malformed.json"
if run_refresh "$tmpdir/malformed.json" 200 > "$tmpdir/command-output" 2> "$tmpdir/command-error"; then
  echo 'FAIL executable accepted HTTP 200 malformed JSON' >&2
  failed=$((failed + 1))
fi
if ! cmp -s "$tmpdir/prior-log.txt" "$tmpdir/command-log.txt"; then
  echo 'FAIL malformed JSON changed the existing query log' >&2
  failed=$((failed + 1))
fi

printf '{"message":"fixture denied"}\n' > "$tmpdir/http-error.json"
cp "$tmpdir/prior-log.txt" "$tmpdir/command-log.txt"
if run_refresh "$tmpdir/http-error.json" 403 > "$tmpdir/command-output" 2> "$tmpdir/command-error"; then
  echo 'FAIL executable accepted HTTP 403' >&2
  failed=$((failed + 1))
fi
if ! cmp -s "$tmpdir/prior-log.txt" "$tmpdir/command-log.txt"; then
  echo 'FAIL HTTP error changed the existing query log' >&2
  failed=$((failed + 1))
fi

cp "$tmpdir/prior-log.txt" "$tmpdir/command-log.txt"
run_refresh "$tmpdir/tabular.json" 200 > "$tmpdir/command-output"
cat "$tmpdir/prior-log.txt" > "$tmpdir/expected-log.txt"
printf '%s\n' "$parsed" >> "$tmpdir/expected-log.txt"
if ! cmp -s "$tmpdir/expected-log.txt" "$tmpdir/command-log.txt"; then
  echo 'FAIL executable did not append the valid response exactly' >&2
  failed=$((failed + 1))
fi
assert_eq "executable incremental startTime" "2026-07-20T00:00:00Z" "$(jq -r .startTime "$tmpdir/request.json")"
assert_eq "executable endTime" "now" "$(jq -r .endTime "$tmpdir/request.json")"

if [ "$failed" -ne 0 ]; then
  echo "$failed assertion(s) failed" >&2
  exit 1
fi
echo "All refresh-query-log offline checks passed."
