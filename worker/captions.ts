/**
 * Builds the words a clip shows. The transcript route gives every chunk with exact
 * times; a search hit adds its neighbours' text with estimated times, which is what
 * remains if that route is unavailable.
 */
export type CaptionLine = {
  readonly startMs: number;
  readonly endMs: number;
  readonly text: string;
  readonly matched: boolean;
};

type Chunk = {
  readonly startMs: number;
  readonly endMs: number;
  readonly text: string;
  readonly exact: boolean;
};

export type ClipWindow = { readonly startMs: number; readonly endMs: number };

const WORDS_PER_LINE = 8;
const FALLBACK_CHARS_PER_MS = 0.015;
// Too common to be what someone searched for; marking them would decorate every line.
const UNMARKED_WORDS: ReadonlySet<string> = new Set(
  "the and for you are but not was did why how what who that this with from have they them when where which will would could should about into your his her its our".split(
    " ",
  ),
);

const normalize = (text: string): string => text.replace(/\s+/g, " ").trim();

function chunksFromHit(hit: Record<string, unknown>): Chunk[] {
  if (hit.match_kind !== "transcript" || typeof hit.t_start_ms !== "number") return [];
  if (typeof hit.text !== "string" || !hit.text.trim()) return [];
  const text = normalize(hit.text);
  const startMs = hit.t_start_ms;
  const declaredEnd = typeof hit.t_end_ms === "number" ? hit.t_end_ms : 0;
  const endMs = declaredEnd > startMs ? declaredEnd : startMs + text.length / FALLBACK_CHARS_PER_MS;
  const charsPerMs = text.length / (endMs - startMs);
  const chunks: Chunk[] = [{ startMs, endMs, text, exact: true }];
  if (typeof hit.prev_text === "string" && hit.prev_text.trim()) {
    const before = normalize(hit.prev_text);
    chunks.push({
      startMs: Math.max(0, startMs - before.length / charsPerMs),
      endMs: startMs,
      text: before,
      exact: false,
    });
  }
  if (typeof hit.next_text === "string" && hit.next_text.trim()) {
    const after = normalize(hit.next_text);
    chunks.push({
      startMs: endMs,
      endMs: endMs + after.length / charsPerMs,
      text: after,
      exact: false,
    });
  }
  return chunks;
}

/** Terms worth marking in a line, from the words someone searched for. */
export function markableTerms(query: string | undefined): ReadonlySet<string> {
  const words = (query ?? "").toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? [];
  return new Set(words.filter((word) => word.length >= 3 && !UNMARKED_WORDS.has(word)));
}

const hasTerm = (text: string, terms: ReadonlySet<string>): boolean =>
  (text.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? []).some((word) => terms.has(word));

/** Splits a chunk into caption-sized lines, timed by each word's place in the chunk. */
function linesOfChunk(chunk: Chunk, terms: ReadonlySet<string>): CaptionLine[] {
  const words = chunk.text.split(" ");
  const count = Math.ceil(words.length / WORDS_PER_LINE);
  const perLine = Math.ceil(words.length / count);
  const msPerWord = (chunk.endMs - chunk.startMs) / words.length;
  const lines: CaptionLine[] = [];
  for (let first = 0; first < words.length; first += perLine) {
    const text = words.slice(first, first + perLine).join(" ");
    lines.push({
      startMs: Math.round(chunk.startMs + first * msPerWord),
      endMs: Math.round(chunk.startMs + Math.min(first + perLine, words.length) * msPerWord),
      text,
      matched: hasTerm(text, terms),
    });
  }
  return lines;
}

/** The transcript lines overlapping a window, in order. Exact times beat estimated ones. */
export function captionLines(
  hits: readonly Record<string, unknown>[],
  window: ClipWindow,
  terms: ReadonlySet<string>,
): CaptionLine[] {
  const byText = new Map<string, Chunk>();
  for (const chunk of hits.flatMap(chunksFromHit)) {
    const known = byText.get(chunk.text);
    if (!known || (chunk.exact && !known.exact)) byText.set(chunk.text, chunk);
  }
  return [...byText.values()]
    .sort((left, right) => left.startMs - right.startMs)
    .flatMap((chunk) => linesOfChunk(chunk, terms))
    .filter((line) => line.startMs < window.endMs && line.endMs > window.startMs);
}
