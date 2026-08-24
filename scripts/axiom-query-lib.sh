#!/usr/bin/env sh

# Shared Axiom query boundary. Source this file after resolving SCRIPT_DIR.
# Query credentials never fall back to the ingest-only AXIOM_API_KEY.

. "$SCRIPT_DIR/load-env.sh"

axiom_require_query_config() {
  AXIOM_QUERY_TOKEN="${AXIOM_API_QUERY:-${AXIOM_PROD_API_KEY:-}}"
  AXIOM_QUERY_DATASET="${AXIOM_DATASET:-}"
  AXIOM_QUERY_BASE="${AXIOM_EDGE_URL:-${AXIOM_URL:-https://api.axiom.co}}"
  AXIOM_QUERY_BASE="${AXIOM_QUERY_BASE%/}"
  if [ -z "$AXIOM_QUERY_TOKEN" ] || [ -z "$AXIOM_QUERY_DATASET" ]; then
    echo "AXIOM_API_QUERY and AXIOM_DATASET are required (query token only; not AXIOM_API_KEY)." >&2
    exit 1
  fi
}

axiom_validate_duration() {
  axiom_duration_name="$1"
  axiom_duration_value="$2"
  case "$axiom_duration_value" in
    *[!0-9mhdw]* | "")
      echo "$axiom_duration_name must be a duration such as 24h, 7d, or 365d." >&2
      exit 1
      ;;
  esac
}

axiom_query_to_file() {
  axiom_apl="$1"
  axiom_start="$2"
  axiom_response="$3"
  axiom_body="$(jq -n \
    --arg apl "$axiom_apl" \
    --arg start "$axiom_start" \
    '{apl:$apl, startTime:$start, endTime:"now"}')"
  axiom_status="$(curl -sS --max-time 60 -o "$axiom_response" -w '%{http_code}' \
    -H "Authorization: Bearer $AXIOM_QUERY_TOKEN" \
    -H 'Content-Type: application/json' \
    -d "$axiom_body" \
    "$AXIOM_QUERY_BASE/v1/datasets/_apl?format=tabular")" || axiom_status=000
  case "$axiom_status" in
    2??) return ;;
  esac

  axiom_message="$(jq -r '.message // .error // "unknown Axiom query failure"' "$axiom_response" 2>/dev/null || true)"
  echo "Axiom query failed (HTTP $axiom_status): $axiom_message" >&2
  if [ "$axiom_status" = 403 ]; then
    echo "AXIOM_API_QUERY needs dataset query:read on '$AXIOM_QUERY_DATASET' (Advanced token with Query)." >&2
  fi
  exit 1
}

axiom_print_first_table() {
  jq -r '
    (.tables // [])[0] as $table
    | select($table != null)
    | ($table.fields // [] | map(.name) | @tsv),
      (
        range(0; (($table.columns[0] // []) | length)) as $row
        | [ $table.columns[][$row] ]
        | map(if . == null then "" else tostring end)
        | @tsv
      )
  ' "$1"
}

axiom_query_log_lines() {
  jq -r '
    def non_empty_string:
      type == "string" and length > 0;

    def log_line($time; $query):
      "[\($time)]: \($query | gsub("[\r\n]+"; " "))";

    def from_tabular:
      (.tables // [])[]
      | . as $table
      | ($table.fields // [] | map(.name)) as $names
      | ($names | index("_time")) as $time_index
      | ($names | index("query")) as $query_index
      | select($time_index != null and $query_index != null)
      | (($table.columns[$time_index] // []) | length) as $rows
      | range(0; $rows) as $row
      | ($table.columns[$time_index][$row]) as $time
      | ($table.columns[$query_index][$row]) as $query
      | select(($time | non_empty_string) and ($query | non_empty_string))
      | log_line($time; $query);

    def from_legacy:
      (.matches // [])[]
      | (._time // empty) as $time
      | (.data.query // .data["query"] // empty) as $query
      | select(($time | non_empty_string) and ($query | non_empty_string))
      | log_line($time; $query);

    (from_tabular, from_legacy)
  ' "$1"
}

axiom_last_logged_time() {
  awk '
    match($0, /^\[[0-9]{4}-[0-9]{2}-[0-9]{2}T[^]]+\]:/) {
      line = substr($0, RSTART, RLENGTH)
      print substr(line, 2, length(line) - 3)
      next
    }
    /^[0-9]{4}-[0-9]{2}-[0-9]{2}T/ {
      split($0, fields, "\t")
      print fields[1]
    }
  ' "$1" | tail -n 1
}
