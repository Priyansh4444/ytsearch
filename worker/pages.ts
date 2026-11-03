import { sharedMomentPath, type SharedMoment } from "../shared/share-link";
import {
  discordCard,
  prefersHtml,
  prefersMarkdown,
  routePage,
  isDiscordCrawler,
  type PageRoute,
} from "./page-route";
import {
  clipWindow,
  renderEmbedPage,
  renderShareMarkdown,
  renderSharePage,
  sharedVideoFromSearch,
  type LinkCard,
  type SharedVideo,
} from "./share-page";

const CANONICAL_ORIGIN = "https://search.pronsh.dev";
const MARKDOWN = "text/markdown; charset=utf-8";
const CATALOG_CACHE_SECONDS = 300;
const SHARE_LOOKUP_TIMEOUT_MS = 8_000;
// Long enough for an API that stops when idle to boot; only the first miss waits.
const CATALOG_LOOKUP_TIMEOUT_MS = 30_000;
const AGENT_LINKS = '</llms.txt>; rel="describedby", </openapi.json>; rel="service-desc"';
// Mirrors public/_headers, which only covers responses served from the assets binding.
const SECURITY_HEADERS: Readonly<Record<string, string>> = {
  "Content-Security-Policy":
    "default-src 'self'; script-src 'self' https://static.cloudflareinsights.com; style-src 'self'; style-src-attr 'unsafe-inline'; img-src 'self' data: https://i.ytimg.com; frame-src https://www.youtube-nocookie.com; connect-src 'self' https://us.i.posthog.com; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'",
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "strict-origin-when-cross-origin",
  "Permissions-Policy": "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
  "X-Frame-Options": "DENY",
};

// The clip player drives YouTube's own player through its IFrame API.
const SHARE_PAGE_CSP = (SECURITY_HEADERS["Content-Security-Policy"] ?? "")
  .replace("script-src 'self'", "script-src 'self' https://www.youtube.com")
  .replace(
    "frame-src https://www.youtube-nocookie.com",
    "frame-src https://www.youtube-nocookie.com https://www.youtube.com",
  );

// X loads the embed page in an iframe, so only it may be framed, and only by X.
const { "X-Frame-Options": _noFrameOptions, ...EMBED_SECURITY_HEADERS } = SECURITY_HEADERS;
const EMBED_PAGE_CSP = SHARE_PAGE_CSP.replace(
  "frame-ancestors 'none'",
  "frame-ancestors 'self' https://x.com https://*.x.com https://twitter.com https://*.twitter.com",
);

type PageEnvironment = {
  readonly ASSETS: Fetcher;
  readonly API_BASE_URL: string;
  readonly PUBLIC_APP_ORIGIN?: string;
};

export async function pageResponse(
  request: Request,
  url: URL,
  env: PageEnvironment,
  fetchSearch: (path: string, signal: AbortSignal) => Promise<Response>,
): Promise<Response> {
  const route = routePage(url);
  const markdown = prefersMarkdown(request.headers.get("Accept"));
  switch (route.kind) {
    case "static":
      return negotiatedAsset(url, route, markdown, env);
    case "app":
      return negotiated(await assetAt(url, "/", env), env);
    case "share":
      return shareResponse(
        route.moment,
        markdown,
        isDiscordCrawler(request.headers.get("User-Agent"))
          ? discordCard(url.searchParams.get("discord"))
          : "player",
        env,
        fetchSearch,
      );
    case "embed":
      return embedResponse(route.moment, env, fetchSearch);
    case "redirect":
      return new Response(null, {
        status: 301,
        headers: { Location: new URL(route.location, url).toString() },
      });
    case "asset": {
      const asset = await env.ASSETS.fetch(request);
      return asset.status === 404 ? notFound(url, request, env) : servedAsset(url, asset, env);
    }
    case "asset-or-creator": {
      const asset = await env.ASSETS.fetch(request);
      if (asset.status !== 404) return servedAsset(url, asset, env);
      return (await creatorExists(route.handle, url, env))
        ? negotiatedCreator(url, route.handle, markdown, env)
        : notFound(url, request, env);
    }
  }
}

