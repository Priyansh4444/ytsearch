import { expect, test, type Page } from "@playwright/test";

const channel = {
  id: "example",
  handle: "@example",
  name: "Example Creator",
  channel_id: "UCexample",
  public_video_count: 20,
  indexed_video_count: 10,
  indexed_chunk_count: 50,
};

const searchResponse = {
  query: "rust testing",
  total_moments: 1,
  total_videos: 1,
  total_found: 1,
  out_of: 1,
  time_to_first_result_ms: 8,
  page: 1,
  per_page: 12,
  sort: "relevance",
  mode: "lexical",
  has_next_page: false,
  hits: [],
  videos: [
    {
      video_id: "video1",
      title: "Reliable Rust testing",
      youtuber_handle: "@example",
      youtuber_name: "Example Creator",
      content_kind: "video",
      match_count: 1,
      youtube_url: "https://www.youtube.com/watch?v=video1&t=12s",
      embed_url: "about:blank",
      best_timestamp: "0:12",
      hits: [
        {
          id: "moment1",
          video_id: "video1",
          title: "Reliable Rust testing",
          text: "A practical approach to Rust testing.",
          match_kind: "transcript",
          youtuber_handle: "@example",
          youtuber_name: "Example Creator",
          content_kind: "video",
          t_start_ms: 12_000,
          timestamp: "0:12",
          youtube_url: "https://www.youtube.com/watch?v=video1&t=12s",
          embed_url: "about:blank",
        },
      ],
    },
  ],
};

async function mockApi(page: Page) {
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/youtubers") {
      return route.fulfill({ json: { channels: [channel] } });
    }
    if (url.pathname === "/api/health") {
      return route.fulfill({ json: { ok: true } });
    }
    if (url.pathname === "/api/index-status") {
      return route.fulfill({ json: { handle: "", status: "idle" } });
    }
    if (url.pathname === "/api/search") {
      return route.fulfill({
        json: {
          ...searchResponse,
          query: url.searchParams.get("q"),
          mode: url.searchParams.get("mode") ?? "lexical",
        },
      });
    }
    return route.fulfill({ status: 404, json: { error: "not mocked" } });
  });
}

test.beforeEach(async ({ page }) => {
  await mockApi(page);
});

test("loads channels and completes a basic search", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Example Creator").first()).toBeVisible();

  await page.getByPlaceholder("Search transcripts or type @creator").fill("rust testing");
  await page.getByRole("button", { name: "Search" }).click();

  await expect(page).toHaveURL(/q=rust\+testing/);
  await expect(page.getByRole("link", { name: /Reliable Rust testing/ })).toBeVisible();
  await expect(page.getByText("1 video · 1 moment · 8 ms to first result")).toBeVisible();
});

test("semantic mode remains opt-in and is sent to the API", async ({ page }) => {
  await page.goto("/");
  const mode = page.getByRole("combobox", { name: "Search mode" });
  await expect(mode).toHaveValue("lexical");
  await mode.selectOption("semantic");

  const requestPromise = page.waitForRequest(
    (request) => request.url().includes("/api/search?") && request.url().includes("mode=semantic"),
  );
  await page.getByPlaceholder("Search transcripts or type @creator").fill("security bug");
  await page.getByRole("button", { name: "Search" }).click();
  await requestPromise;

  await expect(page).toHaveURL(/mode=semantic/);
  await expect(page.locator("article")).toHaveCount(1);
});

test("dashboard reports catalog and operational metrics", async ({ page }) => {
  await page.goto("/dashboard");

  await expect(page).toHaveTitle("Dashboard · YouTube Search");
  await expect(page.getByRole("heading", { name: "System dashboard" })).toBeVisible();
  await expect(page.getByText("Indexed videos").first()).toBeVisible();
  await expect(page.getByText("Transcript moments").first()).toBeVisible();
  await expect(page.getByText("Operational")).toBeVisible();
  await expect(page.getByText("Example Creator")).toBeVisible();
  await expect(page.getByText("10 / 20")).toBeVisible();
  await expect(page.getByText("50%").first()).toBeVisible();
});

for (const viewport of [
  { name: "small phone", width: 320, height: 700 },
  { name: "tablet", width: 768, height: 900 },
  { name: "desktop", width: 1280, height: 900 },
]) {
  test(`dashboard fits a ${viewport.name} viewport`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.goto("/dashboard");
    await expect(page.getByRole("heading", { name: "System dashboard" })).toBeVisible();
    await expect(page.getByText("Example Creator")).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    }));
    expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  });
}

for (const viewport of [
  { name: "small phone", width: 320, height: 700 },
  { name: "tablet", width: 768, height: 900 },
  { name: "desktop", width: 1280, height: 900 },
]) {
  test(`search UI fits a ${viewport.name} viewport`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.goto("/?q=rust+testing");
    await expect(page.locator("article")).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      clientWidth: document.documentElement.clientWidth,
      scrollWidth: document.documentElement.scrollWidth,
    }));
    expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);

    for (const element of [
      page.getByPlaceholder("Search transcripts or type @creator"),
      page.getByRole("button", { name: "Search" }),
      page.locator("article"),
    ]) {
      const box = await element.boundingBox();
      expect(box).not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width + 1);
    }
  });
}
