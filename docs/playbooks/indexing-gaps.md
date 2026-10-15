# Indexing gaps

## Fast path

1. Capture public creator counts without changing the writer:

   ```bash
   curl -fsS https://ytsearch-api-raw-production.fly.dev/api/youtubers > /tmp/ytsearch-creators.json
   jq '[.channels[] | {handle,indexed_video_count,public_video_count,gap:(.public_video_count-.indexed_video_count)}] | sort_by(-.gap)' /tmp/ytsearch-creators.json
   ```

2. On the already running queue worker, capture classified outcomes:

   ```bash
   ytsearch failure-report /var/lib/ytsearch/transcript-cache --transcript-failure-retry-seconds 21600 > /tmp/ytsearch-failures.json
   jq '{total_records,unattributed_records,totals,by_creator}' /tmp/ytsearch-failures.json
   ```

3. Join `.channels[].id` to `.by_creator` for a diagnostic summary, or report a
   combined production capture by video ID:

   ```bash
   python3 scripts/report-video-gaps.py /tmp/ytsearch-production-capture.json > /tmp/ytsearch-video-gaps.json
   ```

   The report input combines `catalog`, `discoveries`, `failures` and optional
   selected worker `state` in one JSON object; it makes no network calls. It
   includes every captured creator and marks missing or stale evidence. For
   completeness, use `scripts/check-raw-creator-reconciliation.sh` after one
   full-list run and its publication. Read its errors: the checker intentionally
   rejects stale manifests, unpublished mutations and unclassified IDs.

4. If `unknown` dominates, inspect a bounded sample of the matching creator's
   `.failures/*.failed` JSON reasons. Read only ledger files; never print process
   environment, API credentials or cookie contents. Classify the actual errors
   before retrying a large batch. Keep `update`, `JOBS=1` and `CHANNEL_JOBS=1`.

## Traps

- A public/discovered count includes discoverable members-only and paid videos.
  Their metadata can be visible while captions require access. These are access
  restrictions (`private`), not a broken transcript parser or ordinary retries.
- The cloud indexer has Python 3 but no `curl`. Use Python `urllib.request`
  for remote HTTP diagnostics; never print credential-bearing environment or
  headers. Its short idle timeout can stop the Machine during separate SSH
  steps, so capture selected evidence in one bounded command.
- `no_captions` means no usable English caption track. With a preflight-ready
  `--transcribe-missing` runtime, these records reopen for captions first, then
  audio inference. A validated final transcript cache also reopens the video
  without requiring an ASR runtime. Private, members-only, paid, deleted,
  bot-challenged and rate-limited outcomes retain their access/cooldown policy.
- Cloud indexer images contain checksum-verified `base.en` and pinned CPU
  whisper.cpp. `TRANSCRIBE_MISSING=1` enables the same arguments for queue,
  daily catalog and idle gap repair. `TRANSCRIPTION_BUDGET_SECONDS=480` is an
  aggregate active-work budget per CLI invocation, shared across all catalog
  creators; caption/cache processing continues when it runs out. `transcription`
  operational failures remain retryable, and budget deferrals do not increment
  attempts. A completed inference with no usable speech is `no_speech`, a terminal
  outcome which a newly supplied validated cache can still recover.
- Completed ASR chunks, including silence, checkpoint under
  `TRANSCRIPTION_CACHE_DIR` (default `$TRANSCRIPT_CACHE_DIR/.transcription`).
  A later pass resumes them. The work cache retains at most 1.5 GB of completed
  audio files; a full work cache allows existing videos to resume before new
  downloads. Final transcripts commit atomically to `$TRANSCRIPT_CACHE_DIR/<id>.json`
  only after every chunk completes; successful commits remove intermediate audio.
- Whisper uses 20ms timestamp frames. Clamp a final offset at most one frame
  beyond the chunk boundary; reject larger or unordered ranges. The real canary
  produced `29520..30020` on a 30-second chunk, exposing this boundary case.
- Keep the invocation budget below `VIDEO_TIMEOUT_SECONDS`. Source clones share
  a serialized inference slot, and cancellation consumes elapsed active budget.
  Do not give every video a new budget inside a catalog process.
- Failure ledger totals are not the current gap total: records can refer to IDs
  no longer discovered or already indexed. Never subtract aggregate ledger
  counts from the public gap and call the difference unexplained. Compare IDs
  against a generation-pinned snapshot and current discovery instead.
- `.failed` files include legacy plain-text records. Parsing every file as JSON
  aborts on the first legacy record. The `failure-report` command handles both.
- The canonical list and live approved creators can differ. A strict canonical
  reconciliation reports those extra profiles; do not delete approved creators
  merely to make it pass. Ensure ongoing refresh covers approved creators too.
- A stopped queue worker can be healthy while idle. Waking it does not force a
  daily run whose persisted schedule is not due.

## Facts

- 2026-10-15: public API exposed 69 creators, 41,399 indexed videos and 42,427
  discovered videos (1,028 visible gaps). The checked-in canonical list had 62
  creators. Snapshot counts are diagnostic, not a completed reconciliation.
- The captured failure report contained 970 `no_captions` records. HealthyGamerGG
  had 108 `unknown` records, all members-only; CommonLuke had five and
  LiveOverflow/Vimjoyer one each. Two Vsauce failures required payment.
- One Google for Developers failure was a JSON3 caption event with integer
  `-10` where the parser required `u32`. Signed event times, clipping negative
  starts at zero while preserving the end, prevent one event from discarding
  the whole transcript.
