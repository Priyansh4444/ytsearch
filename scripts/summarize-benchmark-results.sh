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
    case_label = "default"
    total = ""
    http = ""
    curl_exit = ""
    valid_response = ""
    first_result = ""
    total_videos = ""
    total_moments = ""
    bytes = ""
    for (i = 1; i <= NF; i++) {
      split($i, pair, "=")
      if (pair[1] == "backend") {
        backend = pair[2]
      } else if (pair[1] == "case") {
        case_label = pair[2]
      } else if (pair[1] == "total") {
        total = pair[2]
      } else if (pair[1] == "http") {
        http = pair[2]
      } else if (pair[1] == "curl_exit") {
        curl_exit = pair[2]
      } else if (pair[1] == "valid_response") {
        valid_response = pair[2]
      } else if (pair[1] == "first_result_ms") {
        first_result = pair[2]
      } else if (pair[1] == "total_videos") {
        total_videos = pair[2]
      } else if (pair[1] == "total_moments") {
        total_moments = pair[2]
      } else if (pair[1] == "bytes") {
        bytes = pair[2]
      }
    }

    if (backend == "" || total == "") {
      next
    }
    backend = case_label "/" backend

    if (http != "200" ||
        (curl_exit != "" && curl_exit != "0") ||
        (valid_response != "" && valid_response != "1")) {
      errors[backend]++
      seen[backend] = 1
      next
    }

    count[backend]++
    n = count[backend]
    value = seconds(total)
    values[backend, n] = value
    sum[backend] += value
    bytes_sum[backend] += bytes + 0
    if (first_result != "") {
      first_result_sum[backend] += first_result + 0
      first_result_count[backend]++
    }
    if (total_videos != "") {
      total_videos_sum[backend] += total_videos + 0
      total_videos_count[backend]++
      if (!(backend in total_videos_min) || total_videos + 0 < total_videos_min[backend]) total_videos_min[backend] = total_videos + 0
      if (!(backend in total_videos_max) || total_videos + 0 > total_videos_max[backend]) total_videos_max[backend] = total_videos + 0
    }
    if (total_moments != "") {
      total_moments_sum[backend] += total_moments + 0
      total_moments_count[backend]++
      if (!(backend in total_moments_min) || total_moments + 0 < total_moments_min[backend]) total_moments_min[backend] = total_moments + 0
      if (!(backend in total_moments_max) || total_moments + 0 > total_moments_max[backend]) total_moments_max[backend] = total_moments + 0
    }
    seen[backend] = 1
  }

  END {
    printf "case/backend\tcount\terrors\tmin_ms\tavg_ms\tp50_ms\tp95_ms\tmax_ms\tavg_bytes\tavg_first_result_ms\tavg_total_videos\tavg_total_moments\ttotal_videos_range\ttotal_moments_range\tcorpus_stable\n"
    for (backend in seen) {
      c = count[backend] + 0
      e = errors[backend] + 0
      if (c == 0) {
        printf "%s\t0\t%d\t\t\t\t\t\t\t\t\t\t\t\t\n", backend, e
        continue
      }
      delete sorted
      sort_values(backend, c, values, sorted)
      min = sorted[1] * 1000
      avg = (sum[backend] / c) * 1000
      p50 = sorted[pct_index(c, 0.50)] * 1000
      p95 = sorted[pct_index(c, 0.95)] * 1000
      max = sorted[c] * 1000
      first_result_avg = first_result_count[backend] ? first_result_sum[backend] / first_result_count[backend] : 0
      videos_avg = total_videos_count[backend] ? total_videos_sum[backend] / total_videos_count[backend] : 0
      moments_avg = total_moments_count[backend] ? total_moments_sum[backend] / total_moments_count[backend] : 0
      stable = (total_videos_min[backend] == total_videos_max[backend] && total_moments_min[backend] == total_moments_max[backend]) ? "yes" : "no"
      printf "%s\t%d\t%d\t%.0f\t%.0f\t%.0f\t%.0f\t%.0f\t%.0f\t%.0f\t%.0f\t%.0f\t%d-%d\t%d-%d\t%s\n", backend, c, e, min, avg, p50, p95, max, bytes_sum[backend] / c, first_result_avg, videos_avg, moments_avg, total_videos_min[backend], total_videos_max[backend], total_moments_min[backend], total_moments_max[backend], stable
    }
  }
' "$INPUT"
