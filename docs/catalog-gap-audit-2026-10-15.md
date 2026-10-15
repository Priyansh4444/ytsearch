# Creator video gap audit, 2026-10-15

The production snapshot captured around 12:18 UTC reports 41,399 indexed videos
across 69 creators and 42,427 reported public videos. Its count difference is
1,028. The live catalog continues to change. This is a diagnostic snapshot,
not a fresh end-to-end reconciliation or a completed backfill.

## Findings

- The failure ledger contained 970 `no_captions` records. The cloud indexer has
  no speech transcription binary, so those videos need usable captions or a
  permitted locally generated transcript cache before they can be searched.
- HealthyGamerGG's 108 `unknown` failures were members-only videos. Other
  creators had the same error, and two Vsauce records required payment.
  Restricted videos now classify as `private` instead of repeatedly retrying.
- A Google for Developers caption failed because JSON3 contained a start time
  of `-10`. The parser now accepts signed timing, clips the start at zero and
  preserves the end. Negative durations remain invalid.
- An update retaining existing videos could return success before reporting
  failed new videos. All successful indexing paths now share the retry outcome.
- Captionless uploads with a known publication date within seven days now
  receive a bounded scheduled retry. Old or unknown-date terminal records are
  preserved, avoiding a sudden retry of the whole captionless corpus.
- Seven already-indexed creators were absent from the 62-entry maintenance list.
  The checked-in list now includes all 69 observed profiles. The scheduled
  catalog pass also merges the public indexed directory, so newly approved
  creators remain covered when their existing counts match.
- No unindexed video in the captured discovery manifests had a matching local
  transcript cache in `data/transcript-cache`; there was no ready cached
  transcript to upload as an immediate repair.

## Evidence limits

The generation-pinned admin catalog was stable at generation 10006, while
sequence 10010 was durable. Four mutations were not yet published. The 65
available discovery manifests span 59 catalog run IDs; four creators have no
captured discovery manifest. Comparing those manifests to the active generation
finds 1,095 absent IDs, a different population from the 1,028 profile count
difference. These numbers must not be combined or called a fresh repair backlog.

Among the 1,095 IDs observed absent from the active snapshot, captured failure
categories explain 952 as `no_captions`, 118 as `unknown`, 17 as `scheduled`, six
as `bot_challenge`, and one as `unavailable`. One ID lacks a classified outcome,
`Dwci2xV0xgo` for `@bmdavis419`. The `unknown` labels are from the production
binary before the restricted-video classifier fix. Thirty-nine recorded failure
IDs are already indexed; 21 are outside captured discovery. This is why
subtracting raw failure totals from a count difference gives misleading results.

## Every observed creator

The absent-ID and category columns use captured discovery evidence. An em dash
means evidence is missing, not that the creator has no missing videos. Counts
are from the active generation; profiles can differ from later discovery.

