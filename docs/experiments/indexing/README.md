# Fetch, import, and scaling audit

Measured and validated 2026-08-01, before the Raw production cutover. This
historical record covers the indexing pipeline then shared by the Typesense
production path and Raw corpus exporter; Raw is now the live backend.

## Contract

The indexer must attempt every requested creator, avoid repeating expensive
work after a partial failure, bound total concurrency, and declare a video
complete only after its chunks are accepted by the backend. Search correctness
remains covered separately by the Raw and Typesense contract suites.

Rollback is one commit per change. No fetch-rate or batch-size increase is
retained without a production-shaped measurement.

## Retained changes

| Area | Previous behavior | Retained behavior |
| --- | --- | --- |
| Metadata and captions | Successful videos launched `yt-dlp` twice | Metadata JSON is handed to caption selection in memory, removing the second successful-path process |
| Caption requests | A failed URL ended that attempt immediately | Up to six bounded attempts, a shared 1.5 second request gate, and rate-limit backoff; all are configurable |
| Process timeout | A timed-out future could leave a child running | `yt-dlp`, `ffprobe`, `ffmpeg`, and `voxtype` children are killed when their owner is dropped |
| Bulk import | A client-wide 60 second deadline could abort a valid synchronous import | Import keeps connect timeout and TCP keepalive; the existing per-video deadline owns the total bound |
| Creator list | The first failed creator stopped the run | Every creator is attempted and failures are aggregated at the end |
| Creator scale | Only one creator ran at once | `--channel-jobs` overlaps at most four creators and enforces `channel_jobs * video_jobs <= 16` |
| Failed videos | A flaky video was retried on every pass | Metadata, transcript, import, and timeout failures cool down for six hours by default; a committed import clears the marker |

The request controls are:

```text
YT_DLP_CAPTION_FETCH_ATTEMPTS=6
YT_DLP_CAPTION_MIN_INTERVAL_MS=1500
YT_DLP_RATE_LIMIT_RETRY_BASE_SECONDS=5
```

Set `--transcript-failure-retry-seconds 0` to disable failed-video cooldown for
an explicit repair pass. The marker directory is
`data/transcript-cache/.failures` by default.

For a machine with measured headroom, start with two creators and four videos
per creator:

```bash
cargo run --release -p youtube_cli -- index-youtubers scripts/indexed-channels.txt \
  --mode update --channel-jobs 2 --jobs 4
```

The scheduler caps this example at eight simultaneous video workers. It
automatically reduces creator concurrency if `--jobs` would exceed the global
budget of sixteen.

## Raw builder measurement

The four-channel workload contains 420,720 chunks from 2,743 videos, 59,596
terms, and produces a 72,441,536-byte fuzzy-v2 index. The retained builder
completed this corpus in 4,090 ms during the deployment build.

Four fresh baseline builds measured 4,414, 4,437, 4,223, and 4,256 ms: a
4,335 ms median. Two candidate changes were rejected:

| Candidate | Four-run median | Delta | Decision |
| --- | ---: | ---: | --- |
| Reuse JSON line buffer only | 4,505.5 ms | +3.9% | Reject |
| Reuse line buffer and zstd compressor | 4,135.5 ms | -4.6% | Reject: changed index bytes and added state for a small offline-only gain |

The experiment left the builder source unchanged. At roughly 103,000 chunks
per second, fetch latency and caption rate limits dominate an end-to-end
channel install; optimizing the offline serializer is not currently the best
use of complexity.

## Why these bounds exist

The caption gate is deliberately shared by cloned workers. Increasing `--jobs`
still overlaps metadata, parsing, chunking, and imports, but it does not turn
caption downloads into an unbounded burst. This improves throughput without
trading it for a higher transient-error rate.

Typesense imports remain batched at 500 documents. Most individual videos do
not exceed one batch, so raising that number would not improve their request
count. Cross-video buffering is deferred until `import_ms` traces show backend
round trips are material relative to metadata and transcript time.

## Validation

- `youtube_cli`: 11 release tests, including creator budgeting and failed-video
  deferral.
- `youtube_ytdlp`: 8 release tests for handoff, retries, parsing, and error
  classification.
- `youtube_typesense`: 14 release tests, including partial bulk-import errors.
- Workspace Clippy runs with warnings denied.
- The complete repository gate and external Raw contract are run before
  handoff.

## Next measurements

The next optimization should use stage timings already emitted as
`metadata_ms`, `transcript_ms`, `chunking_ms`, `import_ms`, and `total_ms`.
Collect a production-shaped sample before changing defaults. If transcript
time dominates, evaluate adaptive host-aware pacing. If imports dominate,
prototype a bounded cross-video batch with per-video commit accounting. Keep
permanent unavailable-caption results distinct from transient network and
rate-limit failures before making the cooldown durable across machines.
