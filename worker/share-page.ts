import {
  formatClockTime,
  youtubeEmbedUrl,
  youtubeMomentUrl,
  type SharedMoment,
} from "../shared/share-link";

export type SharedVideo = {
  readonly title: string;
  readonly creatorName: string;
  readonly creatorHandle: string;
  readonly thumbnail: string | undefined;
  /** Real caption lines from the window, oldest first. Empty when none could be loaded. */
  readonly lines: readonly SharedLine[];
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

/** Picks the caption lines inside the window from a video-scoped search response. */
export function sharedVideoFromSearch(
  body: unknown,
  moment: SharedMoment,
): SharedVideo | undefined {
  const videos = (body as { videos?: unknown } | null)?.videos;
  if (!Array.isArray(videos)) return undefined;
  const video = (videos as Record<string, unknown>[]).find(
    (candidate) => candidate.video_id === moment.videoId,
  );
  if (!video || typeof video.title !== "string") return undefined;
  const hits = Array.isArray(video.hits) ? (video.hits as Record<string, unknown>[]) : [];
  const startMs = moment.startSeconds * 1000;
  const endMs = (moment.endSeconds ?? moment.startSeconds + 60) * 1000;
  const lines = hits
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
    title: video.title,
    creatorName: typeof video.youtuber_name === "string" ? video.youtuber_name : "",
    creatorHandle: typeof video.youtuber_handle === "string" ? video.youtuber_handle : "",
    thumbnail: typeof video.thumbnail === "string" ? video.thumbnail : undefined,
    lines,
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

export function renderSharePage(
  moment: SharedMoment,
  video: SharedVideo | undefined,
  canonicalUrl: string,
): string {
  const label = windowLabel(moment);
  const title = video?.title ?? "A shared YouTube moment";
  const quote = video?.lines[0]?.text;
  const description = quote
    ? `“${preview(quote)}”`
    : `Watch ${label} of this video, found with ytsearch.`;
  const pageTitle = `${title} (${label}) · ytsearch`;
  const image =
    video?.thumbnail ??
    `https://i.ytimg.com/vi/${encodeURIComponent(moment.videoId)}/hqdefault.jpg`;
  const creator = video?.creatorName
    ? `<p class="clip-creator">${escapeHtml(video.creatorName)} <span>${escapeHtml(video.creatorHandle)}</span></p>`
    : "";
  const quotes = (video?.lines ?? [])
    .map(
      (line) => `<figure class="quote">
          ${line.before ? `<p class="context">${escapeHtml(line.before)}</p>` : ""}
          <blockquote>${escapeHtml(line.text)}</blockquote>
          ${line.after ? `<p class="context">${escapeHtml(line.after)}</p>` : ""}
          <figcaption>Said at ${formatClockTime(line.startMs / 1000)}</figcaption>
        </figure>`,
    )
    .join("\n        ");
  const result = resultUrl(moment);
  const moreFromCreator = creatorSearchUrl(moment, video);
  const openLabel = `Open on YouTube at ${formatClockTime(moment.startSeconds)}`;
  // Declared exactly as YouTube's own pages declare it. X needs an approved domain for
  // a player card, so this page declares the large image card it can always show; the
  // crawlers that play video are sent to YouTube in pages.ts.
  const playerEmbed = `https://www.youtube.com/embed/${moment.videoId}?start=${moment.startSeconds}`;
  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="theme-color" content="#ffc799" />
    <title>${escapeHtml(pageTitle)}</title>
    <meta name="description" content="${escapeHtml(description)}" />
    <link rel="canonical" href="${escapeHtml(canonicalUrl)}" />
    <link rel="icon" href="/favicon.png" type="image/png" />
    <meta property="og:type" content="video.other" />
    <meta property="og:site_name" content="ytsearch" />
    <meta property="og:title" content="${escapeHtml(`${title} (${label})`)}" />
    <meta property="og:description" content="${escapeHtml(description)}" />
    <meta property="og:url" content="${escapeHtml(canonicalUrl)}" />
    <meta property="og:image" content="${escapeHtml(image)}" />
    <meta property="og:video" content="${escapeHtml(playerEmbed)}" />
    <meta property="og:video:url" content="${escapeHtml(playerEmbed)}" />
    <meta property="og:video:secure_url" content="${escapeHtml(playerEmbed)}" />
    <meta property="og:video:type" content="text/html" />
    <meta property="og:video:width" content="1280" />
    <meta property="og:video:height" content="720" />
    <meta name="twitter:card" content="summary_large_image" />
    <meta name="twitter:title" content="${escapeHtml(`${title} (${label})`)}" />
    <meta name="twitter:description" content="${escapeHtml(description)}" />
    <meta name="twitter:image" content="${escapeHtml(image)}" />
    <link rel="stylesheet" href="/pages.css" />
  </head>
  <body class="clip-page">
    <main>
      <header class="clip-bar">
        <a class="clip-brand" href="/">← ytsearch</a>
        <a href="/">Search every transcript</a>
      </header>
      <iframe
        id="clip-video"
        class="player"
        src="${escapeHtml(youtubeEmbedUrl(moment))}"
        title="${escapeHtml(`YouTube player for ${title}`)}"
        data-video-id="${escapeHtml(moment.videoId)}"
        data-start="${moment.startSeconds}"
        data-end="${moment.endSeconds ?? moment.startSeconds + DEFAULT_CLIP_SECONDS}"
        allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
        referrerpolicy="strict-origin-when-cross-origin"
        allowfullscreen
      ></iframe>
      <div id="clip-controls" class="clip-controls" hidden>
        <button id="clip-toggle" type="button" class="clip-toggle">Play clip</button>
        <input id="clip-seek" class="clip-seek" type="range" min="0" step="0.1" value="0" aria-label="Position in this clip" />
        <output id="clip-clock" class="clip-clock" for="clip-seek"></output>
      </div>
      <div class="clip-body">
        <p class="clip-time">${escapeHtml(label)}</p>
        <h1>${escapeHtml(title)}</h1>
        ${creator}
        ${quotes}
        <p class="actions">
          <a class="action-primary" href="${escapeHtml(youtubeMomentUrl(moment))}">${escapeHtml(openLabel)}</a>
          ${result ? `<a class="action-secondary" href="${escapeHtml(result)}">See this result in ytsearch</a>` : ""}
          ${moreFromCreator ? `<a class="action-secondary" href="${escapeHtml(moreFromCreator.href)}">${escapeHtml(moreFromCreator.label)}</a>` : ""}
        </p>
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
