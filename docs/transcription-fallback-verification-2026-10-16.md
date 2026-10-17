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

Production ledger inspection also found five audio subprocess timeouts. Inference
shared a two-minute helper timeout with ffmpeg and ffprobe even though the
invocation had a 480-second budget. Inference now uses the remaining invocation
budget; short helper commands retain their separate bound. A virtual-time
regression crosses the former timeout while inference is running, then completes
the transcript. It failed with the old deadline and passes with the fix.

Another regression proved completed speech-free audio remained in the bounded
work cache. Completed silence now removes its intermediate audio/checkpoints and
retains the classified `no_speech` outcome; interrupted inference still resumes.

## Production repair

The verified final transcript was atomically copied into the existing queue
worker's portable cache. Its canonical entrypoint was restored, with fallback
enabled and the complete 69-creator list. The prior completion marker was backed
up and reset for the explicitly approved full catalog repair. Existing three-day
catalog, daily gap-sweep and twelve-hour routine-publication cost caps remain.

## Production confirmation, 2026-10-17

Public search for `uniqueness`, filtered to `_X5FbiW0L_8`, returns the recovered
Fireship video with a matching timestamped transcript. This verifies the portable
canary cache reached the production index and public search. The public catalog
now contains 41,480 videos across 69 creators, compared with the initial 41,399.
New uploads contribute to that increase; it is not a count of closed old gaps.

The final worker runs source `d05b21944c9858e189c33806343c01883038b40d`, image
`sha256:8abd1145f46974ed43a0e1553ae0d685b910e97d779e6920bdb375f6da7a5cc2`.
A real loopback connection failure on that deployed binary returned retry exit
70. Whisper remains enabled with a shared 480-second budget. The frontend
deployment `bd647187-aed5-48a6-a4e9-a0794290a4d9` passed its availability smoke
check; public health, creator-filtered `I`, and `fable on high` checks passed.

The 10 GB worker volume had 7.45 GB free, below publication's required 8 GB.
Its 2.44 GB transcript cache included 1.53 GB of resumable ASR work; publication
work contained only 12 KB, so deleting stale builds could not fix it. The worker
volume was extended to 12 GB, preserving those checkpoints and the publication
reserve. The existing publication-only builder remains stopped on its 10 GB
volume. Publication passed the space gate and began downloading the mutation
snapshot through sequence 10212 after the extension.

The catalog sweep and transcription backlog remain active. A fresh complete-run
reconciliation is still required after their publication; mixed historical
manifests must not be presented as a completed repair. Private, paid, unavailable
and speech-free videos retain explicit outcomes.

Validation: `bun run check:precommit` and `bun run check` passed, including
350 Rust tests, all 18 browser scenarios, Worker/frontend tests, operational
fixtures and bundle limits. Deadline and speech-free cache regressions failed
before their fixes and passed afterward. The 12 GB provisioning default is
covered by the cost/capacity configuration guard.
