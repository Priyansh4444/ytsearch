// Attributes built bundle bytes to source modules and packages.
//
//   bun bundle-attribution.ts <out.json>            measure the current directory
//   bun bundle-attribution.ts --compare <a> <b>     print what grew from a to b
//
// Frontend chunks are measured from Vite source maps (minified bytes per
// source file); the Worker from Wrangler's esbuild metafile (bytes per
// package). bundle-diff.sh runs both sides against two git refs.
/* oxlint-disable no-console -- a CLI whose output is its result. */
import { execFile } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";

type Sizes = Record<string, number>;
type Attribution = { frontend: Record<string, Sizes>; worker: Sizes };

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

// Decodes one source-map segment's base64 VLQ fields.
function decodeSegment(segment: string): number[] {
  const values: number[] = [];
  let value = 0;
  let shift = 0;
  for (const character of segment) {
    const digit = BASE64.indexOf(character);
    value += (digit & 31) << shift;
    if (digit & 32) {
      shift += 5;
    } else {
      values.push(value & 1 ? -(value >>> 1) : value >>> 1);
      value = 0;
      shift = 0;
    }
  }
  return values;
}

// Strips build paths so the same module keys match across worktrees.
function moduleKey(source: string): string {
  const packagePath = source.match(/node_modules\/(.+)$/);
  if (packagePath) return packagePath[1]!;
  return source.replace(/^(\.\.\/)+/, "").replace(/^.*?\/(src|shared|worker)\//, "$1/");
}

function frontendSizes(directory: string): Record<string, Sizes> {
  const chunks: Record<string, Sizes> = {};
  for (const file of readdirSync(directory).filter((name) => name.endsWith(".js"))) {
    const code = readFileSync(join(directory, file), "utf8").split("\n");
    const map = JSON.parse(readFileSync(join(directory, `${file}.map`), "utf8")) as {
      sources: string[];
      mappings: string;
    };
    const sizes: Sizes = {};
    let source = 0;
    map.mappings.split(";").forEach((line, lineIndex) => {
      const lineLength = code[lineIndex]?.length ?? 0;
      let column = 0;
      const segments = line ? line.split(",").map(decodeSegment) : [];
      segments.forEach((fields, index) => {
        column += fields[0]!;
        // The source index is a delta from the last segment that had one,
        // carried across lines.
        if (fields.length >= 4) source += fields[1]!;
        const next = index + 1 < segments.length ? column + segments[index + 1]![0]! : lineLength;
        const key = fields.length >= 4 ? moduleKey(map.sources[source]!) : "[unmapped]";
        sizes[key] = (sizes[key] ?? 0) + Math.max(0, next - column);
      });
    });
    chunks[file.replace(/-[\w-]{8}\.js$/, "")] = sizes;
  }
  return chunks;
}

const execFileAsync = promisify(execFile);

async function run(command: string[]): Promise<void> {
  const [program, ...args] = command as [string, ...string[]];
  await execFileAsync(program, args, { maxBuffer: 64 * 1024 * 1024 });
}

async function measure(output: string): Promise<void> {
  const scratch = mkdtempSync(join(tmpdir(), "ytsearch-bundle-"));
  try {
    await run(["bunx", "vite", "build", "--sourcemap", "--outDir", join(scratch, "web")]);
    // Wrangler's dry run requires the configured assets directory to exist.
    // Create an empty one if needed rather than building into it, so source
    // maps never land where a deploy would upload them.
    const createdAssets = !existsSync("dist");
    if (createdAssets) mkdirSync("dist");
    await run([
      "bunx",
      "wrangler",
      "deploy",
      "--dry-run",
      "--outdir",
      join(scratch, "worker"),
      "--metafile",
      join(scratch, "meta.json"),
    ]).finally(() => {
      if (createdAssets) rmSync("dist", { recursive: true });
    });
    const meta = JSON.parse(readFileSync(join(scratch, "meta.json"), "utf8")) as {
      outputs: Record<string, { inputs?: Record<string, { bytesInOutput: number }> }>;
    };
    const worker: Sizes = {};
    for (const out of Object.values(meta.outputs)) {
      for (const [path, input] of Object.entries(out.inputs ?? {})) {
        const key = path.match(/node_modules\/((?:@[^/]+\/)?[^/]+)/)?.[1] ?? path.split("/")[0]!;
        worker[key] = (worker[key] ?? 0) + input.bytesInOutput;
      }
    }
    const attribution: Attribution = {
      frontend: frontendSizes(join(scratch, "web", "assets")),
      worker,
    };
    writeFileSync(output, JSON.stringify(attribution, null, 1));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

function compare(before: Attribution, after: Attribution): void {
  const total = (sizes: Sizes = {}) => Object.values(sizes).reduce((sum, bytes) => sum + bytes, 0);
  const signed = (bytes: number) => (bytes > 0 ? `+${bytes}` : `${bytes}`);
  console.log("frontend chunks (minified bytes):");
  const chunks = [
    ...new Set([...Object.keys(before.frontend), ...Object.keys(after.frontend)]),
  ].sort();
  for (const chunk of chunks) {
    const [a, b] = [total(before.frontend[chunk]), total(after.frontend[chunk])];
    if (a !== b) console.log(`  ${signed(b - a).padStart(8)}  ${chunk}  (${a} -> ${b})`);
  }
  // Compare modules across all chunks: Vite moves shared modules between
  // chunks, which would otherwise show as a fake loss in one and gain in another.
  const flatten = (attribution: Attribution) => {
    const sizes: Sizes = {};
    for (const chunk of Object.values(attribution.frontend)) {
      for (const [module, bytes] of Object.entries(chunk))
        sizes[module] = (sizes[module] ?? 0) + bytes;
    }
    return sizes;
  };
  const [a, b] = [flatten(before), flatten(after)];
  const moved = [...new Set([...Object.keys(a), ...Object.keys(b)])]
    .map((module) => [module, (b[module] ?? 0) - (a[module] ?? 0)] as const)
    .filter(([, delta]) => Math.abs(delta) >= 200)
    .sort((x, y) => y[1] - x[1]);
  console.log("frontend modules (|change| >= 200 bytes):");
  for (const [module, delta] of moved) console.log(`  ${signed(delta).padStart(8)}  ${module}`);
  console.log("worker packages (bytes):");
  for (const name of [...new Set([...Object.keys(before.worker), ...Object.keys(after.worker)])]) {
    const delta = (after.worker[name] ?? 0) - (before.worker[name] ?? 0);
    if (delta) {
      console.log(
        `  ${signed(delta).padStart(8)}  ${name}  (${before.worker[name] ?? 0} -> ${after.worker[name] ?? 0})`,
      );
    }
  }
}

const [first, second, third] = process.argv.slice(2);
if (first === "--compare" && second && third) {
  compare(JSON.parse(readFileSync(second, "utf8")), JSON.parse(readFileSync(third, "utf8")));
} else if (first) {
  await measure(first);
} else {
  console.error(
    "usage: bun bundle-attribution.ts <out.json> | --compare <before.json> <after.json>",
  );
  process.exit(2);
}
