import { expect, test, type Page } from "@playwright/test";
import type { SearchAvailability } from "../../src/types";

test.use({ timezoneId: "America/Los_Angeles" });

const channel = {
  id: "example",
  handle: "@example",
  name: "Example Creator",
  channel_id: "UCexample",
  public_video_count: 20,
  indexed_video_count: 10,
  indexed_chunk_count: 50,
};

const searchMoment = {
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
};

const searchResponse = {
  query: "rust testing",
  total_moments: 1,
  total_videos: 1,
  totals_approximate: false,
  total_found: 1,
  out_of: 1,
  time_to_first_result_ms: 8,
  page: 1,
  per_page: 12,
  sort: "relevance",
  mode: "lexical",
  has_next_page: false,
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
      hits: [searchMoment],
    },
  ],
};

const readyAvailability: SearchAvailability = {
  status: "ready",
  search_enabled: true,
  enforced: true,
  schedule_policy: "scheduled",
  schedule_phase: "open",
  time_zone: "America/Los_Angeles",
  daily_window: {
    opens: "12:00",
    closes: "00:00",
    label: "12:00 PM–12:00 AM Pacific Time",
  },
  next_transition_at: "2026-07-27T07:00:00.000Z",
};

const solidRuntimeWarnings = new WeakMap<Page, string[]>();

async function mockApi(page: Page, availability = readyAvailability, channels = [channel]) {
  let searchRequests = 0;
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/availability") {
      return route.fulfill({ json: availability });
    }
    if (url.pathname === "/api/youtubers") {
      return route.fulfill({ json: { channels } });
    }
    if (url.pathname === "/api/health") {
      return route.fulfill({ json: { ok: true } });
    }
    if (url.pathname === "/api/index-status") {
      return route.fulfill({ json: { handle: "", status: "idle" } });
    }
    if (url.pathname === "/api/search") {
      searchRequests += 1;
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
  return { searchRequests: () => searchRequests };
}

test.beforeEach(async ({ page }) => {
  const warnings: string[] = [];
  solidRuntimeWarnings.set(page, warnings);
  const recordSolidWarning = (message: string) => {
    if (/STRICT_READ_UNTRACKED|ASYNC_OUTSIDE_LOADING_BOUNDARY/.test(message)) {
      warnings.push(message);
    }
  };
  page.on("console", (message) => recordSolidWarning(message.text()));
  page.on("pageerror", (error) => recordSolidWarning(error.message));
  await mockApi(page);
});

test.afterEach(async ({ page }) => {
  expect(solidRuntimeWarnings.get(page) ?? [], "Solid emitted an async-boundary warning").toEqual(
    [],
  );
});

test("renders the search shell while an initial lazy chunk is delayed", async ({ page }) => {
  let chunkRequested = false;
  const { promise: chunkGate, resolve: releaseChunk } = Promise.withResolvers<void>();
  await page.route("**/src/components/channel-picker.tsx*", async (route) => {
    chunkRequested = true;
    await chunkGate;
    await route.continue();
  });

  const navigation = page.goto("/");
  await expect
    .poll(() => chunkRequested, {
      message: "the cold Vite server should eventually request the delayed lazy chunk",
      timeout: 15_000,
    })
    .toBe(true);
  let shellFailure: unknown;
  try {
    await expect(page.getByRole("combobox", { name: "Search transcripts" })).toBeVisible({
      timeout: 1_000,
    });
  } catch (error) {
    shellFailure = error;
  }
  releaseChunk();
  await navigation;
  if (shellFailure) throw shellFailure;
  await expect(page.getByText("Example Creator").first()).toBeVisible();
});

