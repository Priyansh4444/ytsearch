import { captionLines, markableTerms, type CaptionLine } from "./captions";
import {
  embeddedMomentPath,
  formatClockTime,
  youtubeEmbedUrl,
  youtubeMomentUrl,
  type SharedMoment,
} from "../shared/share-link";

const PLAY_ICON =
  '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5.5v13l11-6.5z" fill="currentColor"/></svg>';
const PAUSE_ICON =
  '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z" fill="currentColor"/></svg>';
const REPLAY_ICON =
  '<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 5v5h5"/><path d="M4.6 10a8 8 0 1 1-.4 4.5"/></svg>';
const SOUND_ICON =
  '<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 10v4h3l5 4V6L7 10z" fill="currentColor"/><path d="M16 9a4 4 0 0 1 0 6"/></svg>';
const MUTED_ICON =
  '<svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 10v4h3l5 4V6L7 10z" fill="currentColor"/><path d="m16 10 4 4M20 10l-4 4"/></svg>';

export type SharedVideo = {
  readonly title: string;
  readonly creatorName: string;
  readonly creatorHandle: string;
  readonly thumbnail: string | undefined;
  readonly publishedAt: string | undefined;
  /** The hits inside the window that matched the search, oldest first. */
  readonly lines: readonly SharedLine[];
  /** Every transcript line known for the window, for reading along. */
  readonly captions: readonly CaptionLine[];
};

export type SharedLine = {
  readonly text: string;
  readonly before: string | undefined;
  readonly after: string | undefined;
  readonly startMs: number;
};

const QUOTE_PREVIEW_CHARS = 180;
// A link with no end still plays as a clip, for a minute from its start.
const DEFAULT_CLIP_SECONDS = 60;

export function clipWindow(moment: SharedMoment): { startMs: number; endMs: number } {
  return {
    startMs: moment.startSeconds * 1000,
    endMs: (moment.endSeconds ?? moment.startSeconds + DEFAULT_CLIP_SECONDS) * 1000,
  };
}

const hitsOf = (video: Record<string, unknown> | undefined): Record<string, unknown>[] =>
  Array.isArray(video?.hits) ? (video.hits as Record<string, unknown>[]) : [];

/** Hit-shaped lines, with metadata, from `/api/videos/{id}/transcript`. */
function transcriptOf(body: unknown, videoId: string): Record<string, unknown> | undefined {
  const transcript = body as Record<string, unknown> | null | undefined;
  if (transcript?.video_id !== videoId || typeof transcript.title !== "string") return undefined;
  const lines = Array.isArray(transcript.lines) ? (transcript.lines as object[]) : [];
  return { ...transcript, hits: lines.map((line) => ({ ...line, match_kind: "transcript" })) };
}

function videoIn(body: unknown, videoId: string): Record<string, unknown> | undefined {
  const videos = (body as { videos?: unknown } | null)?.videos;
  if (!Array.isArray(videos)) return undefined;
  const video = (videos as Record<string, unknown>[]).find(
    (candidate) => candidate.video_id === videoId,
  );
  return video && typeof video.title === "string" ? video : undefined;
}

/**
 * Builds the page's data from a search for the shared words and, optionally, the
 * window's transcript. Only text that came back from the API is ever shown.
 */
