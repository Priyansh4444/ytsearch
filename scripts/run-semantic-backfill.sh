#!/usr/bin/env bash
# Resumable semantic backfill from a Raw chunk archive: for each batch of
# creators, build snapshots, then import them into the local Qdrant collection.
#
#   usage: scripts/run-semantic-backfill.sh BACKFILL_DIR ARCHIVE.jsonl.gz
#
# BACKFILL_DIR holds assets/ (copy of data/semantic), bin/ (youtube-semantic and
# build-semantic-snapshots.py), qdrant.yaml, qdrant-up.sh and batches.json
# ([{"name":"b00","creators":["@handle"]}]); per-batch snapshots, import logs and
# `imported-*`/`done` markers are written beneath it, so a rerun resumes. Batches
# keep the snapshot builder, which holds every selected creator in memory, small.
# SEMANTIC_SNAPSHOT_ARGS passes extra builder flags, such as
# "--mutations DB --rename @old=@new", so snapshots match the published corpus.
#
# The experimental Fly machine runs this beside serving (see its entrypoint). The
# model, binaries and Qdrant then come from the image through the environment:
# SEMANTIC_MODEL_DIR, ORT_DYLIB_PATH, SEMANTIC_BIN, SEMANTIC_SNAPSHOT_BUILDER and
# SEMANTIC_QDRANT_URL. Set SEMANTIC_PRUNE_SNAPSHOTS=1 where disk is small: a batch's
# snapshots are deleted once imported, and a rerun skips it through its done marker.
set -uo pipefail
[[ $# -eq 2 ]] || { echo "usage: $0 BACKFILL_DIR ARCHIVE.jsonl.gz" >&2; exit 2; }
B="$(cd "$1" && pwd)"
ARCHIVE="$2"
export SEMANTIC_MODEL_DIR="${SEMANTIC_MODEL_DIR:-$B/assets/model}"
export ORT_DYLIB_PATH="${ORT_DYLIB_PATH:-$B/assets/runtime/onnxruntime-linux-x64-1.31.0/lib/libonnxruntime.so.1.31.0}"
SEMANTIC_BIN="${SEMANTIC_BIN:-$B/bin/youtube-semantic}"
SEMANTIC_SNAPSHOT_BUILDER="${SEMANTIC_SNAPSHOT_BUILDER:-$B/bin/build-semantic-snapshots.py}"
export SEMANTIC_QDRANT_URL="${SEMANTIC_QDRANT_URL:-http://127.0.0.1:16401}"
export SEMANTIC_COLLECTION="${SEMANTIC_COLLECTION:-ytsearch_semantic_pilot_experimental_128}"
export SEMANTIC_DIMENSIONS=128
export SEMANTIC_INDEX_THREADS="${SEMANTIC_INDEX_THREADS:-8}"
log() { echo "$(date -Is) $*"; }

if [[ -x "$B/qdrant-up.sh" ]]; then "$B/qdrant-up.sh" || exit 1; fi
python3 - "$B/batches.json" <<'PY' > "$B/batches.list"
import json, sys
for batch in json.load(open(sys.argv[1])):
    print(batch["name"], " ".join("--creator " + creator for creator in batch["creators"]))
PY
while read -r name creators; do
  dir="$B/$name"
  mkdir -p "$dir"
  [[ -f "$dir/done" ]] && { log "$name already done"; continue; }
  if [[ ! -d "$dir/snapshots" ]]; then
    rm -rf "$dir/tmp"
    log "$name: building snapshots"
    # shellcheck disable=SC2086 # one flag per word
    python3 "$SEMANTIC_SNAPSHOT_BUILDER" "$ARCHIVE" "$dir/tmp" $creators ${SEMANTIC_SNAPSHOT_ARGS:-} > "$dir/build.log" 2>&1 \
      || { log "$name: snapshot build failed (see $dir/build.log)"; exit 1; }
    mv "$dir/tmp" "$dir/snapshots"
  fi
  for snapshot in "$dir"/snapshots/snapshot-*.jsonl; do
    marker="$dir/imported-$(basename "$snapshot")"
    [[ -f "$marker" ]] && continue
    log "$name: importing $(basename "$snapshot")"
    nice -n 10 "$SEMANTIC_BIN" index "$snapshot" >> "$dir/import.log" 2>&1 \
      || { log "$name: import failed for $(basename "$snapshot") (see $dir/import.log)"; exit 1; }
    touch "$marker"
  done
  touch "$dir/done"
  [[ "${SEMANTIC_PRUNE_SNAPSHOTS:-}" == 1 ]] && rm -rf "$dir/snapshots"
  log "$name: done"
done < "$B/batches.list"
log "all batches done"