test("copies a result permalink that scrolls back to that video", async ({ page }) => {
  const videos = Array.from({ length: 8 }, (_, index) => ({
    ...searchResponse.videos[0],
    video_id: `video${index}`,
    title: `Reliable Rust testing ${index}`,
    youtube_url: `https://www.youtube.com/watch?v=video${index}&t=12s`,
    embed_url: "about:blank",
    hits: [
      {
        ...searchMoment,
        id: `moment${index}`,
        video_id: `video${index}`,
        title: `Reliable Rust testing ${index}`,
        youtube_url: `https://www.youtube.com/watch?v=video${index}&t=12s`,
      },
    ],
  }));
  await page.unroute("**/api/**");
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/availability") {
      return route.fulfill({ json: readyAvailability });
    }
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
          total_videos: videos.length,
          total_moments: videos.length,
          total_found: videos.length,
          videos,
        },
      });
    }
    return route.fulfill({ status: 404, json: { error: "not mocked" } });
  });

  await page.setViewportSize({ width: 1_280, height: 700 });
  await page.goto("/?q=rust+testing");
  const lastResult = page.locator("article").last();
  await expect(lastResult).toBeVisible();
  await lastResult.getByRole("button", { name: "Copy result link" }).click();
  await expect(page).toHaveURL(/q=rust\+testing#video-video7$/);

  await page.evaluate(() => window.scrollTo(0, 0));
  await page.reload({ waitUntil: "networkidle" });
  const linkedResult = page.locator("#video-video7");
  await expect(linkedResult).toBeVisible();
  await expect
    .poll(async () => {
      const box = await linkedResult.boundingBox();
      return box ? box.y : Number.POSITIVE_INFINITY;
    })
    .toBeLessThan(160);
});

test("loads channels and completes a basic search", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("Example Creator").first()).toBeVisible();

  await page.getByRole("combobox", { name: "Search transcripts" }).fill("rust testing");
  await page.getByRole("button", { name: "Search" }).click();

  await expect(page).toHaveURL(/q=rust\+testing/);
  await expect(page.getByRole("link", { name: /Reliable Rust testing/ })).toBeVisible();
  await expect(page.getByText("All channels", { exact: true })).toBeVisible();
  await expect(page.getByText(/1 video · 1 moment · \d+ ms request · 8 ms search/)).toBeVisible();
  await expect(page.locator("iframe")).toHaveCount(0);
  await page.getByRole("button", { name: "Play Reliable Rust testing at 0:12" }).click();
  await expect(page.locator("iframe")).toHaveCount(1);
  await expect(page.locator("iframe")).toHaveAttribute("src", /autoplay=1/);
});

test("a playing video survives the periodic availability refresh", async ({ page }) => {
  // Production is always on, but the client's provisional availability follows
  // the scheduled window. Before noon Pacific that guess says search is closed,
  // and resetting to it on every refresh unmounted the results mid-playback.
  await page.unroute("**/api/**");
  const api = await mockApi(page, {
    ...readyAvailability,
    enforced: false,
    schedule_policy: "always_on",
    schedule_phase: "closed",
  });
  await page.clock.install({ time: new Date("2026-07-26T16:00:00Z") });

  await page.goto("/?q=rust+testing");
  await page.getByRole("button", { name: "Play Reliable Rust testing at 0:12" }).click();
  const player = page.locator("iframe");
  await expect(player).toHaveCount(1);
  await player.evaluate((element) => element.setAttribute("data-original-player", ""));

  await page.clock.runFor(61_000);
  await expect(page.getByText("Loading", { exact: false })).toHaveCount(0);

  await expect(page.locator("iframe[data-original-player]")).toHaveCount(1);
  expect(api.searchRequests()).toBe(1);
});

test("changing the sort re-sorts the submitted search", async ({ page }) => {
  await page.goto("/?q=rust+testing");
  await expect(page.getByRole("link", { name: /Reliable Rust testing/ })).toBeVisible();
  // Typing a new query without submitting must not leak into the re-sort.
  await page.getByRole("combobox", { name: "Search transcripts" }).fill("unsubmitted");

  const resorted = page.waitForRequest((request) => {
    const url = new URL(request.url());
    return url.pathname === "/api/search" && url.searchParams.get("sort") === "published_desc";
  });
  await page.getByLabel("Sort results").selectOption("published_desc");

  const url = new URL((await resorted).url());
  expect(url.searchParams.get("q")).toBe("rust testing");
  expect(url.searchParams.get("page")).toBe("1");
  await expect(page).toHaveURL(/sort=published_desc/);
});