export function sharedVideoFromSearch(
  body: unknown,
  moment: SharedMoment,
  transcriptBody?: unknown,
): SharedVideo | undefined {
  const matched = videoIn(body, moment.videoId);
  const transcript = transcriptOf(transcriptBody, moment.videoId);
  const video = matched ?? transcript;
  if (!video) return undefined;
  const { startMs, endMs } = clipWindow(moment);
  const lines = hitsOf(matched)
    .filter(
      (hit) =>
        hit.match_kind === "transcript" &&
        typeof hit.text === "string" &&
        typeof hit.t_start_ms === "number" &&
        hit.t_start_ms >= startMs &&
        hit.t_start_ms < endMs,
    )
    .map((hit): SharedLine => ({
      text: hit.text as string,
      before: typeof hit.prev_text === "string" ? hit.prev_text : undefined,
      after: typeof hit.next_text === "string" ? hit.next_text : undefined,
      startMs: hit.t_start_ms as number,
    }))
    .sort((left, right) => left.startMs - right.startMs);
  return {
    title: video.title as string,
    creatorName: typeof video.youtuber_name === "string" ? video.youtuber_name : "",
    creatorHandle: typeof video.youtuber_handle === "string" ? video.youtuber_handle : "",
    thumbnail: typeof video.thumbnail === "string" ? video.thumbnail : undefined,
    publishedAt: typeof video.published_at === "string" ? video.published_at : undefined,
    lines,
    captions: captionLines(
      [...hitsOf(matched), ...hitsOf(transcript)],
      clipWindow(moment),
      markableTerms(moment.query),
    ),
  };
}

const escapeHtml = (value: string): string =>
  value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");

const preview = (text: string): string =>
  text.length <= QUOTE_PREVIEW_CHARS
    ? text
    : `${text.slice(0, QUOTE_PREVIEW_CHARS).replace(/\s+\S*$/, "")}…`;

function windowLabel(moment: SharedMoment): string {
  const start = formatClockTime(moment.startSeconds);
  return moment.endSeconds === undefined
    ? `from ${start}`
    : `${start} to ${formatClockTime(moment.endSeconds)}`;
}

function resultUrl(moment: SharedMoment): string | undefined {
  if (!moment.query) return undefined;
  const params = new URLSearchParams({ q: moment.query });
  if (moment.hybrid) params.set("mode", "hybrid");
  return `/?${params.toString()}#video-${encodeURIComponent(moment.videoId)}`;
}

function creatorSearchUrl(
  moment: SharedMoment,
  video: SharedVideo | undefined,
): { href: string; label: string } | undefined {
  if (!moment.query || !video?.creatorHandle) return undefined;
  const params = new URLSearchParams({ q: moment.query, youtuber: video.creatorHandle });
  return { href: `/?${params.toString()}`, label: `More from ${video.creatorHandle}` };
}

// What X asks for, as proven by a player card on a small personal domain
// (tweetcraft.jai.vin): a player page it may frame, sized 480 by 480.
const X_PLAYER_SIZE = 480;

type CardText = {
  readonly label: string;
  readonly title: string;
  readonly description: string;
  readonly image: string;
};

function cardText(moment: SharedMoment, video: SharedVideo | undefined): CardText {
  const label = windowLabel(moment);
  const quote = video?.lines[0]?.text ?? video?.captions[0]?.text;
  return {
    label,
    title: video?.title ?? "A shared YouTube moment",
    description: quote
      ? `“${preview(quote)}”`
      : `Watch ${label} of this video, found with ytsearch.`,
    image:
      video?.thumbnail ??
      `https://i.ytimg.com/vi/${encodeURIComponent(moment.videoId)}/hqdefault.jpg`,
  };
}

/** What a link preview may do: play our player, as X does, or show a picture. */
export type LinkCard = "player" | "picture";

