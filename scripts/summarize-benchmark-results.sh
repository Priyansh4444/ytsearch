#!/usr/bin/env sh
set -eu

INPUT="${1:-/dev/stdin}"

awk '
  function pct_index(count, pct) {
    idx = int((count * pct) + 0.999999)
    if (idx < 1) {
      idx = 1
    }
    if (idx > count) {
      idx = count
    }
    return idx
  }

  function seconds(value) {
    sub(/s$/, "", value)
    return value + 0
  }

  function sort_values(backend, count, values, sorted, i, j, tmp) {
    for (i = 1; i <= count; i++) {
      sorted[i] = values[backend, i]
    }
    for (i = 2; i <= count; i++) {
      tmp = sorted[i]
      j = i - 1
      while (j >= 1 && sorted[j] > tmp) {
        sorted[j + 1] = sorted[j]
        j--
      }
      sorted[j + 1] = tmp
    }
  }

  {
    backend = ""
    total = ""
    http = ""
    for (i = 1; i <= NF; i++) {
      split($i, pair, "=")
      if (pair[1] == "backend") {
        backend = pair[2]
      } else if (pair[1] == "total") {
        total = pair[2]
      } else if (pair[1] == "http") {
        http = pair[2]
      }
    }

    if (backend == "" || total == "") {
      next
    }

    if (http != "200") {
      errors[backend]++
      seen[backend] = 1
      next
    }

    count[backend]++
    n = count[backend]
    value = seconds(total)
    values[backend, n] = value
    sum[backend] += value
    seen[backend] = 1
  }

  END {
    printf "backend\tcount\terrors\tmin_ms\tavg_ms\tp50_ms\tp95_ms\tmax_ms\n"
    for (backend in seen) {
      c = count[backend] + 0
      e = errors[backend] + 0
      if (c == 0) {
        printf "%s\t0\t%d\t\t\t\t\t\n", backend, e
        continue
      }
      delete sorted
      sort_values(backend, c, values, sorted)
      min = sorted[1] * 1000
      avg = (sum[backend] / c) * 1000
      p50 = sorted[pct_index(c, 0.50)] * 1000
      p95 = sorted[pct_index(c, 0.95)] * 1000
      max = sorted[c] * 1000
      printf "%s\t%d\t%d\t%.0f\t%.0f\t%.0f\t%.0f\t%.0f\n", backend, c, e, min, avg, p50, p95, max
    }
  }
' "$INPUT"
