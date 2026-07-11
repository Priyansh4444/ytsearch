export type HighlightPart = {
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
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    update();
    return;
  }

  const documentWithTransitions = document as Document & {
    startViewTransition?: (callback: () => void) => unknown;
  };
  if (documentWithTransitions.startViewTransition) {
    documentWithTransitions.startViewTransition(update);
    return;
  }
  update();
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
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.append(textarea);
  textarea.select();
  document.execCommand("copy");
  textarea.remove();
}

export function highlightedParts(
  text: string,
  highlight: string | undefined,
  query: string,
): HighlightPart[] {
  if (highlight) return partsFromTypesenseSnippet(highlight);
  return partsFromQuery(text, query);
}

function partsFromTypesenseSnippet(snippet: string): HighlightPart[] {
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

function partsFromQuery(text: string, query: string): HighlightPart[] {
  const terms = query
    .toLowerCase()
    .split(/\s+/)
    .map((term) => term.replace(/[^a-z0-9']/gi, ""))
    .filter((term) => term.length >= 2);
  if (terms.length === 0) return [{ text, highlighted: false }];

  const pattern = new RegExp(`(${terms.map(escapeRegex).join("|")})`, "gi");
  return text
    .split(pattern)
    .filter(Boolean)
    .map((part) => ({
      text: part,
      highlighted: terms.includes(part.toLowerCase()),
    }));
}

function escapeRegex(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function decodeEntities(value: string): string {
  const element = document.createElement("textarea");
  element.innerHTML = value;
  return element.value;
}
