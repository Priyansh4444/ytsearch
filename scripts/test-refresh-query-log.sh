#!/usr/bin/env sh
set -eu

# Offline checks for scripts/refresh-query-log tabular parsing, log format, and
# time window selection. Does not call Axiom.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT_DIR="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
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

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT HUP INT TERM

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
last_seen_bare="$(axiom_last_logged_time "$ROOT_DIR/query-log.txt" 2>/dev/null || true)"
# Only assert emptiness when the real log is still legacy-only.
if ! grep -qE '^(\[[0-9]{4}-[0-9]{2}-[0-9]{2}T|[0-9]{4}-[0-9]{2}-[0-9]{2}T)' "$ROOT_DIR/query-log.txt" 2>/dev/null; then
  assert_eq "legacy query-log has no last_seen" "" "$last_seen_bare"
fi

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

if [ "$failed" -ne 0 ]; then
  echo "$failed assertion(s) failed" >&2
  exit 1
fi
echo "All refresh-query-log offline checks passed."
