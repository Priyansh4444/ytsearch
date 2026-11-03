import { parseSharedMoment, type SharedMoment } from "../shared/share-link";

/** Creators who changed their YouTube handle: old links keep working. */
const RENAMED_CREATORS: ReadonlyMap<string, string> = new Map([["@t3dotgg", "@theo"]]);

export function currentHandle(handle: string): string {
  return RENAMED_CREATORS.get(handle.toLowerCase()) ?? handle;
}

const STATIC_PAGES: ReadonlyMap<string, { html: string; markdown: string }> = new Map([
  ["/", { html: "/", markdown: "/index.md" }],
  ["/about", { html: "/about", markdown: "/about.md" }],
  ["/contact", { html: "/contact", markdown: "/contact.md" }],
  ["/docs", { html: "/docs", markdown: "/docs.md" }],
  ["/privacy", { html: "/privacy", markdown: "/privacy.md" }],
]);

const REDIRECTS: ReadonlyMap<string, string> = new Map([
  ["/developers", "/docs"],
  ["/api-docs", "/docs"],
]);

const CREATOR_PATH = /^\/@?([A-Za-z0-9][A-Za-z0-9._-]{0,63})$/;

export type PageRoute =
  | { readonly kind: "static"; readonly html: string; readonly markdown: string }
  | { readonly kind: "app" }
  | { readonly kind: "share"; readonly moment: SharedMoment }
  | { readonly kind: "redirect"; readonly location: string }
  | { readonly kind: "asset-or-creator"; readonly handle: string }
  | { readonly kind: "asset" };

/** Decides what a non-API path is, without I/O. Unknown paths end in a real 404. */
export function routePage(url: URL): PageRoute {
  const { pathname, search } = url;
  const page = STATIC_PAGES.get(pathname);
  if (page) return { kind: "static", ...page };
  if (pathname === "/dashboard") return { kind: "app" };
  const redirect = REDIRECTS.get(pathname);
  if (redirect) return { kind: "redirect", location: redirect };
  const moment = parseSharedMoment(pathname, url.searchParams);
  if (moment) return { kind: "share", moment };
  const creator = CREATOR_PATH.exec(pathname)?.[1];
  if (!creator) return { kind: "asset" };
  const renamed = RENAMED_CREATORS.get(`@${creator.toLowerCase()}`);
  if (renamed) return { kind: "redirect", location: `/${renamed.slice(1)}${search}` };
  return { kind: "asset-or-creator", handle: `@${creator}` };
}

/** True when a client asked for Markdown at least as much as for HTML. */
export function prefersMarkdown(accept: string | null): boolean {
  if (!accept) return false;
  let markdown = 0;
  let html = 0;
  for (const part of accept.split(",")) {
    const [type = "", ...parameters] = part.split(";").map((piece) => piece.trim().toLowerCase());
    const q = parameters.find((parameter) => parameter.startsWith("q="));
    const weight = q ? Number(q.slice(2)) : 1;
    if (!Number.isFinite(weight)) continue;
    if (type === "text/markdown") markdown = Math.max(markdown, weight);
    if (type === "text/html" || type === "application/xhtml+xml") html = Math.max(html, weight);
  }
  return markdown > 0 && markdown >= html;
}

/**
 * Discord and X only play video from players they trust, and YouTube's is one.
 * Sending just their link crawlers to YouTube, as a shortener would, lets a pasted
 * link play in place; people who click it still get the clip page.
 */
export function wantsYouTubeUnfurl(userAgent: string | null): boolean {
  return /\b(?:Discordbot|Twitterbot)\b/.test(userAgent ?? "");
}

/** True when a client wants a page rather than data, for choosing a 404's format. */
export function prefersHtml(accept: string | null): boolean {
  return (accept ?? "").toLowerCase().includes("text/html");
}