function assetAt(url: URL, path: string, env: PageEnvironment): Promise<Response> {
  return env.ASSETS.fetch(new Request(new URL(path, url)));
}

async function negotiatedAsset(
  url: URL,
  route: Extract<PageRoute, { kind: "static" }>,
  markdown: boolean,
  env: PageEnvironment,
): Promise<Response> {
  if (!markdown) return negotiated(await assetAt(url, route.html, env), env);
  const source = await assetAt(url, route.markdown, env);
  return markdownResponse(await source.text(), env);
}

async function negotiatedCreator(
  url: URL,
  handle: string,
  markdown: boolean,
  env: PageEnvironment,
): Promise<Response> {
  if (!markdown) return negotiated(await assetAt(url, "/", env), env);
  const origin = publicOrigin(env);
  return markdownResponse(
    `# ${handle} on ytsearch

Search this creator's transcripts:

\`\`\`sh
curl '${origin}/api/search?q=YOUR+WORDS&youtuber=${encodeURIComponent(handle)}'
\`\`\`

See the [API docs](${origin}/docs).
`,
    env,
  );
}

async function shareResponse(
  moment: SharedMoment,
  markdown: boolean,
  card: LinkCard,
  env: PageEnvironment,
  fetchSearch: (path: string, signal: AbortSignal) => Promise<Response>,
): Promise<Response> {
  const video = await lookUpSharedVideo(moment, fetchSearch);
  const canonical = `${publicOrigin(env)}${sharedMomentPath(moment)}`;
  if (markdown) return markdownResponse(renderShareMarkdown(moment, video), env);
  return new Response(renderSharePage(moment, video, canonical, card), {
    headers: {
      ...SECURITY_HEADERS,
      "Content-Security-Policy": SHARE_PAGE_CSP,
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "public, max-age=300",
      Vary: "Accept, User-Agent",
      Link: AGENT_LINKS,
    },
  });
}

async function embedResponse(
  moment: SharedMoment,
  env: PageEnvironment,
  fetchSearch: (path: string, signal: AbortSignal) => Promise<Response>,
): Promise<Response> {
  const video = await lookUpSharedVideo(moment, fetchSearch);
  const canonical = `${publicOrigin(env)}${sharedMomentPath(moment)}`;
  return new Response(renderEmbedPage(moment, video, canonical), {
    headers: {
      ...EMBED_SECURITY_HEADERS,
      "Content-Security-Policy": EMBED_PAGE_CSP,
      "Content-Type": "text/html; charset=utf-8",
      "Cache-Control": "public, max-age=300",
    },
  });
}

// The page must still open when search is slow or down: a link is then a video
// at a timestamp with no quote, never an error and never invented text.
async function lookUpSharedVideo(
  moment: SharedMoment,
  fetchSearch: (path: string, signal: AbortSignal) => Promise<Response>,
): Promise<SharedVideo | undefined> {
  const json = async (path: string): Promise<unknown> => {
    try {
      const response = await fetchSearch(path, AbortSignal.timeout(SHARE_LOOKUP_TIMEOUT_MS));
      return response.ok ? await response.json() : undefined;
    } catch {
      return undefined;
    }
  };
  const matches = () => {
    const params = new URLSearchParams({
      q: moment.query ?? "",
      video_id: moment.videoId,
      per_page: "100",
    });
    if (moment.hybrid) params.set("mode", "hybrid");
    return json(`/api/search?${params.toString()}`);
  };
  const window = clipWindow(moment);
  const [matched, transcript] = await Promise.all([
    moment.query ? matches() : undefined,
    json(
      `/api/videos/${moment.videoId}/transcript?start_ms=${window.startMs}&end_ms=${window.endMs}`,
    ),
  ]);
  return sharedVideoFromSearch(matched, moment, transcript);
}

