#!/usr/bin/env bash
# Checks what agents depend on, against a deployed site or a local Worker. This
# mirrors is-agentic's public checks so a regression shows before its next scan.
#   usage: scripts/check-agent-surface.sh [BASE_URL]
set -uo pipefail
BASE="${1:-https://search.pronsh.dev}"
BASE="${BASE%/}"
failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; failures=$((failures + 1)); }
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# status URL [curl args...]: prints the status and leaves headers and body in $tmp.
fetch() {
  local url="$1"
  shift
  curl -sS --max-time 45 -o "$tmp/body" -D "$tmp/headers" -w '%{http_code}' "$@" "$url" 2>/dev/null || printf '000'
}
header() { grep -i "^$1:" "$tmp/headers" | tail -1 | cut -d: -f2- | tr -d '\r' | sed 's/^ *//'; }

for path in / /about /contact /docs /privacy /llms.txt /robots.txt /sitemap.xml /openapi.json /.well-known/api-catalog; do
  status="$(fetch "$BASE$path")"
  [[ "$status" == 200 ]] && pass "$path answers 200" || fail "$path answered $status"
done

status="$(fetch "$BASE/this-path-should-not-exist-$RANDOM")"
[[ "$status" == 404 ]] && pass "unknown paths answer 404" || fail "unknown path answered $status"
grep -q 'llms.txt' "$tmp/body" && pass "a 404 points agents at llms.txt" || fail "the 404 body has no pointers"

for path in / /about /docs; do
  status="$(fetch "$BASE$path" -H 'Accept: text/markdown')"
  type="$(header content-type)"
  [[ "$status" == 200 && "$type" == text/markdown* ]] && pass "$path negotiates Markdown" \
    || fail "$path with Accept: text/markdown answered $status $type"
  [[ "$(header vary)" == *Accept* ]] && pass "$path Markdown varies on Accept" || fail "$path Markdown lacks Vary: Accept"
  fetch "$BASE$path" -H 'Accept: text/html' >/dev/null
  [[ "$(header vary)" == *Accept* ]] && pass "$path HTML varies on Accept" || fail "$path HTML lacks Vary: Accept"
done

fetch "$BASE/" -H 'Accept: text/html' >/dev/null
python3 - "$tmp/body" "$BASE" <<'PY'
import html, json, re, sys
page = open(sys.argv[1], encoding="utf-8").read()
text = re.sub(r"<script.*?</script>|<style.*?</style>", "", page, flags=re.S)
text = " ".join(html.unescape(re.sub(r"<[^>]+>", " ", text)).split())
checks = {
    "home has an H1 without JavaScript": "<h1" in page,
    f"home has 500+ characters without JavaScript ({len(text)})": len(text) >= 500,
    f"home is at least 5% text ({100 * len(text) / len(page):.1f}%)": len(text) / len(page) >= 0.05,
    "home declares its language": '<html lang="en"' in page,
    "home names its own canonical URL": f'rel="canonical" href="{sys.argv[2]}/"' in page,
    "home links its API docs": 'href="/docs"' in page,
}
blocks = re.findall(r'<script type="application/ld\+json">(.*?)</script>', page, flags=re.S)
checks["home JSON-LD parses"] = bool(blocks) and all(json.loads(block) for block in blocks)
for name, ok in checks.items():
    print(("ok   " if ok else "FAIL ") + name)
sys.exit(sum(not ok for ok in checks.values()))
PY
failures=$((failures + $?))

status="$(fetch "$BASE/api/search?q=rust&per_page=1")"
[[ "$status" == 200 ]] && jq -e '.videos | type == "array"' "$tmp/body" >/dev/null && pass "the search API answers JSON" \
  || fail "search answered $status"
status="$(fetch "$BASE/api/v1/search?q=rust&per_page=1")"
[[ "$status" == 200 ]] && [[ -n "$(header ratelimit-policy)" ]] && pass "/api/v1 answers and states its rate limit" \
  || fail "/api/v1/search answered $status with policy '$(header ratelimit-policy)'"
for case in "/api/this-route-does-not-exist 404" "/api/search?q=rust&sort=bad 400" "/api/search?q= 400"; do
  path="${case% *}"
  expected="${case##* }"
  status="$(fetch "$BASE$path")"
  [[ "$status" == "$expected" ]] && jq -e '.error | type == "string"' "$tmp/body" >/dev/null && pass "$path is a JSON $expected" \
    || fail "$path answered $status, not a JSON $expected"
done

jq -e '
  [.paths[][] | select(.operationId)] as $operations
  | ($operations | length) > 0
    and ($operations | map(.operationId) | unique | length) == ($operations | length)
    and ($operations | all(.description and .responses))
' <(curl -sS --max-time 45 "$BASE/openapi.json") >/dev/null \
  && pass "every OpenAPI operation has a unique ID, a description and responses" || fail "the OpenAPI document is incomplete"

status="$(fetch "$BASE/t3dotgg" -H 'Accept: text/html')"
[[ "$status" == 301 && "$(header location)" == */theo ]] && pass "a renamed creator's old link redirects" \
  || fail "/t3dotgg answered $status $(header location)"
# An indexed video: a shared moment of a video the index does not have is a real 404.
status="$(fetch "$BASE/s/Ash4Q06ZcHU?t=30&end=60" -H 'Accept: text/html')"
[[ "$status" == 200 ]] && grep -q 'start=30' "$tmp/body" && pass "a shared moment opens at its time" \
  || fail "a shared moment answered $status"
status="$(fetch "$BASE/s/zzzzzzzzzzz?t=30&end=60" -H 'Accept: text/html')"
[[ "$status" == 404 ]] && pass "a shared moment of an unknown video answers 404" \
  || fail "a shared moment of an unknown video answered $status"

echo "failures=$failures"
exit "$((failures > 0 ? 1 : 0))"
