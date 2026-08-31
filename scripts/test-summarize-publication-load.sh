#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

for worker in 1 2 3 4; do
  jq -cn --argjson worker "$worker" \
    '{worker:$worker,started_ms:100,ended_ms:200,ok:true}' \
    >> "$tmp/concurrent.jsonl"
done
"$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/concurrent.jsonl" 110 190 4 \
  > "$tmp/concurrent-summary.json"
jq -e '
  .samples == 4 and .errors == 0 and .workers == 4 and
  .workers_observed_across_publication == 4 and .peak_in_flight_requests == 4
' "$tmp/concurrent-summary.json" >/dev/null

for worker in 1 2 3 4; do
  start=$(( worker * 100 ))
  end=$(( start + 50 ))
  jq -cn --argjson worker "$worker" --argjson start "$start" --argjson end "$end" \
    '{worker:$worker,started_ms:$start,ended_ms:$end,ok:true}' \
    >> "$tmp/serial.jsonl"
done
if "$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/serial.jsonl" 100 450 4 \
  >/dev/null 2>&1; then
  printf 'serial requests must not satisfy a concurrency-four gate\n' >&2
  exit 1
fi

cat > "$tmp/boundary-only.jsonl" <<'JSONL'
{"worker":1,"started_ms":100,"ended_ms":150,"ok":true}
{"worker":2,"started_ms":150,"ended_ms":200,"ok":true}
JSONL
if "$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/boundary-only.jsonl" 100 200 2 \
  >/dev/null 2>&1; then
  printf 'touching half-open intervals must not count as concurrent requests\n' >&2
  exit 1
fi

for worker in 1 2 3 4; do
  jq -cn --argjson worker "$worker" \
    '{worker:$worker,started_ms:0,ended_ms:50,ok:true}' \
    >> "$tmp/pre-publication-peak.jsonl"
  start=$(( 100 + ((worker - 1) * 25) ))
  end=$(( start + 25 ))
  jq -cn --argjson worker "$worker" --argjson start "$start" --argjson end "$end" \
    '{worker:$worker,started_ms:$start,ended_ms:$end,ok:true}' \
    >> "$tmp/pre-publication-peak.jsonl"
  jq -cn --argjson worker "$worker" \
    '{worker:$worker,started_ms:250,ended_ms:300,ok:true}' \
    >> "$tmp/pre-publication-peak.jsonl"
done
if "$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/pre-publication-peak.jsonl" 100 200 4 \
  >/dev/null 2>&1; then
  printf 'concurrency before publication must not satisfy the publication-window gate\n' >&2
  exit 1
fi

jq -cn '{worker:1,started_ms:100,ended_ms:200,ok:false}' > "$tmp/error.jsonl"
if "$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/error.jsonl" 110 190 1 \
  >/dev/null 2>&1; then
  printf 'failed search samples must fail the publication load gate\n' >&2
  exit 1
fi

printf 'publication_load_summary=ok\n'
