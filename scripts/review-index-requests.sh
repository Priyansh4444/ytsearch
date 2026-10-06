#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

REVIEW_URL="${INDEX_REVIEW_API_BASE_URL:-${PUBLIC_URL:-https://search.pronsh.dev}}"
REVIEW_URL="${REVIEW_URL%/}"

if [ -z "${YTSEARCH_INDEX_QUEUE_TOKEN:-}" ]; then
  echo "Missing required environment variable: YTSEARCH_INDEX_QUEUE_TOKEN" >&2
  exit 1
fi

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_command curl
require_command jq

api_request() {
  method="$1"
  path="$2"
  curl -fsS \
    -X "$method" \
    -H "Authorization: Bearer $YTSEARCH_INDEX_QUEUE_TOKEN" \
    "$REVIEW_URL$path"
}

validate_request_id() {
  case "$1" in
    ""|*[!A-Za-z0-9._@-]*)
      echo "Invalid request id: $1" >&2
      exit 1
      ;;
  esac
}

list_pending_json() {
  api_request GET "/api/index-requests?limit=1000"
}

print_pending() {
  list_pending_json | jq -r '
    [.requests[] | select(.status == "pending")] as $pending
    | if ($pending | length) == 0 then
        "No index requests are awaiting approval."
      else
        (["#", "handle", "backend", "requested", "note"] | @tsv),
        ($pending | to_entries[] |
          [
            (.key + 1),
            .value.handle,
            (.value.target_backend // .value.source_backend),
            (.value.requested_at | todateiso8601),
            (.value.note // "" | gsub("[\\t\\r\\n]+"; " "))
          ] | @tsv)
      end'
}

review_request() {
  action="$1"
  request_id="$2"
  validate_request_id "$request_id"
  api_request POST "/api/index-requests/$request_id/$action" \
    | jq -r '"\(.request.handle) is now \(.request.status) (\(.request.id))"'
}

interactive_review() {
  pending_file="$(mktemp)"
  trap 'rm -f "$pending_file"' EXIT
  trap 'exit 130' HUP INT TERM
  list_pending_json >"$pending_file"
  count="$(jq '[.requests[] | select(.status == "pending")] | length' "$pending_file")"
  if [ "$count" -eq 0 ]; then
    echo "No index requests are awaiting approval."
    return 0
  fi

  echo "Pending index requests: $count"
  index=0
  while [ "$index" -lt "$count" ]; do
    request="$(jq -c --argjson index "$index" '[.requests[] | select(.status == "pending")][$index]' "$pending_file")"
    request_id="$(printf '%s' "$request" | jq -r '.id')"
    handle="$(printf '%s' "$request" | jq -r '.handle')"
    backend="$(printf '%s' "$request" | jq -r '.target_backend // .source_backend')"
    requested="$(printf '%s' "$request" | jq -r '.requested_at | todateiso8601')"
    note="$(printf '%s' "$request" | jq -r '.note // ""')"

    echo
    echo "[$((index + 1))/$count] $handle"
    echo "  id:        $request_id"
    echo "  backend:   $backend"
    echo "  requested: $requested"
    if [ -n "$note" ]; then
      echo "  note:      $note"
    fi
    printf "Approve, reject, skip, or quit? [a/r/s/q] "
    if ! IFS= read -r choice; then
      echo
      return 0
    fi
    case "$choice" in
      a|A|approve)
        review_request approve "$request_id"
        ;;
      r|R|reject)
        review_request reject "$request_id"
        ;;
      s|S|skip|"")
        echo "Skipped $handle"
        ;;
      q|Q|quit)
        return 0
        ;;
      *)
        echo "Choose a, r, s, or q."
        continue
        ;;
    esac
    index=$((index + 1))
  done
}

command="${1:-review}"
case "$command" in
  review)
    if [ "$#" -ne 0 ] && [ "$#" -ne 1 ]; then
      echo "Usage: $0 [review|list|approve REQUEST_ID|reject REQUEST_ID]" >&2
      exit 1
    fi
    interactive_review
    ;;
  list)
    [ "$#" -eq 1 ] || { echo "Usage: $0 list" >&2; exit 1; }
    print_pending
    ;;
  approve|reject)
    [ "$#" -eq 2 ] || { echo "Usage: $0 $command REQUEST_ID" >&2; exit 1; }
    review_request "$command" "$2"
    ;;
  *)
    echo "Usage: $0 [review|list|approve REQUEST_ID|reject REQUEST_ID]" >&2
    exit 1
    ;;
esac
