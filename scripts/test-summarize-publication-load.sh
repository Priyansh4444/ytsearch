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
  .workers_observed_across_publication == 4 and .peak_in_flight_requests == 4 and
  .minimum_worker_covered_ms == 80 and .required_concurrency_covered_ms == 80
' "$tmp/concurrent-summary.json" >/dev/null

for worker in 1 2 3 4; do
  jq -cn --argjson worker "$worker" \
    '{worker:$worker,started_ms:0,ended_ms:1,ok:true},
     {worker:$worker,started_ms:119999,ended_ms:120000,ok:true}' \
    >> "$tmp/discontinuous.jsonl"
done
"$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/discontinuous.jsonl" 0 120000 4 \
  > "$tmp/discontinuous-summary.json"
jq -e '
  .minimum_worker_covered_ms == 2 and
  .required_concurrency_covered_ms == 2
' "$tmp/discontinuous-summary.json" >/dev/null

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

for worker in 1 2 3 4; do
  jq -cn --argjson worker "$worker" \
    '{worker:$worker,started_ms:100,ended_ms:200,request_ok:true,
      contract_valid:false,ok:false}' >> "$tmp/old-contract.jsonl"
done
if "$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/old-contract.jsonl" 100 200 4 strict \
  >/dev/null 2>&1; then
  printf 'strict load must reject a relevance-contract mismatch\n' >&2
  exit 1
fi
"$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/old-contract.jsonl" 100 200 4 record-only \
  > "$tmp/old-contract-summary.json"
jq -e '.errors == 0 and .contract_mismatches == 4' \
  "$tmp/old-contract-summary.json" >/dev/null

jq -cn '{worker:1,started_ms:100,ended_ms:200,ok:false}' > "$tmp/error.jsonl"
if "$SCRIPT_DIR/summarize-publication-load.sh" "$tmp/error.jsonl" 110 190 1 \
  >/dev/null 2>&1; then
  printf 'failed search samples must fail the publication load gate\n' >&2
  exit 1
fi

printf 'publication_load_summary=ok\n'