function cardTags(
  moment: SharedMoment,
  video: SharedVideo | undefined,
  canonicalUrl: string,
  card: LinkCard,
): string {
  const { label, title, description, image } = cardText(moment, video);
  const cardTitle = escapeHtml(`${title} (${label})`);
  const safeDescription = escapeHtml(description);
  const safeImage = escapeHtml(image);
  const player = `${new URL(canonicalUrl).origin}${embeddedMomentPath(moment)}`;
  const shared = `<meta property="og:type" content="video.other" />
    <meta property="og:site_name" content="ytsearch" />
    <meta property="og:title" content="${cardTitle}" />
    <meta property="og:description" content="${safeDescription}" />
    <meta property="og:url" content="${escapeHtml(canonicalUrl)}" />
    <meta property="og:image" content="${safeImage}" />
    <meta name="twitter:title" content="${cardTitle}" />
    <meta name="twitter:description" content="${safeDescription}" />
    <meta name="twitter:image" content="${safeImage}" />`;
  if (card === "picture") {
    return `${shared}
    <meta name="twitter:card" content="summary_large_image" />`;
  }
  const videoObject = {
    "@context": "https://schema.org",
    "@type": "VideoObject",
    name: `${title} (${label})`,
    description,
    thumbnailUrl: image,
    embedUrl: player,
    ...(video?.publishedAt ? { uploadDate: video.publishedAt } : {}),
  };
  return `${shared}
    <meta name="twitter:card" content="player" />
    <meta name="twitter:player" content="${escapeHtml(player)}" />
    <meta name="twitter:player:width" content="${X_PLAYER_SIZE}" />
    <meta name="twitter:player:height" content="${X_PLAYER_SIZE}" />
    <script type="application/ld+json">${JSON.stringify(videoObject).replaceAll("<", "\\u003c")}</script>`;
}

function markedText(text: string, terms: ReadonlySet<string>): string {
  return text
    .split(/([\p{L}\p{N}]+)/u)
    .map((part) =>
      terms.has(part.toLowerCase()) ? `<mark>${escapeHtml(part)}</mark>` : escapeHtml(part),
    )
    .join("");
}

function captionList(moment: SharedMoment, video: SharedVideo | undefined): string {
  const terms = markableTerms(moment.query);
  const items = (video?.captions ?? [])
    .map(
      (line) =>
        `<li data-start="${line.startMs}"${line.matched ? ' class="is-match"' : ""}><button type="button">${markedText(line.text, terms)}</button></li>`,
    )
    .join("");
  return items ? `<ol id="clip-captions" class="captions">${items}</ol>` : "";
}

function playerFrame(
  moment: SharedMoment,
  title: string,
  image: string,
  autoplay: boolean,
  link: { readonly href: string; readonly label: string },
): string {
  return `<div class="clip-video">
        <div id="clip-stage" class="clip-stage">
          <button id="clip-poster" class="clip-poster" type="button" aria-label="Play clip" hidden>
            <img src="${escapeHtml(image)}" alt="" />
            <span class="clip-poster-glyph"><span class="icon-play">${PLAY_ICON}</span><span class="icon-replay">${REPLAY_ICON}</span></span>
          </button>
          <iframe
            id="clip-video"
            class="player"
            src="${escapeHtml(youtubeEmbedUrl(moment))}"
            title="${escapeHtml(`YouTube player for ${title}`)}"
            data-video-id="${escapeHtml(moment.videoId)}"
            data-start="${moment.startSeconds}"
            data-end="${moment.endSeconds ?? moment.startSeconds + DEFAULT_CLIP_SECONDS}"
            ${autoplay ? "data-autoplay" : ""}
            allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
            referrerpolicy="strict-origin-when-cross-origin"
            allowfullscreen
          ></iframe>
        </div>
        <div id="clip-controls" class="clip-controls" hidden>
          <div class="clip-timeline">
            <span id="clip-fill" class="clip-fill"></span>
            <input id="clip-seek" class="clip-seek" type="range" min="0" step="0.05" value="0" aria-label="Position in this clip" />
          </div>
          <div class="clip-row">
            <button id="clip-toggle" class="clip-icon" type="button" aria-label="Play clip"><span class="icon-play">${PLAY_ICON}</span><span class="icon-pause">${PAUSE_ICON}</span><span class="icon-replay">${REPLAY_ICON}</span></button>
            <button id="clip-mute" class="clip-icon" type="button" aria-label="Mute"><span class="icon-sound">${SOUND_ICON}</span><span class="icon-muted">${MUTED_ICON}</span></button>
            <output id="clip-clock" class="clip-clock" for="clip-seek"></output>
            <a class="clip-row-link" href="${escapeHtml(link.href)}" target="_blank" rel="noreferrer">${escapeHtml(link.label)}</a>
          </div>
        </div>
      </div>`;
}

