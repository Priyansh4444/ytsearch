# Rust performance record

Date: 2026-08-01

## Multi-page search ranking

Workload: eight backend pages, 128 videos, four transcript hits per video, and a five-term relevance query. The benchmark runs 40 end-to-end rounds in an optimized build.

Invariant: final video ordering and every retained moment must remain identical. Backend fetching, deduplication, nearby-hit merging, pagination, and relevance scoring semantics may not change.

Hypothesis: `collect_visible_videos` spends material CPU cloning, grouping, normalizing, and sorting the entire growing hit collection after every backend page only to ask whether enough distinct videos have arrived.

Change: use a borrowed, early-exit `HashSet<&str>` probe to answer the distinct-video predicate on intermediate pages. Run the existing full grouping/ranking pass only when returning the final page result. The established final grouping implementation is unchanged.

Result from three same-process comparisons:

| Run | Previous behavior | Distinct probe | Speedup |
| --- | ---: | ---: | ---: |
| 1 | 166.96 ms | 71.60 ms | 2.33× |
| 2 | 158.90 ms | 72.00 ms | 2.21× |
| 3 | 155.70 ms | 63.55 ms | 2.45× |

The benchmark asserts exact final video-ID order parity. The API optimized test suite passes 24 tests; the three manual performance comparisons remain ignored during normal test runs.

Rejected experiments:

- Caching normalized hit strings and query-term coverage increased the representative grouping median from 0.520 ms to 0.540 ms per round. The bookkeeping cost did not clear the keep threshold.
- Consuming hits into an index-map/vector grouping representation increased the median to 0.681 ms. Moving large result structs through a growing vector cost more than the removed clones in this workload.

Decision: keep the whole-pass elimination and benchmark; reject both inner-loop rewrites. Revisit only when a new profile shows final ranking, rather than repeated intermediate ranking, dominates request time.

## Release profile experiment

Workload: clean release builds of `youtube-api` and `ytsearch`, plus raw and gzip artifact size.

| Profile | Clean build | API raw / gzip | CLI raw / gzip |
| --- | ---: | ---: | ---: |
| Default release | 13.78 s | 8,239,016 / 3,472,019 B | 9,164,936 / 3,845,261 B |
| Thin LTO | 44.20 s | 8,287,616 / 3,518,062 B | 9,191,600 / 3,900,099 B |

Decision: reject Thin LTO. It made both artifacts larger and made the clean comparison build about 3.2× slower. No manifest/profile change was retained.

## Safety and tooling audit

- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `unsafe` blocks under `crates/`: none.
- Miri: unavailable on the installed stable toolchain; no UB-coverage claim is made.
- The isolated Thin-LTO target directory was removed after measurement and contained only reproducible build artifacts.

