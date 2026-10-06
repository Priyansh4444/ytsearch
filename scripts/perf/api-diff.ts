// Replays the same requests against two API binaries that share one reader
// and compares status, caching headers and the full JSON body. Called by
// api-diff.sh, which builds the binaries and starts the reader.
//
//   bun api-diff.ts <cases.tsv> <api-base> <api-head> <reader-port> <work-dir>
//
// Prints one line per mismatching request with its first difference, then
// `requests=N mismatches=M`. Exits non-zero on any mismatch.
/* oxlint-disable no-console -- a CLI whose output is its result. */
import { type ChildProcess, spawn } from "node:child_process";
import { openSync, readFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";

import { searchCaseParams } from "./search-cases";

const [casesFile, apiBase, apiHead, readerPort, workDir] = process.argv.slice(2);
if (!casesFile || !apiBase || !apiHead || !readerPort || !workDir) {
  console.error(
    "usage: bun api-diff.ts <cases.tsv> <api-base> <api-head> <reader-port> <work-dir>",
  );
  process.exit(2);
}
const BASE_PORT = 18141;
const HEAD_PORT = 18142;
// Timing is the only thing allowed to differ between two runs.
const TIMING_FIELDS = new Set([
  "time_to_first_result_ms",
  "search_time_ms",
  "duration_ms",
  "backend_search_ms",
  "first_backend_search_ms",
  "result_processing_ms",
]);
const HEADERS = ["content-type", "cache-control"];
// Endpoints and validation errors the benchmark cases do not reach.
const EXTRA_PATHS = [
  "/api/health",
  "/api/youtubers",
  "/api/index-status",
  "/api/index-status?handle=@t3dotgg",
  "/api/index-status?handle=bad!!",
  "/api/search",
  "/api/search?q=%20%20",
  "/api/search?q=I&youtuber=%40t3dotgg",
  "/api/search?q=fable%20on%20high&youtuber=%40t3dotgg",
  "/api/search?q=fire&sort=bogus",
  "/api/search?q=fire&per_page=0",
  "/api/search?q=fire&per_page=500",
  "/api/search?q=fire&page=0",
  "/api/search?q=fire&page=999",
  "/api/search?q=fire&youtuber=@nobody",
  "/api/search?q=fire&video_id=bad!",
  "/api/search?q=fire&video_id=",
  "/api/search?q=fire&video_id=%20",
  "/api/search?q=",
  "/api/search?q=fire&per_page=1",
  "/api/search?q=fire&per_page=5",
  "/api/nope",
];
// Single-video searches (the frontend's "show all moments") for the first two
// videos of each case's first page, capped like the original harness.
const MAX_SCOPED_VIDEOS = 60;
const SCOPED_VARIANTS: Record<string, string>[] = [
  { per_page: "100" },
  {},
  { page: "2", per_page: "3" },
  { sort: "published_desc" },
];

function startApi(name: string, binary: string, port: number): ChildProcess {
  const log = openSync(`${workDir}/${name}.log`, "w");
  // A minimal environment and a scratch cwd, so neither the shell's variables
  // nor a .env (the API loads one with dotenvy) change what is compared.
  return spawn(binary, [], {
    cwd: workDir,
    env: {
      PATH: process.env.PATH,
      RAW_SEARCH_API_URL: `http://[::1]:${readerPort}`,
      RAW_SEARCH_API_KEY: "perf-local",
      YTSEARCH_API_ADDR: `127.0.0.1:${port}`,
      YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE: "0",
      YTSEARCH_ENGAGEMENT_SORTS: "true",
      YTSEARCH_INDEX_QUEUE_PATH: `${workDir}/${name}-queue.jsonl`,
    },
    stdio: ["ignore", log, log],
  });
}

async function waitHealthy(name: string, api: ChildProcess, port: number): Promise<void> {
  const deadline = Date.now() + 60_000;
  for (;;) {
    const healthy = await fetch(`http://127.0.0.1:${port}/api/health`).then(
      (response) => response.ok,
      () => false,
    );
    if (healthy) return;
    if (api.exitCode !== null || Date.now() > deadline) {
      // The work directory is removed on exit, so show the log here.
      const log = readFileSync(`${workDir}/${name}.log`, "utf8").trimEnd().split("\n");
      throw new Error(`${name} API did not become healthy:\n${log.slice(-5).join("\n")}`);
    }
    await sleep(200);
  }
}

type Snapshot = { status: number; headers: Record<string, string | null>; body: unknown };

async function fetchSnapshot(port: number, path: string): Promise<Snapshot> {
  const response = await fetch(`http://127.0.0.1:${port}${path}`);
  const text = await response.text();
  let body: unknown = text;
  try {
    body = JSON.parse(text);
  } catch {
    // Not JSON: compare the text as is.
  }
  return {
    status: response.status,
    headers: Object.fromEntries(HEADERS.map((name) => [name, response.headers.get(name)])),
    body,
  };
}

function show(value: unknown): string {
  const text = value === undefined ? "(missing)" : JSON.stringify(value);
  return text.length > 100 ? `${text.slice(0, 97)}...` : text;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// The first path at which the two values differ, ignoring timing fields and
// key order.
function firstDifference(base: unknown, head: unknown, path: string): string | undefined {
  if (Array.isArray(base) && Array.isArray(head)) {
    for (let index = 0; index < Math.max(base.length, head.length); index++) {
      if (index >= base.length || index >= head.length) {
        return `${path}: length ${base.length} != ${head.length}`;
      }
      const difference = firstDifference(base[index], head[index], `${path}[${index}]`);
      if (difference) return difference;
    }
    return undefined;
  }
  if (isRecord(base) && isRecord(head)) {
    for (const key of new Set([...Object.keys(base), ...Object.keys(head)])) {
      if (TIMING_FIELDS.has(key)) continue;
      const difference = firstDifference(base[key], head[key], `${path}.${key}`);
      if (difference) return difference;
    }
    return undefined;
  }
  return Object.is(base, head) ? undefined : `${path || "."}: ${show(base)} != ${show(head)}`;
}

const base = startApi("base", apiBase, BASE_PORT);
const head = startApi("head", apiHead, HEAD_PORT);
const stop = () => {
  base.kill();
  head.kill();
};
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    stop();
    process.exit(130);
  });
}

