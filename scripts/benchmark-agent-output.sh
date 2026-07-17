#!/usr/bin/env sh
set -eu

# Compare raw and RTK-filtered output on representative repository commands.
# Approximate tokens use the conventional bytes/4 heuristic; exact tokenization
# varies by model and is intentionally not presented as provider billing data.

if ! command -v rtk >/dev/null 2>&1; then
  echo "rtk is required: https://github.com/rtk-ai/rtk" >&2
  exit 1
fi

output_dir="$(mktemp -d)"
trap 'rm -rf "$output_dir"' EXIT HUP INT TERM
results="$output_dir/results.tsv"

printf 'case\tmode\texit\tlines\tbytes\tapprox_tokens\tduration_ms\n' >"$results"

run_command() {
  case_name="$1"
  mode="$2"
  command="$3"
  output="$output_dir/$case_name.$mode.log"
  started="$(date +%s%N)"
  set +e
  sh -c "$command" >"$output" 2>&1
  status="$?"
  set -e
  finished="$(date +%s%N)"
  bytes="$(wc -c <"$output" | tr -d ' ')"
  lines="$(wc -l <"$output" | tr -d ' ')"
  tokens="$(((bytes + 3) / 4))"
  duration_ms="$(((finished - started) / 1000000))"
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$case_name" "$mode" "$status" "$lines" "$bytes" "$tokens" "$duration_ms" >>"$results"
}

run_pair() {
  case_name="$1"
  raw_command="$2"
  compact_command="$3"
  run_command "$case_name" raw "$raw_command"
  run_command "$case_name" rtk "$compact_command"
}

run_pair git-log \
  'git log -n 30' \
  'rtk git log -n 30'
run_pair search \
  'rg -n "TODO|FIXME|AXIOM_|TYPESENSE_" --glob "!target/**" .' \
  'rtk rg -n "TODO|FIXME|AXIOM_|TYPESENSE_" --glob "!target/**" .'
run_pair rust-tests \
  'cargo test --workspace' \
  'rtk cargo test --workspace'

cat "$results"
awk -F '\t' '
  NR == 1 { next }
  $2 == "raw" { raw[$1] = $6; raw_total += $6 }
  $2 == "rtk" { compact[$1] = $6; compact_total += $6 }
  END {
    print ""
    print "case\tapprox_token_savings"
    for (name in raw) {
      printf "%s\t%.1f%%\n", name, 100 * (raw[name] - compact[name]) / raw[name]
    }
    printf "overall\t%.1f%%\n", 100 * (raw_total - compact_total) / raw_total
  }
' "$results"
