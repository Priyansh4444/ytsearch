#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <typesense-backup-dir> <new-raw-index-dir>" >&2
  exit 2
fi

BACKUP_DIR="$1"
OUTPUT_DIR="$2"
SOURCE_CHUNKS_ARCHIVE="$BACKUP_DIR/youtube_transcript_chunks.jsonl.gz"
CHUNKS_ARCHIVE="${RAW_CHUNKS_ARCHIVE:-$SOURCE_CHUNKS_ARCHIVE}"
CHANNELS_ARCHIVE="$BACKUP_DIR/youtube_youtubers.jsonl.gz"

case "$OUTPUT_DIR" in
  "" | / | .)
    echo "refusing unsafe output directory: $OUTPUT_DIR" >&2
    exit 2
    ;;
esac

for path in "$SOURCE_CHUNKS_ARCHIVE" "$CHUNKS_ARCHIVE" "$CHANNELS_ARCHIVE" "$BACKUP_DIR/COUNTS.tsv" "$BACKUP_DIR/SHA256SUMS"; do
  if [[ ! -f "$path" ]]; then
    echo "missing backup artifact: $path" >&2
    exit 2
  fi
done
if [[ -e "$OUTPUT_DIR/manifest.json" ]]; then
  echo "refusing to overwrite completed Raw generation: $OUTPUT_DIR" >&2
  exit 2
fi

for command in cargo gzip jq sha256sum; do
  command -v "$command" >/dev/null || {
    echo "missing required command: $command" >&2
    exit 2
  }
done

(
  cd "$BACKUP_DIR"
  sha256sum -c SHA256SUMS
)
if [[ "$CHUNKS_ARCHIVE" != "$SOURCE_CHUNKS_ARCHIVE" ]]; then
  CHUNKS_CHECKSUM="${RAW_CHUNKS_SHA256_FILE:-$CHUNKS_ARCHIVE.sha256}"
  if [[ ! -f "$CHUNKS_CHECKSUM" || "$(dirname "$CHUNKS_CHECKSUM")" != "$(dirname "$CHUNKS_ARCHIVE")" ]]; then
    echo "custom chunks archive requires a checksum file beside it: $CHUNKS_CHECKSUM" >&2
    exit 2
  fi
  expected_chunks_sha256="$(awk 'NR == 1 { print $1 }' "$CHUNKS_CHECKSUM")"
  actual_chunks_sha256="$(sha256sum "$CHUNKS_ARCHIVE" | awk '{ print $1 }')"
  if [[ ! "$expected_chunks_sha256" =~ ^[0-9a-f]{64}$ || "$actual_chunks_sha256" != "$expected_chunks_sha256" ]]; then
    echo "custom chunks archive failed its SHA-256 checksum: $CHUNKS_ARCHIVE" >&2
    exit 1
  fi
fi
gzip -t "$CHUNKS_ARCHIVE"

expected_documents="$(awk -F '\t' '$1 == "youtube_transcript_chunks" { print $2 }' "$BACKUP_DIR/COUNTS.tsv")"
expected_channels="$(awk -F '\t' '$1 == "youtube_youtubers" { print $2 }' "$BACKUP_DIR/COUNTS.tsv")"
if [[ ! "$expected_documents" =~ ^[0-9]+$ || ! "$expected_channels" =~ ^[0-9]+$ ]]; then
  echo "backup counts are missing or malformed" >&2
  exit 2
