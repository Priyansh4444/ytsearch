import { chromium } from "@playwright/test";

const publicUrl = process.env.PUBLIC_URL ?? "https://search.pronsh.dev";
const configuredHost = process.env.VITE_POSTHOG_HOST?.trim() || "https://us.i.posthog.com";
const apiKeyConfigured = Boolean(process.env.VITE_POSTHOG_KEY?.trim());
const eventPaths = new Set(["/e/", "/i/v0/e/", "/batch/"]);

if (!apiKeyConfigured) {
  throw new Error("VITE_POSTHOG_KEY is required to verify frontend analytics");
}

const posthogOrigin = new URL(configuredHost).origin;
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({
  // PostHog intentionally suppresses HeadlessChrome as bot traffic. Use a normal
  // browser UA so this synthetic operator check exercises the real-user path.
  userAgent:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
});
await page.addInitScript(() => {
  Object.defineProperty(navigator, "webdriver", { configurable: true, get: () => false });
  Object.defineProperty(navigator, "userAgentData", {
    configurable: true,
    get: () => ({
      brands: [
        { brand: "Chromium", version: "140" },
        { brand: "Google Chrome", version: "140" },
      ],
      mobile: false,
      platform: "Linux",
    }),
  });
});

const attemptedRequests: Array<{ method: string; path: string }> = [];
const acceptedRequests: Array<{ path: string; status: number }> = [];
const failedRequests: Array<{ path: string; error: string }> = [];
const pageErrors: string[] = [];
let homeDimensions:
  | { clientHeight: number; clientWidth: number; scrollHeight: number; scrollWidth: number }
  | undefined;

page.on("request", (request) => {
  const url = new URL(request.url());
  if (url.origin !== posthogOrigin || !eventPaths.has(url.pathname)) return;
  attemptedRequests.push({ method: request.method(), path: url.pathname });
});

page.on("response", (response) => {
  const request = response.request();
  const url = new URL(response.url());
  if (
    url.origin !== posthogOrigin ||
    request.method() !== "POST" ||
    !eventPaths.has(url.pathname) ||
    response.status() < 200 ||
    response.status() >= 300
  )
    return;
  acceptedRequests.push({ path: url.pathname, status: response.status() });
});

page.on("requestfailed", (request) => {
  const url = new URL(request.url());
  if (url.origin !== posthogOrigin || !eventPaths.has(url.pathname)) return;
  failedRequests.push({ path: url.pathname, error: request.failure()?.errorText ?? "unknown" });
});

page.on("pageerror", (error) => pageErrors.push(error.message));

try {
  await page.goto(publicUrl, { waitUntil: "networkidle" });
  homeDimensions = await page.evaluate(() => ({
    clientHeight: document.documentElement.clientHeight,
    clientWidth: document.documentElement.clientWidth,
    scrollHeight: document.documentElement.scrollHeight,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  const searchResponse = page.waitForResponse((response) =>
    new URL(response.url()).pathname.endsWith("/api/search"),
  );
  await page.getByPlaceholder("Search transcripts or type @creator").fill("observability canary");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await searchResponse;
  await page.waitForTimeout(15_000);
} finally {
  await browser.close();
}

const result = {
  ok:
    acceptedRequests.length > 0 &&
    failedRequests.length === 0 &&
    pageErrors.length === 0 &&
    homeDimensions !== undefined &&
    homeDimensions.scrollHeight <= homeDimensions.clientHeight &&
    homeDimensions.scrollWidth <= homeDimensions.clientWidth,
  publicUrl,
  posthogOrigin,
  attemptedRequests,
  acceptedRequests,
  failedRequests,
  pageErrors,
  homeDimensions,
};

// oxlint-disable-next-line no-console -- operators consume this machine-readable result.
console.log(JSON.stringify(result));
if (!result.ok) process.exit(1);
