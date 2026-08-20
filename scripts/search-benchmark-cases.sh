#!/usr/bin/env sh

# Shared validation for the canonical page-one relevance workload. Specialized
# pagination/sort/video contracts remain in their focused checks.

validate_search_benchmark_cases() {
  cases_file="$1"
  awk -F '\t' '
    BEGIN { ok = 1 }
    /^#/ || /^[[:space:]]*$/ { next }
    {
      sub(/\r$/, "", $NF)
      if (NF < 2 || NF > 3) {
        printf "%s:%d: expected label, query, and optional creator\n", FILENAME, NR > "/dev/stderr"
        ok = 0
        next
      }
      if ($1 !~ /^[a-z0-9_]+$/ || seen[$1]++) {
        printf "%s:%d: label must be unique snake_case: %s\n", FILENAME, NR, $1 > "/dev/stderr"
        ok = 0
      }
      if ($2 == "") {
        printf "%s:%d: query must not be empty\n", FILENAME, NR > "/dev/stderr"
        ok = 0
      }
    }
    END { exit ok ? 0 : 1 }
  ' "$cases_file"
}