| Creator | Indexed | Reported public | Count difference | Absent IDs in captured discovery | Captured reasons |
| --- | ---: | ---: | ---: | ---: | --- |
| @3blue1brown | 161 | 162 | +1 | 1 | no_captions 1 |
| @_johnhammond | 1,679 | 1,706 | +27 | 27 | no_captions 27 |
| @aidenbai | 16 | 21 | +5 | 5 | no_captions 5 |
| @aidotengineer | 1,380 | 1,380 | +0 | 5 | no_captions 4, scheduled 1 |
| @andrejkarpathy | 15 | 17 | +2 | 2 | no_captions 2 |
| @appledeveloper | 1,027 | 1,030 | +3 | 3 | no_captions 2, scheduled 1 |
| @appliedscience | 269 | 278 | +9 | 9 | no_captions 9 |
| @ardens. | 28 | 29 | +1 | 2 | no_captions 2 |
| @arjay_the_dev | 61 | 61 | +0 | 0 | No absent IDs observed |
| @asianometry | 717 | 728 | +11 | 11 | no_captions 11 |
| @bawad | 1,083 | 1,118 | +35 | 35 | no_captions 25, scheduled 10 |
| @benjdicken | 83 | 83 | +0 | 0 | No absent IDs observed |
| @bettersoftwareconference | 17 | 17 | +0 | 0 | No absent IDs observed |
| @bmdavis419 | 204 | 205 | +1 | 2 | scheduled 1, unclassified 1 |
| @brancheducation | 67 | 68 | +1 | 1 | no_captions 1 |
| @brodierobertson | 2,369 | 2,387 | +18 | 18 | no_captions 18 |
| @bycloudai | 254 | 259 | +5 | 6 | no_captions 6 |
| @codebullet | 86 | 94 | +8 | 8 | no_captions 8 |
| @commonluke | 27 | 32 | +5 | 5 | unknown 5 |
| @computerphile | 918 | 924 | +6 | 6 | no_captions 6 |
| @convex-dev | 236 | 244 | +8 | 8 | no_captions 8 |
| @cursor_ai | 57 | 58 | +1 | 1 | no_captions 1 |
| @defconconference | 4,642 | 4,730 | +88 | 88 | no_captions 88 |
| @dougdoug | 215 | 216 | +1 | 1 | no_captions 1 |
| @dreamsofcode | 121 | 121 | +0 | — | No discovery manifest |
| @dwarkeshpatel | 275 | 275 | +0 | 1 | no_captions 1 |
| @fireship | 778 | 805 | +27 | 27 | no_captions 27 |
| @fknight | 363 | 374 | +11 | 11 | no_captions 11 |
| @googledevelopers | 5,891 | 6,113 | +222 | 222 | no_captions 220, unknown 2 |
| @hankschannel | 640 | 663 | +23 | 24 | no_captions 24 |
| @healthygamergg | 1,121 | 1,234 | +113 | 114 | no_captions 7, unknown 107 |
| @hoog-youtube | 63 | 63 | +0 | — | No discovery manifest |
| @hyperplexed | 96 | 97 | +1 | 1 | no_captions 1 |
| @jeffgeerling | 493 | 535 | +42 | 42 | no_captions 42 |
| @lauriewired | 113 | 118 | +5 | 5 | no_captions 5 |
| @leerob | 103 | 105 | +2 | 2 | no_captions 2 |
| @lexfridman | 824 | 869 | +45 | 45 | no_captions 45 |
| @liveoverflow | 423 | 428 | +5 | 5 | no_captions 4, unknown 1 |
| @lowleveltv | 399 | 399 | +0 | 0 | No absent IDs observed |
| @marimo-team | 172 | 172 | +0 | 0 | No absent IDs observed |
| @masterdotdev | 306 | 307 | +1 | 1 | no_captions 1 |
| @mattpocockuk | 164 | 165 | +1 | 1 | no_captions 1 |
| @michaelreeves | 52 | 53 | +1 | 1 | no_captions 1 |
| @mollyrocket | 800 | 826 | +26 | 26 | no_captions 26 |
| @neetcode | 476 | 482 | +6 | 6 | no_captions 5, scheduled 1 |
| @nerd-snipe | 79 | 80 | +1 | 1 | no_captions 1 |
| @planetscale | 176 | 178 | +2 | 2 | no_captions 2 |
| @portalrunner | 67 | 67 | +0 | — | No discovery manifest |
| @practicalengineeringchannel | 246 | 252 | +6 | 6 | no_captions 5, scheduled 1 |
| @pragmaticengineer | 108 | 108 | +0 | 0 | No absent IDs observed |
| @rasmic | 161 | 161 | +0 | 0 | No absent IDs observed |
| @reactconferences | 673 | 699 | +26 | 26 | no_captions 25, scheduled 1 |
| @reducible | 23 | 23 | +0 | 0 | No absent IDs observed |
| @rossmanngroup | 4,189 | 4,321 | +132 | 178 | bot_challenge 1, no_captions 175, scheduled 1, unavailable 1 |
| @ryanlpeterman | 538 | 539 | +1 | 1 | no_captions 1 |
| @ryansolid | 144 | 144 | +0 | 0 | No absent IDs observed |
| @sebastianlague | 283 | 297 | +14 | 14 | no_captions 14 |
| @smartereveryday | 371 | 398 | +27 | 27 | bot_challenge 2, no_captions 25 |
| @t3dotgg | 1,113 | 1,118 | +5 | 5 | no_captions 5 |
| @technologyconnections | 231 | 233 | +2 | 2 | no_captions 2 |
| @theavocoder | 9 | 9 | +0 | 0 | No absent IDs observed |
| @theprimeagen | 106 | 109 | +3 | 3 | bot_challenge 1, no_captions 2 |
| @thestanduppod | 76 | 76 | +0 | — | No discovery manifest |
| @tomscottgo | 754 | 761 | +7 | 7 | no_captions 7 |
| @twominutepapers | 1,085 | 1,103 | +18 | 18 | no_captions 18 |
| @vercelhq | 444 | 456 | +12 | 12 | no_captions 12 |
| @veritasium | 536 | 540 | +4 | 4 | bot_challenge 2, no_captions 2 |
| @vimjoyer | 82 | 83 | +1 | 1 | unknown 1 |
| @vsauce | 621 | 621 | +0 | 10 | no_captions 8, unknown 2 |

