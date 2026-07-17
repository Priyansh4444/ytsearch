#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
CONFIG="$ROOT/autoresearch/vector.env"
RESULTS="${AUTORESEARCH_RESULTS:-$ROOT/autoresearch/vector-results.tsv}"
RUN_ROOT="${OUT_DIR:-$ROOT/autoresearch/runs/vector-$(date -u +%Y%m%dT%H%M%SZ)}"
TRIALS="${TRIALS:-3}"
MIN_RECALL="${MIN_RECALL:-0.95}"
DESCRIPTION="${1:-unnamed vector experiment}"

if [ ! -f "$CONFIG" ]; then
  echo "missing_config=$CONFIG" >&2
  exit 2
fi
if [ -n "$(git -C "$ROOT" status --short -- ':!autoresearch/runs' ':!autoresearch/vector-results.tsv')" ]; then
  echo "error=tracked_or_unrelated_worktree_changes_present" >&2
  echo "Commit the one-variable experiment before evaluating it." >&2
  exit 2
fi

# vector.env is trusted repository configuration, not arbitrary user input.
set -a
# shellcheck disable=SC1090
. "$CONFIG"
set +a
case "${HNSW_EF:-}" in
  ''|*[!0-9]*|0) echo "error=HNSW_EF_must_be_a_positive_integer" >&2; exit 2 ;;
esac
case "$TRIALS" in
  ''|*[!0-9]*|0) echo "error=TRIALS_must_be_a_positive_integer" >&2; exit 2 ;;
esac

mkdir -p "$RUN_ROOT"
commit="$(git -C "$ROOT" rev-parse --short HEAD)"
printf 'commit=%s\nhnsw_ef=%s\ntrials=%s\nmin_recall=%s\n' \
  "$commit" "$HNSW_EF" "$TRIALS" "$MIN_RECALL" > "$RUN_ROOT/manifest.txt"

for trial in $(seq 1 "$TRIALS"); do
  echo "trial=$trial/$TRIALS hnsw_ef=$HNSW_EF"
  "$SCRIPT_DIR/benchmark-qdrant-vector.sh" > "$RUN_ROOT/trial-$trial.log" 2>&1
done

awk '
  /^case=unfiltered / {
    for (i = 1; i <= NF; i++) {
      split($i, field, "=")
      if (field[1] == "ann_p95_us") print field[2] > latency
      if (field[1] ~ /^recall_at_/) print field[2] > recall
    }
  }
' latency="$RUN_ROOT/latency.txt" recall="$RUN_ROOT/recall.txt" "$RUN_ROOT"/trial-*.log

if [ "$(wc -l < "$RUN_ROOT/latency.txt")" -ne "$TRIALS" ] || \
   [ "$(wc -l < "$RUN_ROOT/recall.txt")" -ne "$TRIALS" ]; then
  echo "error=incomplete_benchmark_output" >&2
  exit 1
fi

median_latency="$(sort -n "$RUN_ROOT/latency.txt" | awk '{value[NR]=$1} END {print value[int((NR+1)/2)]}')"
min_observed_recall="$(sort -n "$RUN_ROOT/recall.txt" | head -n 1)"
best_latency="$(awk -F '\t' -v min_recall="$MIN_RECALL" '
  NR > 1 && $6 == "keep" && ($5 + 0) >= min_recall { if (best == "" || $4 < best) best=$4 }
  END { print best }
' "$RESULTS" 2>/dev/null || true)"

status=discard
if awk -v recall="$min_observed_recall" -v minimum="$MIN_RECALL" 'BEGIN {exit !(recall >= minimum)}'; then
  if [ -z "$best_latency" ] || [ "$median_latency" -lt "$best_latency" ]; then
    status=keep
  fi
fi

if [ ! -f "$RESULTS" ]; then
  printf 'timestamp\tcommit\thnsw_ef\tann_p95_us_median\tmin_recall_at_10\tstatus\tdescription\n' > "$RESULTS"
fi
description="${DESCRIPTION//$'\t'/ }"
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$HNSW_EF" \
  "$median_latency" "$min_observed_recall" "$status" "$description" >> "$RESULTS"

printf 'status=%s\nann_p95_us_median=%s\nmin_recall_at_10=%s\nout_dir=%s\n' \
  "$status" "$median_latency" "$min_observed_recall" "$RUN_ROOT"
