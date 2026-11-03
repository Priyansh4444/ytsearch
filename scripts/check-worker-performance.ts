import { execFile } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { gzipSync } from "node:zlib";

// Raised from 225_000/50_000 for Effect 4.0.0, which adds about 16 KB raw
// over 4.0.0-rc.111. effect/Schema (two Fly response decoders in
// worker/fly-machine.ts) is about 97 KB of the bundle.
// Raised from 235_000/52_000 for the pages agents read: routing, Markdown
// negotiation and shared moment rendering add about 17 KB raw.
const budgets = {
  workerRawBytes: 255_000,
  workerGzipBytes: 58_000,
};
const outdir = mkdtempSync(join(tmpdir(), "ytsearch-worker-performance-"));
const execFileAsync = promisify(execFile);

try {
  await execFileAsync("bunx", ["wrangler", "deploy", "--dry-run", "--outdir", outdir]);

  const bundle = readFileSync(join(outdir, "index.js"));
  const measurements = {
    workerRawBytes: bundle.byteLength,
    workerGzipBytes: gzipSync(bundle).byteLength,
  };
  const failures = Object.entries(budgets)
    .filter(([name, budget]) => measurements[name as keyof typeof measurements] > budget)
    .map(
      ([name, budget]) =>
        `${name}: ${measurements[name as keyof typeof measurements]} exceeds ${budget}`,
    );

  // oxlint-disable-next-line no-console -- hooks and CI consume this machine-readable line.
  console.log(JSON.stringify({ ok: failures.length === 0, measurements, budgets, failures }));
  if (failures.length > 0) process.exitCode = 1;
} finally {
  rmSync(outdir, { recursive: true, force: true });
}
