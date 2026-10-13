# Load testing production search

## Fast path

1. Check the queue worker is not publishing: it wakes at :17 past each hour and
   a publication activates on the reader. Read
   `/var/lib/ytsearch/raw-publication.pending` and `raw-publication.last-completed`
   over `flyctl ssh console`; a routine publication is due 12 hours after the last.
2. Run phases with [`scripts/perf/web-load.ts`](../../scripts/perf/web-load.ts),
   replaying the local `query-log.txt` (refresh it with `scripts/refresh-query-log`):

   ```bash
   # Website path, 100 users searching at once.
   bun scripts/perf/web-load.ts --base https://search.pronsh.dev --queries query-log.txt --users 100 --burst
   # Website path, realistic pacing.
   bun scripts/perf/web-load.ts --base https://search.pronsh.dev --queries query-log.txt --users 100 --seconds 120 --think 20000-40000
   # Reader capacity: API directly, one client IP per user, no think time.
   bun scripts/perf/web-load.ts --base https://ytsearch-api-raw-production.fly.dev --queries query-log.txt \
     --users 100 --seconds 60 --think 0-0 --per-user-ip
   ```

   The script aborts when over half the requests in 10 s fail.
3. Read reader and API CPU, burst balance, throttling and memory for the run
   from Fly Prometheus (`fly_instance_cpu`, `fly_instance_cpu_balance`,
   `fly_instance_cpu_throttle`, `fly_instance_memory_mem_available`), and status
   counts from `fly_app_http_responses_count`.
4. Record the results in [capacity.md](../capacity.md).

## Traps

- One client IP gets 120 searches a minute; beyond that the API answers 429.
  Website runs measure that limit unless their rate stays below 2 req/s.
  Space website runs 60 s apart.
- `--per-user-ip` works only against the Fly API hostname: Cloudflare overwrites
  `cf-connecting-ip` on the website path. The API trusting that header from
  direct callers is also a rate-limit bypass for anyone.
- A burst does not test the edge cache: it stores an answer only after the
  first response, so simultaneous identical searches all reach the reader.
- The API returns `time_to_first_result_ms`, not `search_time_ms`. It includes
  the wait for a reader search slot.

## Facts (2026-10-13)

- Production sustains about 119 req/s with real queries. The ceiling is
  `RAW_SEARCH_MAX_CONCURRENT_SEARCHES=2`: the reader used 2.1 of 4 cores.
- About 17.5 ms of reader CPU per search; the burst balance lasts about 20
  minutes at that rate.
