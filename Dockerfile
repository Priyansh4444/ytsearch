FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --locked --release -p youtube_api -p youtube_cli

FROM denoland/deno:bin-2.8.1 AS deno

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg python3-pip \
    && python3 -m pip install --no-cache-dir --break-system-packages --upgrade 'yt-dlp[default]' \
    && rm -rf /var/lib/apt/lists/*
COPY --from=deno /deno /usr/local/bin/deno
COPY --from=builder /app/target/release/youtube-api /usr/local/bin/youtube-api
COPY --from=builder /app/target/release/ytsearch /usr/local/bin/ytsearch
COPY scripts/cloud-index-queue-entrypoint.sh /usr/local/bin/cloud-index-queue
# `ytsearch publish` rebuilds each generation from the reader's base corpus:
# the same empty files Dockerfile.raw-search-selfhost creates. Not under /data,
# which compose mounts over.
RUN base=/usr/local/share/ytsearch/empty-base \
    && install -d "$base" \
    && : | gzip -n > "$base/youtube_transcript_chunks.jsonl.gz" \
    && : | gzip -n > "$base/youtube_youtubers.jsonl.gz"
ENV RAW_SEARCH_BASE_CHUNKS_GZIP=/usr/local/share/ytsearch/empty-base/youtube_transcript_chunks.jsonl.gz
ENV RAW_SEARCH_BASE_CHANNELS_GZIP=/usr/local/share/ytsearch/empty-base/youtube_youtubers.jsonl.gz
ENV YTSEARCH_API_ADDR=0.0.0.0:8787
EXPOSE 8787
CMD ["youtube-api"]