let requests = 0;
let mismatches = 0;
try {
  await waitHealthy("base", base, BASE_PORT);
  await waitHealthy("head", head, HEAD_PORT);
  const compare = async (path: string): Promise<unknown> => {
    requests++;
    const [a, b] = [await fetchSnapshot(BASE_PORT, path), await fetchSnapshot(HEAD_PORT, path)];
    const difference =
      firstDifference(a.status, b.status, "status") ??
      firstDifference(a.headers, b.headers, "headers") ??
      firstDifference(a.body, b.body, "body");
    if (difference) {
      mismatches++;
      console.log(`MISMATCH ${path}\n  ${difference}`);
    }
    return a.body;
  };
  const scoped: [query: string, videoId: string][] = [];
  for (const params of searchCaseParams(casesFile)) {
    const body = await compare(`/api/search?${params}`);
    if (params.get("sort") !== "relevance" || params.get("page") !== "1") continue;
    const videos = isRecord(body) && Array.isArray(body.videos) ? body.videos.slice(0, 2) : [];
    for (const video of videos) {
      if (isRecord(video) && typeof video.video_id === "string") {
        scoped.push([params.get("q") ?? "", video.video_id]);
      }
    }
  }
  for (const [q, videoId] of scoped.slice(0, MAX_SCOPED_VIDEOS)) {
    for (const variant of SCOPED_VARIANTS) {
      await compare(`/api/search?${new URLSearchParams({ q, video_id: videoId, ...variant })}`);
    }
    // The API trims the ID; a blank one once skipped the early stop.
    await compare(`/api/search?${new URLSearchParams({ q, video_id: `  ${videoId}  ` })}`);
  }
  for (const path of EXTRA_PATHS) await compare(path);
} finally {
  stop();
}
console.log(`requests=${requests} mismatches=${mismatches}`);
process.exit(mismatches ? 1 : 0);
