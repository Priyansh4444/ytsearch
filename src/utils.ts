import { loadClientRuntime } from "./client-runtime-loader";
import { flush } from "solid-js";

type HighlightPart = {
  text: string;
  highlighted: boolean;
};

export function creatorFromPath(): string | null {
  const firstSegment = window.location.pathname.split("/").filter(Boolean)[0]?.trim();
  if (!firstSegment || firstSegment === "api") return null;
  return normalizeHandle(firstSegment);
}

export function normalizeHandle(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "";
  return trimmed.startsWith("@") ? trimmed : `@${trimmed}`;
}

export function routeForHandle(handle: string): string {
  return `/${normalizeHandle(handle).slice(1)}`;
}

export function withViewTransition(update: () => void): void {
  const updateAndFlush = () => {
    update();
    // A view transition snapshots immediately after its callback. Solid 2
    // batches setters, so the new DOM must be made visible in this boundary.
    flush();
  };
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    updateAndFlush();
    return;
  }

  if ("startViewTransition" in document) {
    document.startViewTransition(updateAndFlush);
    return;
  }
  updateAndFlush();
}

export function formatDate(value: string | undefined): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return value;
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export async function copyToClipboard(value: string): Promise<void> {
  const { runCopyToClipboard } = await loadClientRuntime();
  await runCopyToClipboard(value);
}

export function highlightedParts(
  text: string,
  highlight: string | undefined,
  _query: string,
): HighlightPart[] {
  if (highlight) return partsFromTaggedSnippet(highlight);
  // Raw owns tokenization and match provenance. Inventing marks from the raw
  // query here highlighted substrings the backend deliberately rejected, such
  // as `fire` in `wildfire` and `react` in `Preact`.
  return [{ text, highlighted: false }];
}

function partsFromTaggedSnippet(snippet: string): HighlightPart[] {
  const parts: HighlightPart[] = [];
  const regex = /<mark>(.*?)<\/mark>/gis;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(snippet))) {
    if (match.index > lastIndex) {
      parts.push({
        text: decodeEntities(snippet.slice(lastIndex, match.index)),
        highlighted: false,
      });
    }
    parts.push({ text: decodeEntities(match[1] ?? ""), highlighted: true });
    lastIndex = regex.lastIndex;
  }

  if (lastIndex < snippet.length) {
    parts.push({
      text: decodeEntities(snippet.slice(lastIndex)),
      highlighted: false,
    });
  }

  return parts.length > 0 ? parts : [{ text: decodeEntities(snippet), highlighted: false }];
}

// The inverse of the reader's `escape_html`, the only producer of snippets.
// It escapes `&` too, so no other entity can appear in a snippet. Decoding
// with plain string operations avoids an HTML parse per snippet part.
const SNIPPET_ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
};

function decodeEntities(value: string): string {
  if (!value.includes("&")) return value;
  return value.replace(/&(?:amp|lt|gt|quot|#39);/g, (entity) => SNIPPET_ENTITIES[entity] ?? entity);
}
