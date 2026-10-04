// Load driver for scripts/perf/*.sh: replays reader requests straight against
// one or two readers, bypassing the API and its response cache.
//
//   bun reader-drive.ts <requests.txt> <runs> <port>          time one reader
//   bun reader-drive.ts <requests.txt> <runs> <portA> <portB> interleaved A/B
//
// A/B mode alternates A and B for every request, swapping which goes first
// each run, so both see the same CPU state. It also hashes every response
// (minus timing fields) and counts requests whose answers differ.
/* oxlint-disable no-console -- a CLI whose output is its result. */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

const [requestsFile, runsArgument, ...ports] = process.argv.slice(2);
if (!requestsFile || !runsArgument || ports.length < 1 || ports.length > 2) {
  console.error("usage: bun reader-drive.ts <requests.txt> <runs> <port> [port]");
  process.exit(2);
}
const runs = Number(runsArgument);
const requests = readFileSync(requestsFile, "utf8")
  .split("\n")
  .map((line) => line.trim())
  .filter(Boolean);
const TIMING_FIELDS = new Set(["search_time_ms", "search_cutoff"]);

function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .filter((key) => !TIMING_FIELDS.has(key))
        .sort()
        .map((key) => [key, canonical((value as Record<string, unknown>)[key])]),
    );
  }
  return value;
}

async function get(port: string, path: string): Promise<{ ms: number; hash: string }> {
  const started = performance.now();
  const response = await fetch(`http://[::1]:${port}${path}`, {
    headers: { "x-raw-search-key": "perf-local" },
  });
  const body = await response.text();
  const ms = performance.now() - started;
  if (!response.ok) throw new Error(`${response.status} from :${port} for ${path}: ${body}`);
  const hash = createHash("sha256")
    .update(JSON.stringify(canonical(JSON.parse(body))))
    .digest("hex");
  return { ms, hash };
}

const median = (values: number[]) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
};
const quantile = (sorted: number[], q: number) =>
  sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))] ?? 0;

// One untimed pass per reader warms the page cache and lazily built state.
for (const path of requests) for (const port of ports) await get(port, path);

if (ports.length === 1) {
  const [port] = ports as [string];
  const medians: number[] = [];
  for (const path of requests) {
    const times: number[] = [];
    for (let run = 0; run < runs; run++) times.push((await get(port, path)).ms);
    medians.push(median(times));
  }
  const sorted = [...medians].sort((a, b) => a - b);
  const sum = medians.reduce((total, ms) => total + ms, 0);
  console.log(
    `requests=${medians.length} sum_ms=${sum.toFixed(0)} p50=${quantile(sorted, 0.5).toFixed(2)} ` +
      `p90=${quantile(sorted, 0.9).toFixed(2)} p99=${quantile(sorted, 0.99).toFixed(2)}`,
  );
} else {
  const [portA, portB] = ports as [string, string];
  let sumA = 0;
  let sumB = 0;
  let mismatches = 0;
  const logRatios: number[] = [];
  for (const path of requests) {
    const times: Record<string, number[]> = { [portA]: [], [portB]: [] };
    const hashes: Record<string, string> = {};
    for (let run = 0; run < runs; run++) {
      for (const port of run % 2 === 0 ? [portA, portB] : [portB, portA]) {
        const { ms, hash } = await get(port, path);
        times[port]!.push(ms);
        hashes[port] = hash;
      }
    }
    if (hashes[portA] !== hashes[portB]) {
      mismatches += 1;
      console.error(`response differs: ${path}`);
    }
    const a = median(times[portA]!);
    const b = median(times[portB]!);
    sumA += a;
    sumB += b;
    logRatios.push(Math.log(b / a));
  }
  logRatios.sort((x, y) => x - y);
  const geomean = Math.exp(logRatios.reduce((total, x) => total + x, 0) / logRatios.length);
  console.log(
    `wall: A_sum_ms=${sumA.toFixed(0)} B_sum_ms=${sumB.toFixed(0)} ratio=${(sumB / sumA).toFixed(3)} ` +
      `geomean=${geomean.toFixed(3)} p10=${Math.exp(quantile(logRatios, 0.1)).toFixed(3)} ` +
      `p90=${Math.exp(quantile(logRatios, 0.9)).toFixed(3)}`,
  );
  console.log(`parity: requests=${requests.length} mismatches=${mismatches}`);
  if (mismatches > 0) process.exitCode = 1;
}
