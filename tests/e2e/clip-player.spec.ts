import { expect, test, type Page } from "@playwright/test";
import { renderSharePage } from "../../worker/share-page";

// A clip from 10 s to 13 s, with one caption line per second.
const MOMENT = {
  videoId: "dQw4w9WgXcQ",
  startSeconds: 10,
  endSeconds: 13,
  query: "two",
  hybrid: false,
};
const CAPTIONS = [
  { startMs: 10_000, endMs: 11_000, text: "line one", matched: false },
  { startMs: 11_000, endMs: 12_000, text: "line two", matched: true },
  { startMs: 12_000, endMs: 13_000, text: "line three", matched: false },
];
const VIDEO = {
  title: "A video",
  creatorName: "Creator",
  creatorHandle: "@creator",
  thumbnail: undefined,
  publishedAt: undefined,
  lines: [],
  captions: CAPTIONS,
};

// Stands in for YouTube's IFrame API: time advances only while it is "playing".
const FAKE_YOUTUBE = `
  window.YT = {
    PlayerState: { UNSTARTED: -1, ENDED: 0, PLAYING: 1, PAUSED: 2, BUFFERING: 3, CUED: 5 },
    Player: class {
      constructor(_frame, options) {
        this.events = options.events;
        this.time = 10;
        this.state = -1;
        this.seeks = [];
        this.volume = 100;
        window.player = this;
        setTimeout(() => this.events.onReady({ target: this }));
      }
      playVideo() {
        this.state = 1;
        clearInterval(this.clock);
        this.clock = setInterval(() => { this.time += 0.05; }, 50);
        this.events.onStateChange({ data: 1 });
      }
      pauseVideo() {
        clearInterval(this.clock);
        this.state = 2;
        this.events.onStateChange({ data: 2 });
      }
      seekTo(time) { this.time = time; this.seeks.push(time); }
      getCurrentTime() { return this.time; }
      getPlayerState() { return this.state; }
      getVolume() { return this.volume; }
      setVolume(volume) { this.volume = volume; }
      isMuted() { return this.muted === true; }
      mute() { this.muted = true; }
      unMute() { this.muted = false; }
      unloadModule() {}
    },
  };
  window.onYouTubeIframeAPIReady();
`;

async function openClip(page: Page, video: typeof VIDEO | undefined = VIDEO): Promise<void> {
  await page.route("**/clip-test", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: renderSharePage(MOMENT, video, "https://search.pronsh.dev/s/dQw4w9WgXcQ"),
    }),
  );
  await page.route("https://www.youtube.com/iframe_api", (route) =>
    route.fulfill({ contentType: "text/javascript", body: FAKE_YOUTUBE }),
  );
  await page.route("https://www.youtube-nocookie.com/**", (route) =>
    route.fulfill({ contentType: "text/html", body: "<!doctype html>" }),
  );
  await page.goto("/clip-test");
  await expect(page.locator("#clip-controls")).toBeVisible();
}

const stage = (page: Page) => page.locator("#clip-stage");
const toggle = (page: Page) => page.locator("#clip-toggle");

test("rests on a still with the matching line and the clip's own length", async ({ page }) => {
  await openClip(page);
  await expect(stage(page)).toHaveAttribute("data-state", "poster");
  await expect(page.locator("#clip-video")).toHaveCSS("opacity", "0");
  await expect(page.locator("#clip-clock")).toHaveText("0:00 / 0:03");
  await expect(page.locator(".captions .is-now")).toHaveText("line two");
});

test("plays, follows the captions, and ends rewound with the replay state", async ({ page }) => {
  await openClip(page);
  await page.locator("#clip-poster").click();
  await expect(stage(page)).toHaveAttribute("data-state", "playing");
  await expect(page.locator("#clip-video")).toHaveCSS("opacity", "1");
  await expect(page.locator(".captions .is-now")).toHaveText("line three", { timeout: 5_000 });

  await expect(stage(page)).toHaveAttribute("data-state", "replay", { timeout: 5_000 });
  await expect(page.locator(".captions .is-now")).toHaveText("line two");
  await expect(page.locator("#clip-clock")).toHaveText("0:00 / 0:03");
  await expect.poll(() => page.evaluate(() => (window as any).player.seeks.at(-1))).toBe(10);
  // The sound faded out for the end and is back for the next play.
  await expect.poll(() => page.evaluate(() => (window as any).player.volume)).toBe(100);
  await expect(toggle(page)).toHaveAttribute("aria-label", "Replay clip");

  await page.locator("#clip-poster").click();
  await expect(stage(page)).toHaveAttribute("data-state", "playing");
});

test("a viewer's pause returns to the still, and play resumes", async ({ page }) => {
  await openClip(page);
  await toggle(page).click();
  await expect(toggle(page)).toHaveAttribute("aria-label", "Pause");
  await toggle(page).click();
  await expect(stage(page)).toHaveAttribute("data-state", "poster");
  await expect(toggle(page)).toHaveAttribute("aria-label", "Play clip");
  await toggle(page).click();
  await expect(stage(page)).toHaveAttribute("data-state", "playing");
});

test("clicking a line plays from it, and the timeline moves between reports", async ({ page }) => {
  await openClip(page);
  await page.locator(".captions button", { hasText: "line three" }).click();
  await expect(stage(page)).toHaveAttribute("data-state", "playing");
  await expect.poll(() => page.evaluate(() => (window as any).player.seeks[0])).toBe(12);
  const scale = () =>
    page.evaluate(
      () => new DOMMatrix(getComputedStyle(document.getElementById("clip-fill")!).transform).a,
    );
  const first = await scale();
  await expect.poll(scale).toBeGreaterThan(first);
});

test("a window with no captions still plays", async ({ page }) => {
  await openClip(page, undefined);
  await page.locator("#clip-poster").click();
  await expect(stage(page)).toHaveAttribute("data-state", "playing");
});
