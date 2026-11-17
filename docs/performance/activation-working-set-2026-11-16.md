# Activation working-set repair (2026-11-16)

The strict SJC release run at `698ee47` rejected activation requests up to
843 ms and a warm request at 301 ms. Search contracts passed, but reader
memory reached 2,436,263,936 bytes and swap reached 1,449,984 bytes.
Compilation and median latency did not establish production recovery.

## Causes and changes

- The receiver wrote duplicate files into separate inodes even when an active
  immutable file had identical bytes. Reuse now requires matching manifest size
  and digest, rehashes the actual source, and still consumes and validates the
  entire incoming body. Corruption and mismatched incoming data remain errors.
- New upload pages displaced the active working set. Newly written files now
  release their clean upload cache; reused files never enter that eviction path.
- Warming covered legacy core files but missed schema-9 query sidecars and
  snippet pages. Activation now replays up to 64 distinct recent successful
  requests, preserving complete parameters with an 8-KiB entry bound. History
  stays in memory and query text is not added to logs or persisted.
- A timed-out warm-up previously still returned a supposedly warmed engine.
  Warm-up now fails closed at its work deadline and reaps native work before
  releasing publication ownership. Retries cannot overlap unfinished warmers.
- Dropping the old engine held the serving write lock; SQLite acknowledgement
  and job persistence ran on the async request thread. Retirement now releases
  the serving lock first, and the complete activation tail owns its publication
  lock on a background thread even if the caller disconnects.

## Independent corpus replay

The opt-in publisher tests build an additional synthetic video from the real
source archives, preserving all 8,142,650 original documents. The replacement
has 8,142,651 documents and independently answers its unique fixture term.
The replay uses physically distinct old/new inode sets, bypassing reuse. Each
run evicts only its owned copies, then runs in a separate 2.5-GiB memory cgroup
with swap disabled. All 55 canonical query top-result lists remain unchanged.

| Changed-video first sweep | Legacy core warm-up | Recent-request warm-up |
| --- | ---: | ---: |
| Maximum engine request | 343.108 ms | 83.849 ms |
| Major page faults after activation | 3,458 | 0 |
| Cgroup memory peak | 2,026,618,880 bytes | 2,038,579,200 bytes |
| Swap | 0 | 0 |
| Activation preparation | 1,664 ms | 5,319 ms |

The improvement schedules work before the swap; it is not a claim that total
work disappeared. These are local engine replay measurements, not browser E2E
or proof that the production release gate passes.

Reproduction uses the ignored `build_full_corpus_changed_video_fixture` and
`profile_corpus_activation_working_set` publisher tests. Supply the documented
`YTSEARCH_*` inputs from each test's ignore message, and run the latter with
`systemd-run --user --scope -p MemoryMax=2560M -p MemorySwapMax=0`. Use separate
owned corpus copies; never evict global caches or production mappings.

## Release measurement repairs

The resource collector now samples on the VM through one bounded SSH stream.
Connection setup and operator scheduling no longer inflate sample timestamps.
The existing six-second maximum gap still rejects a real seven-second VM stall;
malformed frames, failed startup and unexpected closure fail closed. A live
read-only check collected ten samples with a maximum gap of 1,025 ms.

The canary now explicitly mirrors production's two-search backend concurrency;
its previous omitted setting silently selected the code default of one. The
contract checker also avoids a reserved jq variable name and passes its existing
positive and negative fixtures on jq 1.6 as well as the local jq 1.8 installation.

No CPU, memory, swap, latency, disk or load-coverage limits were increased by
this repair. Production deployment still requires the complete strict gate,
followed by public request and browser click-to-painted-results checks.
