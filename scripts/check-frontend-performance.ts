import { gzipSync } from "node:zlib";
import { readdirSync, readFileSync } from "node:fs";
import { chromium } from "@playwright/test";

const publicUrl = process.env.PUBLIC_URL ?? "https://search.pronsh.dev";
const budgets = {
  javascriptGzipBytes: 110_000,
  cssGzipBytes: 8_000,
  firstContentfulPaintMs: 1_800,
  searchToResultsMs: 2_500,
};

const assets = readdirSync("dist/assets");
const javascriptGzipBytes = gzipBytes(assets.filter((file) => file.endsWith(".js")));
const cssGzipBytes = gzipBytes(assets.filter((file) => file.endsWith(".css")));

const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1_440, height: 1_000 } });
await page.goto(publicUrl, { waitUntil: "networkidle" });
await page.screenshot({ type: "png" });

const firstContentfulPaintSamplesMs = [await readFirstContentfulPaint(page)];
for (let sample = 1; sample < 3; sample += 1) {
  const samplePage = await browser.newPage({ viewport: { width: 1_440, height: 1_000 } });
  await samplePage.goto(publicUrl, { waitUntil: "networkidle" });
  await samplePage.screenshot({ type: "png" });
  firstContentfulPaintSamplesMs.push(await readFirstContentfulPaint(samplePage));
  await samplePage.close();
}
const firstContentfulPaintMs = median(firstContentfulPaintSamplesMs);

async function readFirstContentfulPaint(target: typeof page): Promise<number> {
  return target.evaluate(() => {
  const paints = performance.getEntriesByType("paint");
  return Math.round(paints.find((entry) => entry.name === "first-contentful-paint")?.startTime ?? 0);
  });
}
const expectedChannelCount = await page.evaluate(async () => {
  const response = await fetch("/api/youtubers");
  const body = (await response.json()) as { channels: unknown[] };
  return body.channels.length;
});
const renderedChannelCount = await page.locator(".channel-swarm-item").count();

const input = page.getByRole("textbox");
await input.fill("linux exploit");
const searchStarted = performance.now();
await page.getByRole("button", { name: "Search", exact: true }).click();
await page.locator("article").first().waitFor({ state: "visible" });
const searchToResultsMs = Math.round(performance.now() - searchStarted);

await browser.close();

const measurements = {
  publicUrl,
  javascriptGzipBytes,
  cssGzipBytes,
  firstContentfulPaintMs,
  firstContentfulPaintSamplesMs,
  searchToResultsMs,
  renderedChannelCount,
  expectedChannelCount,
};
const failures = [
  overBudget("JavaScript gzip bytes", javascriptGzipBytes, budgets.javascriptGzipBytes),
  overBudget("CSS gzip bytes", cssGzipBytes, budgets.cssGzipBytes),
  firstContentfulPaintMs > 0 ? undefined : "Browser did not expose a first-contentful-paint entry",
  overBudget("FCP milliseconds", firstContentfulPaintMs, budgets.firstContentfulPaintMs),
  overBudget("search-to-results milliseconds", searchToResultsMs, budgets.searchToResultsMs),
  renderedChannelCount === expectedChannelCount
    ? undefined
    : `Rendered ${renderedChannelCount} of ${expectedChannelCount} indexed channels`,
].filter(Boolean);

// oxlint-disable-next-line no-console -- CI consumes this machine-readable line.
console.log(JSON.stringify({ ok: failures.length === 0, measurements, budgets, failures }));
if (failures.length > 0) process.exit(1);

function gzipBytes(files: string[]): number {
  return files.reduce(
    (total, file) => total + gzipSync(readFileSync(`dist/assets/${file}`)).byteLength,
    0,
  );
}

function overBudget(label: string, actual: number, budget: number): string | undefined {
  return actual <= budget ? undefined : `${label}: ${actual} exceeds ${budget}`;
}

function median(values: number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)] ?? 0;
}