// A wrong 404 would hide a real creator page, so any doubt serves the app.
async function creatorExists(handle: string, url: URL, env: PageEnvironment): Promise<boolean> {
  const key = new Request(new URL("/__catalog-handles", url));
  try {
    const cached = await caches.default.match(key);
    const handles = cached ? await cached.json<string[]>() : await fetchCatalogHandles(key, env);
    return handles.includes(handle.toLowerCase());
  } catch {
    return true;
  }
}

async function fetchCatalogHandles(key: Request, env: PageEnvironment): Promise<string[]> {
  const response = await fetch(new URL("/api/youtubers", env.API_BASE_URL), {
    signal: AbortSignal.timeout(CATALOG_LOOKUP_TIMEOUT_MS),
  });
  if (!response.ok) throw new Error(`catalog answered ${response.status}`);
  const body = await response.json<{ channels: { handle: string }[] }>();
  const handles = body.channels.map((channel) => channel.handle.toLowerCase());
  await caches.default.put(
    key,
    Response.json(handles, {
      headers: { "Cache-Control": `public, max-age=${CATALOG_CACHE_SECONDS}` },
    }),
  );
  return handles;
}

function notFound(url: URL, request: Request, env: PageEnvironment): Promise<Response> | Response {
  const origin = publicOrigin(env);
  const headers = { "Cache-Control": "public, max-age=60", Vary: "Accept" };
  if (
    prefersHtml(request.headers.get("Accept")) &&
    !prefersMarkdown(request.headers.get("Accept"))
  ) {
    return htmlNotFound(url, env, headers);
  }
  return new Response(
    `# Not found

Nothing lives at this address. Try one of these:

- [Search transcripts](${origin}/)
- [API docs](${origin}/docs)
- [Instructions for agents](${origin}/llms.txt)
- [Sitemap](${origin}/sitemap.xml)
`,
    { status: 404, headers: { ...headers, "Content-Type": MARKDOWN } },
  );
}

async function htmlNotFound(
  url: URL,
  env: PageEnvironment,
  headers: Record<string, string>,
): Promise<Response> {
  const page = await assetAt(url, "/404", env);
  return new Response(page.ok ? await page.text() : "Not found", {
    status: 404,
    headers: { ...SECURITY_HEADERS, ...headers, "Content-Type": "text/html; charset=utf-8" },
  });
}

function publicOrigin(env: PageEnvironment): string {
  return (env.PUBLIC_APP_ORIGIN ?? CANONICAL_ORIGIN).replace(/\/$/, "");
}

function rewriteOrigin(text: string, env: PageEnvironment): string {
  const origin = publicOrigin(env);
  return origin === CANONICAL_ORIGIN ? text : text.replaceAll(CANONICAL_ORIGIN, origin);
}

async function negotiated(asset: Response, env: PageEnvironment): Promise<Response> {
  const response = new Response(rewriteOrigin(await asset.text(), env), asset);
  response.headers.set("Vary", "Accept");
  response.headers.set("Link", AGENT_LINKS);
  response.headers.delete("ETag");
  return response;
}

function markdownResponse(text: string, env: PageEnvironment): Response {
  return new Response(rewriteOrigin(text, env), {
    headers: {
      "Content-Type": MARKDOWN,
      "Cache-Control": "public, max-age=300",
      Vary: "Accept",
      Link: AGENT_LINKS,
    },
  });
}

// Discovery files name absolute URLs, so a test deployment must rewrite them to
// point at itself. Bundles and images never contain the origin.
const TEXT_ASSET = /\.(?:md|txt|xml|json)$|^\/\.well-known\//;

async function servedAsset(url: URL, asset: Response, env: PageEnvironment): Promise<Response> {
  const isMarkdown = url.pathname.endsWith(".md");
  const isCatalog = url.pathname === "/.well-known/api-catalog";
  if (!asset.ok || !TEXT_ASSET.test(url.pathname)) return asset;
  const response = new Response(rewriteOrigin(await asset.text(), env), asset);
  if (isMarkdown) response.headers.set("Content-Type", MARKDOWN);
  if (isCatalog) response.headers.set("Content-Type", "application/linkset+json");
  response.headers.delete("ETag");
  return response;
}
