/** One shareable window of a video. Links stay stateless: the page rebuilds it from real data. */
export type SharedMoment = {
  readonly videoId: string;
  readonly startSeconds: number;
  /** Absent means "from here on". Always after the start and within the limit. */
  readonly endSeconds: number | undefined;
  /** The search that found the moment; it selects the transcript lines to quote. */
  readonly query: string | undefined;
  /** The search was hybrid, so finding the same lines needs a hybrid lookup. */
  readonly hybrid: boolean;
};

const SHARE_PREFIX = "/s/";
const EMBED_PREFIX = "/embed/";
const VIDEO_ID = /^[A-Za-z0-9_-]{6,20}$/;
const WHOLE_SECONDS = /^\d{1,6}$/;
const MAX_QUERY_LENGTH = 200;
export const MAX_SHARED_WINDOW_SECONDS = 600;

export function sharedMomentPath(moment: SharedMoment): string {
  return momentPath(SHARE_PREFIX, moment);
}

/** The compact player that X loads inside a post. */
export function embeddedMomentPath(moment: SharedMoment): string {
  return momentPath(EMBED_PREFIX, moment);
}

function momentPath(prefix: string, moment: SharedMoment): string {
  const params = new URLSearchParams({ t: String(moment.startSeconds) });
  if (moment.endSeconds !== undefined) params.set("end", String(moment.endSeconds));
  if (moment.query) params.set("q", moment.query);
  if (moment.hybrid) params.set("m", "hybrid");
  return `${prefix}${moment.videoId}?${params.toString()}`;
}

/** Undefined when the path is not a share link, so the page can answer a real 404. */
export function parseSharedMoment(
  pathname: string,
  params: URLSearchParams,
): SharedMoment | undefined {
  return parseMoment(SHARE_PREFIX, pathname, params);
}

export function parseEmbeddedMoment(
  pathname: string,
  params: URLSearchParams,
): SharedMoment | undefined {
  return parseMoment(EMBED_PREFIX, pathname, params);
}

function parseMoment(
  prefix: string,
  pathname: string,
  params: URLSearchParams,
): SharedMoment | undefined {
  if (!pathname.startsWith(prefix)) return undefined;
  const videoId = pathname.slice(prefix.length);
  if (!VIDEO_ID.test(videoId)) return undefined;
  const start = wholeSeconds(params.get("t") ?? "0");
  if (start === undefined) return undefined;
  const end = wholeSeconds(params.get("end") ?? "");
  const query = params.get("q")?.trim().slice(0, MAX_QUERY_LENGTH);
  return {
    videoId,
    startSeconds: start,
    endSeconds:
      end !== undefined && end > start && end - start <= MAX_SHARED_WINDOW_SECONDS
        ? end
        : undefined,
    query: query || undefined,
    hybrid: params.get("m") === "hybrid",
  };
}

/** Opens the video on YouTube at the start of the window. */
export function youtubeMomentUrl(moment: SharedMoment): string {
  return `https://www.youtube.com/watch?v=${moment.videoId}&t=${moment.startSeconds}s`;
}

/** YouTube's own player, which stops at the end of the window. */
export function youtubeEmbedUrl(moment: SharedMoment, host = "www.youtube-nocookie.com"): string {
  const params = new URLSearchParams({ start: String(moment.startSeconds), rel: "0" });
  if (moment.endSeconds !== undefined) params.set("end", String(moment.endSeconds));
  return `https://${host}/embed/${moment.videoId}?${params.toString()}`;
}

const X_QUOTE_CHARS = 180;

/** Composes a post quoting the moment, with the page's player card from its link. */
export function xPostUrl(quote: string, pageUrl: string): string {
  const trimmed = quote.replace(/\s+/g, " ").trim();
  const short =
    trimmed.length <= X_QUOTE_CHARS
      ? trimmed
      : `${trimmed.slice(0, X_QUOTE_CHARS).replace(/\s+\S*$/, "")}…`;
  const params = new URLSearchParams({ url: pageUrl });
  if (short) params.set("text", `“${short}”`);
  return `https://x.com/intent/post?${params.toString()}`;
}

function wholeSeconds(value: string): number | undefined {
  return WHOLE_SECONDS.test(value) ? Number(value) : undefined;
}

export function formatClockTime(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = String(seconds % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

/** The window after moving one bound; it stays ordered, at least minSeconds long and within the limit. */
export function movedWindow(
  window: { readonly start: number; readonly end: number },
  bound: "start" | "end",
  delta: number,
  minSeconds: number,
): { readonly start: number; readonly end: number } {
  if (bound === "end") {
    const earliest = window.start + minSeconds;
    const latest = window.start + MAX_SHARED_WINDOW_SECONDS;
    return { start: window.start, end: Math.min(latest, Math.max(earliest, window.end + delta)) };
  }
  const latest = window.end - minSeconds;
  const earliest = Math.max(0, window.end - MAX_SHARED_WINDOW_SECONDS);
  return { start: Math.max(earliest, Math.min(latest, window.start + delta)), end: window.end };
}
