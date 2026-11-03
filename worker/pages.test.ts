import { createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import { afterEach, describe, expect, test, vi } from "vitest";
import worker from "./index";
import { currentHandle, prefersMarkdown, routePage } from "./page-route";
import { errorForAgents } from "./api-error";
import {
  discordQuote,
  discordTitle,
  renderEmbedPage,
  renderSharePage,
  sharedVideoFromSearch,
  type SharedVideo,
} from "./share-page";

const ORIGIN = "https://search.pronsh.dev";
const ASSETS: Readonly<Partial<Record<string, { body: string; type: string }>>> = {
  "/": {
    body: `<html><head><link href="${ORIGIN}/"></head><h1>home</h1></html>`,
    type: "text/html",
  },
  "/about": { body: "<h1>About</h1>", type: "text/html" },
  "/404": { body: "<h1>Not found</h1>", type: "text/html" },
  "/index.md": { body: `# home at ${ORIGIN}/`, type: "application/octet-stream" },
  "/about.md": { body: "# About", type: "application/octet-stream" },
  "/llms.txt": { body: `# ytsearch ${ORIGIN}/docs`, type: "text/plain" },
  "/.well-known/api-catalog": { body: "{}", type: "application/octet-stream" },
};

const assets = {
  fetch: (input: Request | string) => {
    const path = new URL(typeof input === "string" ? input : input.url).pathname;
    const asset = ASSETS[path];
    return Promise.resolve(
      asset
        ? new Response(asset.body, { headers: { "Content-Type": asset.type, ETag: '"x"' } })
        : new Response("", { status: 404 }),
    );
  },
};

const testEnv = (overrides: Record<string, unknown> = {}) =>
  ({
    ASSETS: assets,
    API_BASE_URL: "https://api.test",
    INDEX_QUEUE_API_BASE_URL: "https://api.test",
    PUBLIC_APP_ORIGIN: ORIGIN,
    SEARCH_SCHEDULE_MODE: "always_on",
    ...overrides,
  }) as unknown as Env;

const CATALOG = { channels: [{ handle: "@theo" }, { handle: "@Ardens." }] };

async function get(path: string, accept?: string, env = testEnv()): Promise<Response> {
  const ctx = createExecutionContext();
  const request = new Request(`${new URL(env.PUBLIC_APP_ORIGIN).origin}${path}`, {
    headers: accept ? { Accept: accept } : {},
    redirect: "manual",
  });
  const response = await worker.fetch(request, env, ctx);
  await waitOnExecutionContext(ctx);
  return response;
}

const stubApi = (handler: (url: URL) => Response) =>
  vi.stubGlobal(
    "fetch",
    vi.fn((input: Request | URL | string) =>
      Promise.resolve(handler(new URL(input instanceof Request ? input.url : input.toString()))),
    ),
  );

afterEach(async () => {
  vi.unstubAllGlobals();
  await caches.default.delete(new Request(`${ORIGIN}/__catalog-handles`));
});

describe("routePage", () => {
  const route = (path: string) => routePage(new URL(path, ORIGIN)).kind;

  test.each([
    ["/", "static"],
    ["/about", "static"],
    ["/docs", "static"],
    ["/dashboard", "app"],
    ["/developers", "redirect"],
    ["/t3dotgg", "redirect"],
    ["/@T3DotGG", "redirect"],
    ["/theo", "asset-or-creator"],
    ["/llms.txt", "asset-or-creator"],
    ["/s/dQw4w9WgXcQ?t=5", "share"],
    ["/s/bad", "asset"],
    ["/embed/dQw4w9WgXcQ?t=5", "embed"],
    ["/embed/bad", "asset"],
    ["/theo/videos", "asset"],
    ["/.well-known/api-catalog", "asset"],
  ])("%s is %s", (path, kind) => {
    expect(route(path)).toBe(kind);
  });

  test("a renamed creator's old link keeps its search", () => {
    expect(routePage(new URL("/t3dotgg?q=rust", ORIGIN))).toEqual({
      kind: "redirect",
      location: "/theo?q=rust",
    });
    expect(currentHandle("@T3DOTGG")).toBe("@theo");
    expect(currentHandle("@ThePrimeagen")).toBe("@ThePrimeagen");
  });
});

describe("prefersMarkdown", () => {
  test.each([
    ["text/markdown", true],
    ["text/markdown, text/html;q=0.5", true],
    ["text/html, text/markdown;q=0.9", false],
    ["text/html", false],
    ["*/*", false],
    ["text/markdown;q=0", false],
    [null, false],
  ])("%s is %s", (accept, expected) => {
    expect(prefersMarkdown(accept)).toBe(expected);
  });
});

describe("pages", () => {
  test("an unknown path answers 404 in the format the client asked for", async () => {
    stubApi(() => Response.json(CATALOG));
    const agent = await get("/some-path-that-does-not-exist");
    expect(agent.status).toBe(404);
    expect(agent.headers.get("Content-Type")).toContain("text/markdown");
    expect(await agent.text()).toContain(`${ORIGIN}/llms.txt`);

    const browser = await get("/some-path-that-does-not-exist", "text/html,*/*;q=0.8");
    expect(browser.status).toBe(404);
    expect(await browser.text()).toContain("Not found");

    expect((await get("/deep/unknown/path")).status).toBe(404);
  });

  test("a known creator serves the app, case-insensitively, even with a dot in the handle", async () => {
    stubApi(() => Response.json(CATALOG));
    for (const path of ["/theo", "/@THEO", "/ardens."]) {
      const response = await get(path, "text/html");
      expect(response.status, path).toBe(200);
      expect(await response.text(), path).toContain("home");
    }
  });

  test("an unreachable catalog serves the app rather than hiding a real creator", async () => {
    stubApi(() => new Response("down", { status: 503 }));
    expect((await get("/theo", "text/html")).status).toBe(200);
  });

  test("pages negotiate Markdown and declare that they vary on it", async () => {
    const html = await get("/about", "text/html");
    expect(html.headers.get("Content-Type")).toContain("text/html");
    expect(html.headers.get("Vary")).toBe("Accept");

    const markdown = await get("/about", "text/markdown");
    expect(markdown.headers.get("Content-Type")).toBe("text/markdown; charset=utf-8");
    expect(markdown.headers.get("Vary")).toBe("Accept");
    expect(await markdown.text()).toBe("# About");
  });

  test("a test deployment points every discovery document at itself", async () => {
    const env = testEnv({ PUBLIC_APP_ORIGIN: "https://search.experimental.pronsh.dev" });
    const own = "https://search.experimental.pronsh.dev";
    expect(await (await get("/", "text/html", env)).text()).toContain(`${own}/`);
    expect(await (await get("/", "text/markdown", env)).text()).toContain(`${own}/`);
    expect(await (await get("/llms.txt", undefined, env)).text()).toContain(`${own}/docs`);
    expect(await (await get("/llms.txt", undefined, env)).text()).not.toContain(ORIGIN);
  });

  test("production documents are served unchanged", async () => {
    expect(await (await get("/llms.txt")).text()).toBe(ASSETS["/llms.txt"]?.body);
  });

  test("redirects old creator links permanently", async () => {
    const response = await get("/t3dotgg?q=rust");
    expect(response.status).toBe(301);
    expect(response.headers.get("Location")).toBe(`${ORIGIN}/theo?q=rust`);
  });
});

describe("API responses for agents", () => {
  test("framework text and empty failures leave as JSON with a hint", async () => {
    const text = await errorForAgents(new Response("unknown variant `bad`", { status: 400 }));
    expect(text.status).toBe(400);
    expect(await text.json()).toMatchObject({
      error: "unknown variant `bad`",
      code: "bad_request",
    });

    const empty = await errorForAgents(
      new Response(null, { status: 405, headers: { Allow: "GET,HEAD" } }),
    );
    expect(empty.headers.get("Allow")).toBe("GET,HEAD");
    expect(await empty.json()).toMatchObject({
      code: "method_not_allowed",
      hint: expect.any(String),
    });

    const unknown = await errorForAgents(new Response("<html>bad gateway</html>", { status: 502 }));
    expect(await unknown.json()).toMatchObject({ code: "unavailable" });
  });

  test("a JSON failure and a success pass through untouched", async () => {
    const failure = Response.json({ error: "query cannot be empty" }, { status: 400 });
    expect(await (await errorForAgents(failure)).json()).toEqual({
      error: "query cannot be empty",
    });
    const success = await errorForAgents(new Response("image", { status: 200 }));
    expect(await success.text()).toBe("image");
  });

  test("the versioned path reaches the same route and states the rate limit", async () => {
    const seen: string[] = [];
    stubApi((url) => {
      seen.push(url.pathname);
      return Response.json({ videos: [] });
    });
    const response = await get("/api/v1/search?q=versioned");
    expect(seen).toEqual(["/api/search"]);
    expect(response.headers.get("RateLimit-Policy")).toBe('"search";q=120;w=60');
    await caches.default.delete(new Request(`${ORIGIN}/api/search?q=versioned`));
  });

  test("searches by a renamed handle ask for the current one", async () => {
    const seen: string[] = [];
    stubApi((url) => {
      seen.push(url.search);
      return Response.json({ videos: [] });
    });
    await get("/api/search?q=rust&youtuber=%40t3dotgg");
    expect(seen).toEqual(["?q=rust&youtuber=%40theo"]);
  });
});

const SEARCH_BODY = {
  videos: [
    {
      video_id: "dQw4w9WgXcQ",
      title: 'Why <script>alert("x")</script> matters',
      youtuber_name: "Theo",
      youtuber_handle: "@theo",
      thumbnail: "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg",
      hits: [
        { match_kind: "transcript", text: "after the window", t_start_ms: 90_000 },
        { match_kind: "transcript", text: "second line", t_start_ms: 40_000, prev_text: "lead in" },
        { match_kind: "transcript", text: "first <b>line</b>", t_start_ms: 30_000 },
        { match_kind: "transcript", text: "before the window", t_start_ms: 29_999 },
        { match_kind: "title", text: "title hit", t_start_ms: 31_000 },
      ],
    },
  ],
};
const MOMENT = {
  videoId: "dQw4w9WgXcQ",
  startSeconds: 30,
  endSeconds: 60,
  query: "line",
  hybrid: false,
};

describe("shared moments", () => {
  test("quotes only real captions inside the window, in order", () => {
    const video = sharedVideoFromSearch(SEARCH_BODY, MOMENT);
    expect(video?.lines.map((line) => line.text)).toEqual(["first <b>line</b>", "second line"]);
    expect(video?.lines[1]?.before).toBe("lead in");
  });

  test("a response for another video or a malformed one yields nothing", () => {
    expect(
      sharedVideoFromSearch(SEARCH_BODY, { ...MOMENT, videoId: "otherVideo1" }),
    ).toBeUndefined();
    expect(sharedVideoFromSearch({ videos: "no" }, MOMENT)).toBeUndefined();
    expect(sharedVideoFromSearch(null, MOMENT)).toBeUndefined();
  });

  test("escapes everything that came from a video or a link", () => {
    const html = renderSharePage(
      { ...MOMENT, query: '"><script>x</script>' },
      sharedVideoFromSearch(SEARCH_BODY, MOMENT),
      `${ORIGIN}/s/dQw4w9WgXcQ`,
    );
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<b>line</b>");
    expect(html).toContain("first &lt;b&gt;line&lt;/b&gt;");
  });

  test("the player stops at the end of the window and the preview names the moment", () => {
    const html = renderSharePage(MOMENT, sharedVideoFromSearch(SEARCH_BODY, MOMENT), `${ORIGIN}/x`);
    expect(html).toContain("embed/dQw4w9WgXcQ?start=30&amp;rel=0&amp;end=60");
    expect(html).toContain("watch?v=dQw4w9WgXcQ&amp;t=30s");
    expect(html).toContain('property="og:description" content="“first &lt;b&gt;line&lt;/b&gt;”"');
    expect(html).toContain("0:30 to 1:00");
  });

  test("gives the clip player its window, and no end still means a bounded clip", () => {
    const attribute = (html: string, name: string) =>
      new RegExp(`${name}="([^"]*)"`).exec(html)?.[1];
    const windowed = renderSharePage(MOMENT, undefined, `${ORIGIN}/x`);
    expect(attribute(windowed, "data-start")).toBe("30");
    expect(attribute(windowed, "data-end")).toBe("60");
    expect(windowed).toContain('src="/clip-player.js"');
    const open = renderSharePage({ ...MOMENT, endSeconds: undefined }, undefined, `${ORIGIN}/x`);
    expect(attribute(open, "data-end")).toBe("90");
  });

  test("declares our own player for X, with a picture every other site can show", () => {
    const html = renderSharePage(MOMENT, sharedVideoFromSearch(SEARCH_BODY, MOMENT), `${ORIGIN}/x`);
    const meta = (property: string) =>
      new RegExp(`(?:property|name)="${property}" content="([^"]*)"`).exec(html)?.[1];
    expect(meta("twitter:card")).toBe("player");
    expect(meta("twitter:player:height")).toBe("480");
    expect(html).toContain('"@type":"VideoObject"');
    expect(html).toContain('"embedUrl":"https://search.pronsh.dev/embed/dQw4w9WgXcQ?t=30');
    expect(html).not.toContain("og:video");
    expect(html).toContain("youtube-nocookie.com/embed/dQw4w9WgXcQ");
  });

  test("the page opens from search data, as Markdown too", async () => {
    const videoIds: (string | null)[] = [];
    stubApi((url) => {
      if (url.pathname === "/api/search") videoIds.push(url.searchParams.get("video_id"));
      return Response.json(SEARCH_BODY);
    });
    const path = "/s/dQw4w9WgXcQ?t=30&end=60&q=line";
    const page = await get(path, "text/html");
    expect(page.status).toBe(200);
    const csp = page.headers.get("Content-Security-Policy");
    expect(csp).toContain("script-src 'self' https://www.youtube.com");
    expect(csp).toContain("frame-src https://www.youtube-nocookie.com https://www.youtube.com");
    const html = await page.text();
    expect(html).toContain('data-start="40000"');
    expect(html).toContain("second <mark>line</mark>");
    const markdown = await (await get(path, "text/markdown")).text();
    expect(markdown).toContain("> first <b>line</b>");
    expect(markdown).toContain("(at 0:30)");
    expect(videoIds).toEqual(["dQw4w9WgXcQ", "dQw4w9WgXcQ"]);
  });

  test("X's crawler reads the player card with the title and quote, not a redirect", async () => {
    stubApi(() => Response.json(SEARCH_BODY));
    const ctx = createExecutionContext();
    const response = await worker.fetch(
      new Request(`${ORIGIN}/s/dQw4w9WgXcQ?t=30&end=60&q=line`, {
        headers: { "User-Agent": "Twitterbot/1.0", Accept: "text/html" },
        redirect: "manual",
      }),
      testEnv(),
      ctx,
    );
    await waitOnExecutionContext(ctx);
    expect(response.status).toBe(200);
    const html = await response.text();
    expect(html).toContain('name="twitter:card" content="player"');
    expect(html).toContain('name="twitter:description" content="“first &lt;b&gt;line&lt;/b&gt;”"');
    expect(html).toContain(
      `name="twitter:player" content="${ORIGIN}/embed/dQw4w9WgXcQ?t=30&amp;end=60&amp;q=line"`,
    );
  });

  test("Discord gets YouTube's player with this site's title and quote", async () => {
    stubApi(() => Response.json(SEARCH_BODY));
    const ctx = createExecutionContext();
    const response = await worker.fetch(
      new Request(`${ORIGIN}/s/dQw4w9WgXcQ?t=30&end=60&q=line&discord=youtube-quote`, {
        headers: {
          "User-Agent": "Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)",
          Accept: "text/html",
        },
        redirect: "manual",
      }),
      testEnv(),
      ctx,
    );
    await waitOnExecutionContext(ctx);
    expect(response.status).toBe(200);
    expect(response.headers.get("Vary")).toContain("User-Agent");
    const html = await response.text();
    expect(html).toContain(
      'property="og:video:url" content="https://www.youtube.com/embed/dQw4w9WgXcQ?start=30"',
    );
    expect(html).toContain('property="og:video:type" content="text/html"');
    expect(html).toContain('property="og:site_name" content="ytsearch"');
    expect(html).toContain(
      'property="og:description" content="“first &lt;b&gt;line&lt;/b&gt; lead in second line',
    );
    expect(html).toContain('property="og:image"');
    expect(html).not.toContain('content="player"');
    expect(html).not.toContain("VideoObject");
  });

  test("X and people never see Discord's card", async () => {
    stubApi(() => Response.json(SEARCH_BODY));
    const ctx = createExecutionContext();
    const response = await worker.fetch(
      new Request(`${ORIGIN}/s/dQw4w9WgXcQ?t=30&end=60&q=line`, {
        headers: { "User-Agent": "Twitterbot/1.0", Accept: "text/html" },
      }),
      testEnv(),
      ctx,
    );
    await waitOnExecutionContext(ctx);
    const html = await response.text();
    expect(html).toContain('name="twitter:card" content="player"');
    expect(html).not.toContain("og:video");
  });

  test("the embed page may be framed by X and nobody else", async () => {
    stubApi(() => Response.json(SEARCH_BODY));
    const response = await get("/embed/dQw4w9WgXcQ?t=30&end=60&q=line", "text/html");
    expect(response.status).toBe(200);
    const csp = response.headers.get("Content-Security-Policy");
    expect(csp).toContain(
      "frame-ancestors 'self' https://x.com https://*.x.com https://twitter.com https://*.twitter.com",
    );
    expect(csp).not.toContain("frame-ancestors 'none'");
    expect(csp).toContain("script-src 'self' https://www.youtube.com");
    expect(response.headers.get("X-Frame-Options")).toBeNull();
    const page = await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line", "text/html");
    expect(page.headers.get("Content-Security-Policy")).toContain("frame-ancestors 'none'");
    expect(page.headers.get("X-Frame-Options")).toBe("DENY");
  });

  test("the embed shows the clip, its title and the real quote, escaped", () => {
    const html = renderEmbedPage(
      MOMENT,
      sharedVideoFromSearch(SEARCH_BODY, MOMENT),
      `${ORIGIN}/s/x`,
    );
    expect(html).toContain("data-autoplay");
    expect(html).toContain('data-end="60"');
    expect(html).toContain("first &lt;b&gt;<mark>line</mark>&lt;/b&gt;");
    expect(html).toContain('id="clip-captions"');
    expect(html).not.toContain("<script>alert");
    expect(html).toContain('name="twitter:player:width" content="480"');
  });

  test("a moment found by hybrid search looks its lines up the same way", async () => {
    const modes: (string | null)[] = [];
    stubApi((url) => {
      modes.push(url.searchParams.get("mode"));
      return Response.json(SEARCH_BODY);
    });
    await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line&m=hybrid", "text/html");
    await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line", "text/html");
    expect(modes.filter((mode) => mode === "hybrid")).toHaveLength(1);
    expect(modes).toHaveLength(4);
  });

  test("a video the index does not have is a real 404, on both pages", async () => {
    stubApi((url) =>
      url.pathname.endsWith("/transcript")
        ? Response.json({ error: "video not found" }, { status: 404 })
        : Response.json({ videos: [] }),
    );
    expect((await get("/s/dQw4w9WgXcQ?t=30&q=line", "text/html")).status).toBe(404);
    expect((await get("/embed/dQw4w9WgXcQ?t=30&q=line", "text/html")).status).toBe(404);
  });

  test("a search failure still opens the video at its time, with no invented text", async () => {
    stubApi(() => new Response("down", { status: 503 }));
    const response = await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line", "text/html");
    expect(response.status).toBe(200);
    const html = await response.text();
    expect(html).toContain("A shared YouTube moment");
    expect(html).toContain("embed/dQw4w9WgXcQ?start=30");
    expect(html).not.toContain("clip-captions");
  });

  test("captions come from the window's transcript, with or without a query", async () => {
    const paths: string[] = [];
    stubApi((url) => {
      paths.push(url.pathname + url.search);
      return Response.json({
        video_id: "dQw4w9WgXcQ",
        title: "From the transcript",
        youtuber_name: "Theo",
        youtuber_handle: "@theo",
        lines: [
          { t_start_ms: 30_000, t_end_ms: 34_000, text: "every line" },
          { t_start_ms: 34_000, t_end_ms: 38_000, text: "with no gaps" },
        ],
      });
    });
    const page = await get("/s/dQw4w9WgXcQ?t=30&end=60", "text/html");
    expect(paths).toEqual(["/api/videos/dQw4w9WgXcQ/transcript?start_ms=30000&end_ms=60000"]);
    const html = await page.text();
    expect(html).toContain("From the transcript");
    expect(html).toContain("every line");
    expect(html).toContain('data-start="34000"');
    expect(html).not.toContain("<mark>");
  });

  test("a failed transcript still leaves the search's own lines", async () => {
    stubApi((url) =>
      url.pathname.endsWith("/transcript")
        ? new Response("down", { status: 503 })
        : Response.json(SEARCH_BODY),
    );
    const html = await (await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line", "text/html")).text();
    expect(html).toContain("second <mark>line</mark>");
  });
});

describe("Discord's text limits", () => {
  const bytes = (text: string) => new TextEncoder().encode(text).length;
  const line = (text: string, matched = false) => ({ startMs: 0, endMs: 1, text, matched });
  const video = (captions: SharedVideo["captions"]): SharedVideo => ({
    title: "t",
    creatorName: "c",
    creatorHandle: "@c",
    thumbnail: undefined,
    publishedAt: undefined,
    lines: [],
    captions,
  });

  test("a quote starts at the matching line and carries on through what follows", () => {
    const quote = discordQuote(
      video([line("said before"), line("the match", true), line("what follows")]),
      350,
    );
    expect(quote).toBe("“the match what follows”");
  });

  test("a long quote fills the limit in bytes, cut at a word, with its marks closed", () => {
    const quote = discordQuote(video([line("héllo wörld ".repeat(80), true)]), 350)!;
    expect(bytes(quote)).toBeLessThanOrEqual(350);
    expect(bytes(quote)).toBeGreaterThan(320);
    expect(quote.endsWith("…”")).toBe(true);
    expect(quote).not.toMatch(/\s…/);
  });

  test("a limit too small for any word still ends, and whitespace cannot stall it", () => {
    expect(discordTitle("   ", "1:26 to 1:56")).toContain("(1:26 to 1:56)");
    expect(discordQuote(video([line("x".repeat(500), true)]), 10)).toBe("“…”");
    expect(discordQuote(video([line(`${"word ".repeat(200)}   `, true)]), 40)!.endsWith("…”")).toBe(
      true,
    );
  });

  test("with no matching line it starts at the first, and with no words it is absent", () => {
    expect(discordQuote(video([line("first"), line("second")]), 350)).toBe("“first second”");
    expect(discordQuote(video([]), 350)).toBeUndefined();
    expect(discordQuote(undefined, 350)).toBeUndefined();
  });

  test("a long title gives way so the window always shows", () => {
    const title = discordTitle("A very long video title ".repeat(6), "1:26 to 1:56");
    expect(bytes(title)).toBeLessThanOrEqual(70);
    expect(title.endsWith("… (1:26 to 1:56)")).toBe(true);
    expect(discordTitle("Short", "0:01 to 0:30")).toBe("Short (0:01 to 0:30)");
  });
});