fi

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/ytsearch-raw-build.XXXXXX")"
CHUNKS_PIPE="$WORK_DIR/youtube_transcript_chunks.jsonl"
CHANNELS_JSONL="$WORK_DIR/youtube_youtubers.jsonl"
producer_pid=""
build_pid=""
cleanup() {
  if [[ -n "$build_pid" ]]; then
    kill "$build_pid" 2>/dev/null || true
    wait "$build_pid" 2>/dev/null || true
  fi
  if [[ -n "$producer_pid" ]]; then
    kill "$producer_pid" 2>/dev/null || true
    wait "$producer_pid" 2>/dev/null || true
  fi
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

gzip -dc "$CHANNELS_ARCHIVE" >"$CHANNELS_JSONL"
mkfifo "$CHUNKS_PIPE"
cargo build --release -p youtube_raw_search
gzip -dc "$CHUNKS_ARCHIVE" >"$CHUNKS_PIPE" &
producer_pid="$!"

mkdir -p "$OUTPUT_DIR"
started_ns="$(date +%s%N)"
target/release/youtube-raw-search build \
  "$CHUNKS_PIPE" "$CHANNELS_JSONL" "$OUTPUT_DIR" \
  >"$OUTPUT_DIR/build-report.json" &
build_pid="$!"
peak_builder_rss_kb=0
peak_decompressor_rss_kb=0
peak_total_rss_kb=0
while kill -0 "$build_pid" 2>/dev/null; do
  builder_rss_kb="$(awk '$1 == "VmRSS:" { print $2 }' "/proc/$build_pid/status" 2>/dev/null || true)"
  decompressor_rss_kb="$(awk '$1 == "VmRSS:" { print $2 }' "/proc/$producer_pid/status" 2>/dev/null || true)"
  [[ "$builder_rss_kb" =~ ^[0-9]+$ ]] || builder_rss_kb=0
  [[ "$decompressor_rss_kb" =~ ^[0-9]+$ ]] || decompressor_rss_kb=0
  ((builder_rss_kb <= peak_builder_rss_kb)) || peak_builder_rss_kb="$builder_rss_kb"
  ((decompressor_rss_kb <= peak_decompressor_rss_kb)) || peak_decompressor_rss_kb="$decompressor_rss_kb"
  total_rss_kb="$((builder_rss_kb + decompressor_rss_kb))"
  if ((total_rss_kb > peak_total_rss_kb)); then
    peak_total_rss_kb="$total_rss_kb"
  fi
  sleep 0.1
done
set +e
wait "$build_pid"
build_status="$?"
build_pid=""
wait "$producer_pid"
producer_status="$?"
producer_pid=""
set -e
if [[ "$build_status" -ne 0 || "$producer_status" -ne 0 ]]; then
  echo "Raw generation build failed: builder=$build_status decompressor=$producer_status" >&2
  exit 1
fi
elapsed_ms="$((($(date +%s%N) - started_ns) / 1000000))"
jq -n \
  --argjson elapsed_ms "$elapsed_ms" \
  --argjson peak_rss_kb "$peak_total_rss_kb" \
  --argjson peak_builder_rss_kb "$peak_builder_rss_kb" \
  --argjson peak_decompressor_rss_kb "$peak_decompressor_rss_kb" \
  '{elapsed_ms:$elapsed_ms,peak_rss_kb:$peak_rss_kb,peak_builder_rss_kb:$peak_builder_rss_kb,peak_decompressor_rss_kb:$peak_decompressor_rss_kb}' \
  >"$OUTPUT_DIR/build-resources.json"
jq . "$OUTPUT_DIR/build-report.json"

actual_documents="$(jq -er '.documents' "$OUTPUT_DIR/build-report.json")"
actual_channels="$(jq -er '.channels' "$OUTPUT_DIR/build-report.json")"
if [[ "$actual_documents" != "$expected_documents" || "$actual_channels" != "$expected_channels" ]]; then
  echo "Raw generation count mismatch: documents=$actual_documents/$expected_documents channels=$actual_channels/$expected_channels" >&2
  exit 1
fi

jq -n \
  --arg backup_dir "$BACKUP_DIR" \
  --arg chunks_archive "$CHUNKS_ARCHIVE" \
  --argjson documents "$actual_documents" \
  --argjson channels "$actual_channels" \
  '{backup_dir:$backup_dir,chunks_archive:$chunks_archive,documents:$documents,channels:$channels,validated_at:(now | todateiso8601)}' \
  >"$OUTPUT_DIR/build-source.json"

echo "$OUTPUT_DIR"