/** The compact player X shows inside a post: the clip, the words, and whose they are. */
export function renderEmbedPage(
  moment: SharedMoment,
  video: SharedVideo | undefined,
  canonicalUrl: string,
): string {
  const { label, title, image } = cardText(moment, video);
  const creator = video?.creatorName ? `${escapeHtml(video.creatorName)} ` : "";
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="robots" content="noindex" />
    <title>${escapeHtml(`${title} (${label}) · ytsearch`)}</title>
    <link rel="canonical" href="${escapeHtml(canonicalUrl)}" />
    ${cardTags(moment, video, canonicalUrl, "player")}
    <link rel="stylesheet" href="/pages.css" />
  </head>
  <body class="clip-embed">
    ${playerFrame(moment, title, image, true, { href: canonicalUrl, label: "Open in ytsearch" })}
    ${captionList(moment, video)}
    <footer class="embed-about">
      <h1>${escapeHtml(title)}</h1>
      <p>${creator}<span>${escapeHtml(label)}</span></p>
    </footer>
    <script src="/clip-player.js" defer></script>
  </body>
</html>
`;
}

export function renderSharePage(
  moment: SharedMoment,
  video: SharedVideo | undefined,
  canonicalUrl: string,
  card: LinkCard = "player",
): string {
  const { label, title, description, image } = cardText(moment, video);
  const pageTitle = `${title} (${label}) · ytsearch`;
  const creator = video?.creatorName
    ? `<p class="clip-creator">${escapeHtml(video.creatorName)} <span>${escapeHtml(video.creatorHandle)}</span></p>`
    : "";
  const result = resultUrl(moment);
  const moreFromCreator = creatorSearchUrl(moment, video);
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="theme-color" content="#050505" />
    <title>${escapeHtml(pageTitle)}</title>
    <meta name="description" content="${escapeHtml(description)}" />
    <link rel="canonical" href="${escapeHtml(canonicalUrl)}" />
    <link rel="icon" href="/favicon.png" type="image/png" />
    ${cardTags(moment, video, canonicalUrl, card)}
    <link rel="stylesheet" href="/pages.css" />
  </head>
  <body class="clip-page">
    <main>
      <header class="clip-bar">
        <a class="clip-brand" href="/">← ytsearch</a>
        <a href="/">Search every transcript</a>
      </header>
      <div class="clip-layout">
        ${playerFrame(moment, title, image, false, { href: youtubeMomentUrl(moment), label: "Open on YouTube" })}
        <aside class="clip-about">
          <h1>${escapeHtml(title)}</h1>
          ${creator}
          <p class="clip-time">${escapeHtml(label)}</p>
          <nav class="clip-links" aria-label="More from this moment">
            ${result ? `<a href="${escapeHtml(result)}">See this result in ytsearch</a>` : ""}
            ${moreFromCreator ? `<a href="${escapeHtml(moreFromCreator.href)}">${escapeHtml(moreFromCreator.label)}</a>` : ""}
          </nav>
        </aside>
        ${captionList(moment, video)}
      </div>
    </main>
    <script src="/clip-player.js" defer></script>
  </body>
</html>
`;
}

export function renderShareMarkdown(moment: SharedMoment, video: SharedVideo | undefined): string {
  const lines = (video?.lines ?? []).map(
    (line) => `> ${line.text.replace(/\s+/g, " ")}\n\n(at ${formatClockTime(line.startMs / 1000)})`,
  );
  const creator = video?.creatorName ? `\n\nBy ${video.creatorName} (${video.creatorHandle}).` : "";
  return `# ${video?.title ?? "A shared YouTube moment"}${creator}

The shared window runs ${windowLabel(moment)}: [open it on YouTube](${youtubeMomentUrl(moment)}).
${lines.length ? `\n${lines.join("\n\n")}\n` : ""}`;
}
