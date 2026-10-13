// Simulated users for /api/search: each virtual user picks a real query,
// waits for the answer, thinks, and repeats until the duration ends.
//
//   bun web-load.ts --base https://search.pronsh.dev --queries query-log.txt \
//     --users 100 --seconds 120 --think 20000-40000 [--burst] [--per-user-ip]
//
// --burst sends exactly one search per user, all at once.
// --per-user-ip gives each user its own cf-connecting-ip so the API's per-IP
// limit sees 100 clients; it only works against the Fly API hostname directly.
// Output is one JSON summary on stdout; progress goes to stderr.
/* oxlint-disable no-console -- a CLI whose output is its result. */
import { readFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

type Sample = {
  at: number;
  ms: number;
  status: number;
  edgeCached: boolean;
  serverMs: number | undefined;
};

const options = parseOptions(process.argv.slice(2));
const queries = loadQueries(options.queries);
const samples: Sample[] = [];
const failures: { status: number; path: string; error: unknown }[] = [];
const started = performance.now();
const deadline = started + options.seconds * 1000;
let aborted = "";

function parseOptions(argv: string[]) {
  const value = (name: string, fallback?: string) => {
    const index = argv.indexOf(`--${name}`);
    const found = index >= 0 ? argv[index + 1] : fallback;
    if (found === undefined) throw new Error(`missing --${name}`);
    return found;
  };
  const [thinkMin, thinkMax] = value("think", "0-0").split("-").map(Number);
  return {
    base: value("base").replace(/\/$/, ""),
    queries: value("queries"),
    users: Number(value("users", "100")),
    seconds: Number(value("seconds", "60")),
    thinkMin: thinkMin ?? 0,
    thinkMax: thinkMax ?? thinkMin ?? 0,
    burst: argv.includes("--burst"),
    perUserIp: argv.includes("--per-user-ip"),
  };
}

// query-log.txt holds bare legacy queries and "timestamp<TAB>query" lines.
function loadQueries(path: string): string[] {
  const unique = new Set<string>();
  for (const line of readFileSync(path, "utf8").split("\n")) {
    if (!line.trim() || line.startsWith("#")) continue;
    const query = (line.includes("\t") ? line.split("\t")[1] : line)?.trim();
    if (query && query.length <= 200) unique.add(query);
  }
  return [...unique];
}

function pick<T>(items: T[]): T {
  return items[Math.floor(Math.random() * items.length)] as T;
}

// Most searches are a first page by relevance; some page or re-sort.
function searchPath(): string {
  const params = new URLSearchParams({ q: pick(queries) });
  const roll = Math.random();
  if (roll > 0.9) params.set("page", "2");
  else if (roll > 0.8) params.set("sort", pick(["published_desc", "views_desc"]));
  return `/api/search?${params}`;
}

async function search(user: number): Promise<void> {
  const headers: Record<string, string> = { "user-agent": "ytsearch-web-load/1" };
  // 198.18.0.0/15 is reserved for benchmarking, so it never names a real client.
  if (options.perUserIp) headers["cf-connecting-ip"] = `198.18.${user >> 8}.${user & 255}`;
  const at = performance.now();
  let status = 0;
  let edgeCached = false;
  let serverMs: number | undefined;
  const path = searchPath();
  try {
    const response = await fetch(`${options.base}${path}`, {
      headers,
      signal: AbortSignal.timeout(30_000),
    });
    status = response.status;
    const body = (await response.json().catch(() => ({}))) as Record<string, unknown>;
    edgeCached = body.edge_cached === true;
    if (typeof body.time_to_first_result_ms === "number") serverMs = body.time_to_first_result_ms;
    if (status >= 500 && failures.length < 20) failures.push({ status, path, error: body.error });
  } catch (error) {
    status = 0;
    if (failures.length < 20) failures.push({ status, path, error: String(error) });
  }
  samples.push({ at: at - started, ms: performance.now() - at, status, edgeCached, serverMs });
}

async function user(index: number): Promise<void> {
  if (options.burst) return search(index);
  // Stagger arrivals over the first think interval so users don't move in lockstep.
  await sleep(Math.random() * Math.max(options.thinkMax, 1000));
  while (performance.now() < deadline && !aborted) {
    await search(index);
    const think = options.thinkMin + Math.random() * (options.thinkMax - options.thinkMin);
    await sleep(Math.min(think, Math.max(0, deadline - performance.now())));
  }
}

// Stop early if most recent requests fail, so a stress run can't keep a
// struggling production reader pinned.
const guard = setInterval(() => {
  const now = performance.now() - started;
  const recent = samples.filter((sample) => sample.at > now - 10_000);
  const failed = recent.filter((sample) => sample.status === 0 || sample.status >= 500).length;
  if (recent.length >= 20 && failed / recent.length > 0.5)
    aborted = `${failed}/${recent.length} failed in 10 s`;
  console.error(
    `t=${Math.round(now / 1000)}s requests=${samples.length} last10s=${recent.length} failed=${failed}`,
  );
}, 5000);

await Promise.all(Array.from({ length: options.users }, (_, index) => user(index)));
clearInterval(guard);

const percentile = (values: number[], p: number) => {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return Math.round(sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))]!);
};
const latency = (subset: Sample[]) => {
  const ms = subset.map((sample) => sample.ms);
  return {
    n: ms.length,
    p50: percentile(ms, 50),
    p95: percentile(ms, 95),
    p99: percentile(ms, 99),
    max: percentile(ms, 100),
  };
};
const ok = samples.filter((sample) => sample.status === 200);
const elapsed = (performance.now() - started) / 1000;
const statuses: Record<string, number> = {};
for (const sample of samples) statuses[sample.status] = (statuses[sample.status] ?? 0) + 1;

console.log(
  JSON.stringify(
    {
      base: options.base,
      users: options.users,
      mode: options.burst ? "burst" : `think ${options.thinkMin}-${options.thinkMax} ms`,
      perUserIp: options.perUserIp,
      distinctQueries: queries.length,
      startedAt: new Date(Date.now() - elapsed * 1000).toISOString(),
      seconds: Math.round(elapsed),
      requests: samples.length,
      requestsPerSecond: Number((samples.length / elapsed).toFixed(2)),
      statuses,
      aborted: aborted || null,
      failures,
      ok: latency(ok),
      edgeCached: latency(ok.filter((sample) => sample.edgeCached)),
      uncached: latency(ok.filter((sample) => !sample.edgeCached)),
      serverSearchMs: (() => {
        const ms = ok.flatMap((sample) => (sample.serverMs === undefined ? [] : [sample.serverMs]));
        return {
          n: ms.length,
          p50: percentile(ms, 50),
          p95: percentile(ms, 95),
          max: percentile(ms, 100),
        };
      })(),
    },
    null,
    2,
  ),
);
