import { readFileSync, readdirSync } from "node:fs";
import { gzipSync } from "node:zlib";

const budgets = {
  initialJavaScriptGzipBytes: 30_000,
  totalJavaScriptGzipBytes: 55_000,
  cssGzipBytes: 8_000,
};

const html = readFileSync("dist/index.html", "utf8");
const assets = readdirSync("dist/assets");
const javascript = assets.filter((file) => file.endsWith(".js"));
const css = assets.filter((file) => file.endsWith(".css"));
const entryMatch = html.match(/<script[^>]+src="\/assets\/([^"]+\.js)"/);
if (!entryMatch?.[1])
  throw new Error("Could not find the entry JavaScript asset in dist/index.html");
const initialJavaScript = staticJavaScriptGraph(entryMatch[1]);

const measurements = {
  initialJavaScriptGzipBytes: gzipBytes(initialJavaScript),
  totalJavaScriptGzipBytes: gzipBytes(javascript),
  cssGzipBytes: gzipBytes(css),
};
const failures = Object.entries(budgets)
  .filter(([name, budget]) => measurements[name as keyof typeof measurements] > budget)
  .map(
    ([name, budget]) =>
      `${name}: ${measurements[name as keyof typeof measurements]} exceeds ${budget}`,
  );

// oxlint-disable-next-line no-console -- hooks and CI consume this machine-readable line.
console.log(JSON.stringify({ ok: failures.length === 0, measurements, budgets, failures }));
if (failures.length > 0) process.exit(1);

function gzipBytes(files: string[]): number {
  return files.reduce(
    (total, file) => total + gzipSync(readFileSync(`dist/assets/${file}`)).byteLength,
    0,
  );
}

function staticJavaScriptGraph(entry: string): string[] {
  const visited = new Set<string>();
  const pending = [entry];
  const staticImport = /\b(?:import|export)(?:[^"'()]*?from)?["']\.\/([^"']+\.js)["']/g;

  while (pending.length > 0) {
    const file = pending.pop()!;
    if (visited.has(file)) continue;
    if (!javascript.includes(file)) throw new Error(`Missing static JavaScript asset: ${file}`);
    visited.add(file);
    const source = readFileSync(`dist/assets/${file}`, "utf8");
    for (const match of source.matchAll(staticImport)) {
      if (match[1]) pending.push(match[1]);
    }
  }

  return [...visited];
}