## Finish the production repair

The changes are local and need deployment before they affect production. Keep
`update`, `JOBS=1` and `CHANNEL_JOBS=1`; do not use `rewipe`.

1. Deploy the reviewed dependency, caption parser, retry, and maintenance changes
   through the existing release workflow. Deploy the frontend redesign separately.
2. Run one full catalog update over all indexed creators on the existing queue
   worker. Do not start a second writer or create one queue row per creator,
   which would rebuild the whole generation repeatedly.
3. Allow caption cooldowns and provider backoff to expire before retrying.
   Prioritize Google for Developers after the parser fix and investigate the
   unclassified Ben Davis ID. Access restrictions require an appropriate source;
   repeated fetches cannot supply unavailable captions.
4. Publish once after the catalog pass, then run
   `scripts/check-raw-creator-reconciliation.sh` with discovery from that run.
   Require every discovered ID to be indexed or classified, with no unpublished
   mutations, stale records, or absent profiles.

Reproduce the diagnostic with `scripts/report-video-gaps.py CAPTURE.json`; its
input contract is in the script. The [indexing gaps playbook](playbooks/indexing-gaps.md)
records the safe capture workflow. Raw captured files remain under
`/tmp/ytsearch-maintenance-20261015`; they contain no copied credentials.

## Validation

- `bun run check:precommit`: passed on the final dependency versions.
- `bun run check`: passed, including 337 Rust tests, 127 frontend tests,
  40 Worker tests, all 18 browser scenarios without retries, Hawk with zero
  findings, license/dependency checks, and the operational script fixtures.
- Reader dependency parity: 330 real-index requests, zero mismatches. Wall-time
  ratio 0.998 against the previous build; no ranking changes were introduced.
- Existing bundle budgets pass: initial JavaScript 33,989 gzip bytes, total
  JavaScript 52,933, CSS 7,972, and Worker 51,005 gzip bytes.
- The current dependency audit leaves only the documented compatibility holds.
  Bun's security audit reports zero vulnerabilities across 374 packages; the
  Rust audit also passed.
- Registry manifests confirm the Bun 1.4.3, Deno 2.9.7, and nginx 1.31 images
  exist for production's platform. Container smoke tests were not run because
  the local Docker daemon socket denies access.
- Live search health passed. `I` returned Theo's October 14, 13, and 12 uploads
  in descending publication order; `fable on high` placed `A proper guide to
  Fable 5` second. Both existing indexer Machines were stopped after inspection.
- The live queue configuration still embeds the old creator list. The local
  `check-queue-indexers.sh` correctly reports this deployment drift until the
  reviewed 69-creator list is rolled out. No new indexer image or redesigned
  frontend has been deployed, and no full backfill is claimed by this audit.

Validation logs are under `/tmp/ytsearch-maintenance-20261015`, especially
`precommit-final.log`, `check-complete.log`, `health-final.log`, and
`image-tags.log`.
