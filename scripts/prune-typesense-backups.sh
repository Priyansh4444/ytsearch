#!/usr/bin/env bash
set -euo pipefail

# Keep 7 newest backups, one backup in each of the newest 4 ISO weeks, and one
# in each of the newest 12 months. Defaults to a report; APPLY=1 deletes.
backup_root="${BACKUP_ROOT:-backups}"
apply="${APPLY:-0}"

mapfile -t stamps < <(find "$backup_root" -mindepth 1 -maxdepth 1 -type d \
  -printf '%f\n' 2>/dev/null | grep -E '^[0-9]{8}T[0-9]{6}Z$' | sort -r)
declare -A keep weeks months

for ((i=0; i<${#stamps[@]} && i<7; i++)); do keep["${stamps[$i]}"]=1; done
for stamp in "${stamps[@]}"; do
  day="${stamp:0:4}-${stamp:4:2}-${stamp:6:2}"
  week="$(date -u -d "$day" +%G-%V)"
  month="${stamp:0:6}"
  if (( ${#weeks[@]} < 4 )) && [[ -z "${weeks[$week]:-}" ]]; then
    weeks[$week]=1; keep[$stamp]=1
  fi
  if (( ${#months[@]} < 12 )) && [[ -z "${months[$month]:-}" ]]; then
    months[$month]=1; keep[$stamp]=1
  fi
done

for stamp in "${stamps[@]}"; do
  [[ -n "${keep[$stamp]:-}" ]] && continue
  if [[ "$apply" == 1 ]]; then
    rm -r -- "$backup_root/$stamp"
    echo "deleted=$backup_root/$stamp"
  else
    echo "would_delete=$backup_root/$stamp"
  fi
done
[[ "$apply" == 1 ]] || echo "dry_run=1 (set APPLY=1 after reviewing)"
