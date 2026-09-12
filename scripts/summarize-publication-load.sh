#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -lt 4 || "$#" -gt 5 ]]; then
  printf 'usage: %s BUILD_JSONL PUBLICATION_START_MS PUBLICATION_END_MS CONCURRENCY [strict|record-only]\n' "$0" >&2
  exit 2
fi

input="$1"
publication_started_ms="$2"
publication_ended_ms="$3"
required_concurrency="$4"
contract_policy="${5:-strict}"

case "$contract_policy" in strict|record-only) ;; *) exit 2 ;; esac

if [[ ! -s "$input" ]]; then
  printf 'publication load input is empty: %s\n' "$input" >&2
  exit 1
fi
for value in "$publication_started_ms" "$publication_ended_ms" "$required_concurrency"; do
  [[ "$value" =~ ^[0-9]+$ ]] || {
    printf 'publication load bounds and concurrency must be integers\n' >&2
    exit 2
  }
done
if (( publication_ended_ms < publication_started_ms || required_concurrency < 1 )); then
  printf 'publication load bounds or concurrency are invalid\n' >&2
  exit 2
fi

summary="$(jq -sce --argjson publication_started "$publication_started_ms" \
  --argjson publication_ended "$publication_ended_ms" \
  --argjson required_concurrency "$required_concurrency" \
  --arg contract_policy "$contract_policy" '
  def request_ok: if has("request_ok") then .request_ok else .ok end;
  def interval_coverage:
    sort_by(.started_ms, .ended_ms) |
    reduce .[] as $interval
      ({end:null,total:0};
        if .end == null then
          .end = $interval.ended_ms |
          .total = ($interval.ended_ms - $interval.started_ms)
        elif $interval.started_ms >= .end then
          .end = $interval.ended_ms |
          .total += ($interval.ended_ms - $interval.started_ms)
        elif $interval.ended_ms > .end then
          .total += ($interval.ended_ms - .end) |
          .end = $interval.ended_ms
        else . end) |
    .total;
  [.[] | select(
    (.worker | type) == "number" and
    (.started_ms | type) == "number" and
    (.ended_ms | type) == "number" and
    .started_ms < .ended_ms
  )] as $rows |
  ([$rows[] |
    select(.started_ms < $publication_ended and .ended_ms > $publication_started) |
    .started_ms = ([.started_ms, $publication_started] | max) |
    .ended_ms = ([.ended_ms, $publication_ended] | min)
  ]) as $publication_rows |
  ($publication_rows | map(
    {time:.started_ms,delta:1},
    {time:.ended_ms,delta:-1}
  ) | sort_by(.time, .delta)) as $events |
  (reduce $events[] as $event
    ({active:0,max_concurrency:0,at:null,required_coverage_ms:0};
      if .at != null and .active >= $required_concurrency then
        .required_coverage_ms += ($event.time - .at)
      else . end |
      .at = $event.time |
      (.active += $event.delta) |
      .max_concurrency = ([.max_concurrency,.active] | max)
    )) as $sweep |
  ([$publication_rows | group_by(.worker)[] |
    interval_coverage
  ] | min // 0) as $minimum_worker_covered_ms |
  {
    samples:($rows | length),
    errors:([$rows[] | select(
      if $contract_policy == "record-only" then (request_ok != true) else (.ok != true) end
    )] | length),
    contract_mismatches:([$rows[] | select(
      request_ok == true and ((.contract_valid // .ok) != true)
    )] | length),
    workers:([$rows[].worker] | unique | length),
    workers_observed_across_publication:([
      $rows | group_by(.worker)[] |
      select(any(.[]; .started_ms < $publication_ended and .ended_ms > $publication_started))
    ] | length),
    requests_overlapping_publication:([
      $rows[] |
      select(.started_ms < $publication_ended and .ended_ms > $publication_started)
    ] | length),
    peak_in_flight_requests:$sweep.max_concurrency,
    minimum_worker_covered_ms:$minimum_worker_covered_ms,
    required_concurrency_covered_ms:$sweep.required_coverage_ms
  }
' "$input")"

if ! jq -e --argjson required_concurrency "$required_concurrency" '
  .samples >= $required_concurrency and
  .errors == 0 and
  .workers == $required_concurrency and
  .workers_observed_across_publication == $required_concurrency and
  .requests_overlapping_publication >= $required_concurrency and
  .peak_in_flight_requests >= $required_concurrency
' <<<"$summary" >/dev/null; then
  printf '%s\n' "$summary"
  printf 'publication load did not prove %s active workers and peak in-flight requests\n' \
    "$required_concurrency" >&2
  exit 1
fi

printf '%s\n' "$summary"
