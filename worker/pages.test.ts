import { createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import { afterEach, describe, expect, test, vi } from "vitest";
import worker from "./index";
import { currentHandle, prefersMarkdown, routePage } from "./page-route";
import { errorForAgents } from "./api-error";
import { renderSharePage, sharedVideoFromSearch } from "./share-page";

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

  test("names YouTube's player for embeds, with a picture every site can fall back to", () => {
    const html = renderSharePage(MOMENT, sharedVideoFromSearch(SEARCH_BODY, MOMENT), `${ORIGIN}/x`);
    const meta = (property: string) =>
      new RegExp(`(?:property|name)="${property}" content="([^"]*)"`).exec(html)?.[1];
    expect(meta("og:video:url")).toBe("https://www.youtube.com/embed/dQw4w9WgXcQ?start=30");
    expect(meta("og:video:type")).toBe("text/html");
    expect(meta("og:image")).toBe("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg");
    // A player card on a domain X has not approved would show no card at all.
    expect(meta("twitter:card")).toBe("summary_large_image");
    expect(html).toContain("youtube-nocookie.com/embed/dQw4w9WgXcQ");
  });

  test("the page opens from search data, as Markdown too", async () => {
    stubApi((url) => {
      expect(url.searchParams.get("video_id")).toBe("dQw4w9WgXcQ");
      return Response.json(SEARCH_BODY);
    });
    const path = "/s/dQw4w9WgXcQ?t=30&end=60&q=line";
    const page = await get(path, "text/html");
    expect(page.status).toBe(200);
    const csp = page.headers.get("Content-Security-Policy");
    expect(csp).toContain("script-src 'self' https://www.youtube.com");
    expect(csp).toContain("frame-src https://www.youtube-nocookie.com https://www.youtube.com");
    expect(await page.text()).toContain("second line");
    const markdown = await (await get(path, "text/markdown")).text();
    expect(markdown).toContain("> first <b>line</b>");
    expect(markdown).toContain("(at 0:30)");
  });

  test.each([
    "Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)",
    "Twitterbot/1.0",
  ])("%s is sent to YouTube's player at the start of the window", async (userAgent) => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const ctx = createExecutionContext();
    const response = await worker.fetch(
      new Request(`${ORIGIN}/s/dQw4w9WgXcQ?t=86&end=116&q=line`, {
        headers: { "User-Agent": userAgent },
        redirect: "manual",
      }),
      testEnv(),
      ctx,
    );
    await waitOnExecutionContext(ctx);
    expect(response.status).toBe(302);
    expect(response.headers.get("Location")).toBe(
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=86s",
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("people, other crawlers and the comparison link get the clip page", async () => {
    stubApi(() => Response.json(SEARCH_BODY));
    for (const [userAgent, query] of [
      ["Mozilla/5.0 (X11; Linux x86_64) Chrome/140", ""],
      ["Slackbot-LinkExpanding 1.0", ""],
      ["Twitterbot/1.0", "&card=page"],
    ] as const) {
      const ctx = createExecutionContext();
      const response = await worker.fetch(
        new Request(`${ORIGIN}/s/dQw4w9WgXcQ?t=30&end=60&q=line${query}`, {
          headers: { "User-Agent": userAgent, Accept: "text/html" },
          redirect: "manual",
        }),
        testEnv(),
        ctx,
      );
      await waitOnExecutionContext(ctx);
      expect(response.status, userAgent).toBe(200);
    }
  });

  test("a moment found by hybrid search looks its lines up the same way", async () => {
    const modes: (string | null)[] = [];
    stubApi((url) => {
      modes.push(url.searchParams.get("mode"));
      return Response.json(SEARCH_BODY);
    });
    await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line&m=hybrid", "text/html");
    await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line", "text/html");
    expect(modes).toEqual(["hybrid", null]);
  });

  test("a search failure still opens the video at its time, with no invented text", async () => {
    stubApi(() => new Response("down", { status: 503 }));
    const response = await get("/s/dQw4w9WgXcQ?t=30&end=60&q=line", "text/html");
    expect(response.status).toBe(200);
    const html = await response.text();
    expect(html).toContain("A shared YouTube moment");
    expect(html).toContain("embed/dQw4w9WgXcQ?start=30");
    expect(html).not.toContain("quote");
  });

  test("a link with no query opens without searching", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    expect((await get("/s/dQw4w9WgXcQ?t=5", "text/html")).status).toBe(200);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});