test("searches battery gate within @t3dotgg", async ({ page }) => {
  const searchRequest = page.waitForRequest((request) => {
    const url = new URL(request.url());
    return (
      url.pathname === "/api/search" &&
      url.searchParams.get("q") === "battery gate" &&
      url.searchParams.get("youtuber") === "@t3dotgg"
    );
  });

  await page.goto("/?q=battery+gate&youtuber=%40t3dotgg");
  await searchRequest;

  await expect(page.getByRole("combobox", { name: "Search transcripts" })).toHaveValue(
    "battery gate",
  );
  await expect(page.getByText("Channel: @t3dotgg")).toBeVisible();
  await expect(page.getByRole("link", { name: /Reliable Rust testing/ })).toBeVisible();
});

test("closed homepage renders immediately with private recent searches", async ({ page }) => {
  await page.unroute("**/api/**");
  await page.clock.setFixedTime(new Date("2026-07-26T17:00:00Z"));
  const api = await mockApi(page, {
    ...readyAvailability,
    status: "scheduled_offline",
    search_enabled: false,
    schedule_phase: "closed",
    next_transition_at: "2026-07-26T19:00:00.000Z",
  });
  await page.addInitScript(() => {
    localStorage.setItem(
      "ytsearch.recent-searches.v1",
      JSON.stringify([
        {
          query: "rust testing",
          youtuber: "@example",
          sort: "relevance",
          mode: "lexical",
          searchedAt: "2026-07-25T20:00:00.000Z",
        },
      ]),
    );
  });

  await page.goto("/?q=bookmarked");

  await expect(page.getByText(/Search returns at 12:00 PM in 2h 0m 0s/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Available at noon" })).toBeDisabled();
  await expect(page.getByRole("heading", { name: "Recent searches" })).not.toBeVisible();
  await expect(page.locator(".animate-pulse")).toHaveCount(0);
  expect(api.searchRequests()).toBe(0);

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Recent searches" })).toBeVisible();
  await page.getByRole("button", { name: /rust testing/i }).click();
  await expect(page.getByRole("combobox", { name: "Search transcripts" })).toHaveValue(
    "rust testing",
  );
  expect(api.searchRequests()).toBe(0);
});

test("channel swarm fills the home canvas without clipping hover labels", async ({ page }) => {
  await page.unroute("**/api/**");
  const channels = Array.from({ length: 43 }, (_, index) => ({
    ...channel,
    id: `example-${index}`,
    handle: `@example${index}`,
    name: `Example Creator ${index}`,
    channel_id: `UCexample${index}`,
  }));
  await mockApi(page, readyAvailability, channels);
  await page.setViewportSize({ width: 1_280, height: 800 });
  await page.goto("/");

  const swarm = page.getByLabel("Choose a YouTube channel");
  await expect(swarm).toBeVisible();
  await expect(swarm).toHaveCSS("pointer-events", "none");
  const stage = await swarm.boundingBox();
  expect(stage).not.toBeNull();
  expect(stage!.width).toBeGreaterThan(1_000);
  expect(stage!.height).toBeGreaterThan(680);

  const items = swarm.getByRole("link");
  await expect(items).toHaveCount(43);
  await expect(items.first()).toHaveCSS("pointer-events", "auto");
  const positions = await items.evaluateAll((elements) =>
    elements.map((element) => {
      const style = (element as HTMLElement).style;
      return {
        x: Number.parseFloat(style.getPropertyValue("--swarm-x")),
        y: Number.parseFloat(style.getPropertyValue("--swarm-y")),
      };
    }),
  );
  const ovalDistances = positions.map(({ x, y }, index) => {
    const ring = index % 2;
    const radiusX = ring === 0 ? 44 : 36;
    const radiusY = ring === 0 ? 43 : 35;
    return ((x - 50) / radiusX) ** 2 + ((y - 50) / radiusY) ** 2;
  });
  expect(ovalDistances.every((distance) => Math.abs(distance - 1) < 0.02)).toBe(true);
  expect(
    new Set(
      await items.evaluateAll((elements) =>
        elements.map((element) => (element as HTMLElement).style.getPropertyValue("--swarm-color")),
      ),
    ).size,
  ).toBe(5);

  for (const index of [0, 2, 4, 6, 8, 10, 12, 14]) {
    await items.nth(index).hover();
    const label = await items.nth(index).locator(".channel-swarm-label").boundingBox();
    expect(label).not.toBeNull();
    expect(label!.x).toBeGreaterThanOrEqual(0);
    expect(label!.x + label!.width).toBeLessThanOrEqual(1_280);
    expect(label!.y).toBeGreaterThanOrEqual(0);
    expect(label!.y + label!.height).toBeLessThanOrEqual(800);
  }
  const firstBox = await items.first().boundingBox();
  expect(firstBox).not.toBeNull();
  await page.mouse.move(firstBox!.x + firstBox!.width / 2 + 80, firstBox!.y + firstBox!.height / 2);
  await expect
    .poll(() => items.first().evaluate((element) => element.style.getPropertyValue("--magnet-x")))
    .not.toBe("0px");
  await expect
    .poll(() => swarm.evaluate((element) => getComputedStyle(element, "::before").opacity))
    .toBe("1");

  const searchInput = page.getByRole("combobox", { name: "Search transcripts" });
  await searchInput.click();
  await searchInput.fill("interaction still works");
  await expect(searchInput).toHaveValue("interaction still works");
});

test("creator suggestions push the schedule down instead of overlaying it", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("combobox", { name: "Search transcripts" }).fill("@exam");

  const suggestions = page.getByTestId("channel-suggestions");
  const schedule = page.getByRole("status");
  await expect(suggestions).toBeVisible();
  await expect(schedule).toBeVisible();
  const suggestionBox = await suggestions.boundingBox();
  const scheduleBox = await schedule.boundingBox();
  expect(suggestionBox).not.toBeNull();
  expect(scheduleBox).not.toBeNull();
  expect(suggestionBox!.y + suggestionBox!.height).toBeLessThanOrEqual(scheduleBox!.y);
});

for (const viewport of [
  { name: "phone", width: 390, height: 844 },
  { name: "desktop", width: 1_280, height: 900 },
]) {
  test(`home page has no phantom scrollbar on ${viewport.name}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.goto("/");
    // Phones show avatars only, so match the creator link's accessible name.
    await expect(page.getByRole("link", { name: "Search Example Creator" })).toBeVisible();

    const dimensions = await page.evaluate(() => ({
      clientHeight: document.documentElement.clientHeight,
      clientWidth: document.documentElement.clientWidth,
      scrollHeight: document.documentElement.scrollHeight,
      scrollWidth: document.documentElement.scrollWidth,
    }));
    expect(dimensions.scrollHeight).toBeLessThanOrEqual(dimensions.clientHeight);
    expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
  });
}

test("dashboard reports catalog and operational metrics", async ({ page }) => {
  await page.goto("/dashboard");

  await expect(page).toHaveTitle("Dashboard · YouTube Search");
  await expect(page.getByRole("heading", { name: "System dashboard" })).toBeVisible();
  await expect(page.getByText("Indexed videos").first()).toBeVisible();
  await expect(page.getByText("Transcript moments").first()).toBeVisible();
  await expect(page.getByText("Operational")).toBeVisible();
  await expect(page.getByText("Scheduled repair")).toBeVisible();
  await expect(page.getByText("10 count gap · 6h repair + daily refresh")).toBeVisible();
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
      page.getByRole("combobox", { name: "Search transcripts" }),
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
