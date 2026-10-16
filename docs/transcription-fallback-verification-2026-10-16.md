# Transcription fallback verification — 2026-10-16

The deployed cloud image was tested on the existing inert publication builder,
using a separate loopback Raw server, empty base corpus, temporary credentials,
and isolated state directory. It did not consume the production queue or write
to the production reader. The builder was stopped after the test, and its
temporary cookie copy and secret-file mapping were removed.

## Actual captionless video

- Video: `_X5FbiW0L_8`, Fireship, “Async Validation with Angular Reactive Forms + Firebase”.
- YouTube metadata reported a public, non-live video with no manual or automatic caption tracks.
- Runtime image: `sha256:10c578caa9533b97ad4455b2657bd5f0eb757f3ea09c93e4d6a5f84f04a0ed8b`.
- Runtime source: `f500a716c638db78fad1947501d192aec388d098`.
- Same cloud shape as the queue worker: two shared CPUs, 2 GiB RAM.
- Pinned whisper.cpp 1.9.5, runtime CPU dispatch, checksum-verified base.en, two inference threads.

The first `index-video` invocation exhausted its shared 480-second active-work
budget. The command retained completed chunk checkpoints and returned a deferred
fallback error, preserving the original no-caption reason. A second invocation
resumed and completed in 135.767 seconds. Total canary wall time, including
metadata, downloads, publication and cache verification, was 626.858 seconds.

The completed transcript contained **107 timed segments**. The normal chunker
imported **54 transcript chunks** and published generation 1 in the isolated
reader. A video-filtered search for `uniqueness` returned one matching hit.
Re-indexing with inference disabled reused the portable cache and completed in
3.991 seconds. Intermediate audio/checkpoints were removed after final commit.

The largest child process peak RSS was 340,392 KiB. This is a child-process
measurement, not an aggregate cgroup peak. Sustained shared CPU inference was
slower after initial burst credits, which makes bounded work and resume important.

## Failures exposed by the real test

The original image contained neither Voxtype nor whisper.cpp. The cloud launcher
never enabled fallback, terminal `no_captions` records blocked recovery, and the
audio path bypassed the caption downloader's session settings. Those causes are
fixed together, rather than deleting failure records.

A static baseline CPU build was too slow. Upstream runtime CPU dispatch preserves
baseline compatibility while selecting optimized kernels on the actual host.
Whisper also decoded endpoints beyond a 30-second audio window, including
`29.600..30.600`. The adapter now intersects segments with real audio, discards
padding-only segments, and rejects reversed or unordered timestamps. Both cases
are covered by regression tests.

## Production repair

The verified final transcript was atomically copied into the existing queue
worker's portable cache. Its canonical entrypoint was restored, with fallback
enabled and the complete 69-creator list. The prior completion marker was backed
up and reset for the explicitly approved full catalog repair. Existing three-day
catalog, daily gap-sweep and twelve-hour routine-publication cost caps remain.

The isolated canary proves cloud ASR, resumability, durable import, publication,
search and cache reuse. Production reconciliation and public search confirmation
are separate checks; their final outcomes must be recorded after the repair pass.
