import { ApiResponseError, ApiTransportError, type ApiError } from "./api-error";
import {
  fetchBootstrapChannels,
  fetchBootstrapIndexStatus,
  requestWireJson,
} from "./bootstrap-api";
import type {
  DashboardData,
  IndexQueueRequest,
  SearchHit,
  SearchRequest,
  SearchResponse,
  SearchVideoResult,
} from "./types";

const DEFAULT_PER_PAGE = 12;
const RETRY_DELAY_MS = 250;

export async function runSearchTranscripts(
  apiBaseUrl: string,
  request: SearchRequest,
  signal?: AbortSignal,
): Promise<SearchResponse> {
  const params = new URLSearchParams({
    q: request.query,
    per_page: String(DEFAULT_PER_PAGE),
    page: String(request.page),
    sort: request.sort,
  });
  if (request.youtuber) params.set("youtuber", request.youtuber);
  const attempt = () =>
    requestWireJson(
      `${apiBaseUrl}/api/search?${params.toString()}`,
      "Search failed",
      "Search returned an invalid response",
      decodeSearchResponse,
      signal,
      { cache: "no-store" },
    );
  try {
    return await attempt();
  } catch (error) {
    if (!isRetryableSearchError(error)) throw error;
    await abortableDelay(RETRY_DELAY_MS, signal);
    return attempt();
  }
}

export function runSearchVideoMoments(
  apiBaseUrl: string,
  query: string,
  videoId: string,
  signal?: AbortSignal,
): Promise<SearchResponse> {
  const params = new URLSearchParams({ q: query, video_id: videoId, per_page: "100" });
  return requestWireJson(
    `${apiBaseUrl}/api/search?${params.toString()}`,
    "Could not load every matching moment",
    "Moment search returned an invalid response",
    decodeSearchResponse,
    signal,
    { cache: "no-store" },
  );
}

export function runRequestIndex(
  apiBaseUrl: string,
  handle: string,
  signal?: AbortSignal,
): Promise<IndexQueueRequest> {
  return requestWireJson(
    `${apiBaseUrl}/api/index-requests`,
    "Could not queue index request",
    "Queue returned an invalid response",
    (input) => decodeIndexQueueResponse(input),
    signal,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ handle }),
    },
  );
}

export async function runDashboardData(
  apiBaseUrl: string,
  signal?: AbortSignal,
): Promise<DashboardData> {
  const [health, channels, status] = await Promise.all([
    settledTimed(() =>
      requestWireJson(
        `${apiBaseUrl}/api/health`,
        "Health check failed",
        "Health check returned an invalid response",
        (input) => boolean(record(input).ok),
        signal,
        { cache: "no-store" },
      ),
    ),
    settledTimed(() => fetchBootstrapChannels(apiBaseUrl, signal)),
    settledTimed(() => fetchBootstrapIndexStatus(apiBaseUrl, undefined, signal)),
  ]);
  const errors = [health, channels, status].flatMap((result) =>
    result.ok ? [] : [result.error instanceof Error ? result.error.message : String(result.error)],
  );
  return {
    channels: channels.ok ? channels.value : [],
    channelsAvailable: channels.ok,
    indexStatus: status.ok ? status.value : undefined,
    apiHealthy: health.ok && health.value,
    fetchedAt: new Date().toISOString(),
    latencyMs: {
      health: health.ok ? health.latencyMs : undefined,
      channels: channels.ok ? channels.latencyMs : undefined,
      indexStatus: status.ok ? status.latencyMs : undefined,
    },
    errors,
  };
}

export async function runCopyToClipboard(value: string): Promise<void> {
  // The Clipboard API is absent outside secure contexts.
  if ("clipboard" in navigator) return navigator.clipboard.writeText(value);
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.append(textarea);
  try {
    textarea.select();
    if (!document.execCommand("copy")) throw new Error("Clipboard copy was rejected");
  } finally {
    textarea.remove();
  }
}

function decodeSearchResponse(input: unknown): SearchResponse {
  const value = record(input);
  return {
    query: string(value.query),
    total_moments: number(value.total_moments),
    total_videos: number(value.total_videos),
    totals_approximate:
      value.totals_approximate === undefined ? false : boolean(value.totals_approximate),
    total_found: number(value.total_found),
    out_of: number(value.out_of),
    time_to_first_result_ms: number(value.time_to_first_result_ms),
    edge_cached: value.edge_cached === true,
    page: number(value.page),
    per_page: number(value.per_page),
    sort: literal(value.sort, [
      "relevance",
      "relevance_engagement",
      "views_desc",
      "likes_desc",
      "comments_desc",
      "published_desc",
      "published_asc",
    ]),
    mode: literal(value.mode, ["lexical"]),
    has_next_page: boolean(value.has_next_page),
    hits: array(value.hits, decodeSearchHit),
    videos: array(value.videos, decodeSearchVideo),
  };
}

function decodeSearchVideo(input: unknown): SearchVideoResult {
  const value = record(input);
  return {
    video_id: string(value.video_id),
    title: string(value.title),
    title_highlight: optionalString(value.title_highlight),
    youtuber_handle: string(value.youtuber_handle),
    youtuber_name: string(value.youtuber_name),
    thumbnail: optionalString(value.thumbnail),
    published_at: optionalString(value.published_at),
    view_count: optionalNumber(value.view_count),
    like_count: optionalNumber(value.like_count),
    comment_count: optionalNumber(value.comment_count),
    content_kind: literal(value.content_kind, ["video", "short", "unknown"]),
    match_count: number(value.match_count),
    youtube_url: string(value.youtube_url),
    embed_url: string(value.embed_url),
    best_timestamp: optionalString(value.best_timestamp),
    hits: array(value.hits, decodeSearchHit),
  };
}

function decodeSearchHit(input: unknown): SearchHit {
  const value = record(input);
  return {
    id: string(value.id),
    video_id: string(value.video_id),
    title: string(value.title),
    text: string(value.text),
    prev_text: optionalNullableString(value.prev_text),
    next_text: optionalNullableString(value.next_text),
    text_highlight: optionalString(value.text_highlight),
    title_highlight: optionalString(value.title_highlight),
    match_kind: literal(value.match_kind, ["transcript", "title"]),
    youtuber_handle: string(value.youtuber_handle),
    youtuber_name: string(value.youtuber_name),
    thumbnail: optionalString(value.thumbnail),
    published_at: optionalString(value.published_at),
    view_count: optionalNumber(value.view_count),
    like_count: optionalNumber(value.like_count),
    comment_count: optionalNumber(value.comment_count),
    content_kind: literal(value.content_kind, ["video", "short", "unknown"]),
    t_start_ms: number(value.t_start_ms),
    timestamp: optionalString(value.timestamp),
    youtube_url: string(value.youtube_url),
    embed_url: string(value.embed_url),
  };
}

function decodeIndexQueueResponse(input: unknown): IndexQueueRequest {
  const value = record(record(input).request);
  return {
    id: string(value.id),
    handle: string(value.handle),
    note: optionalString(value.note),
    requested_at: number(value.requested_at),
    source_backend: string(value.source_backend),
    status: literal(value.status, ["pending", "queued", "running", "failed", "rejected", "idle"]),
  };
}

function isRetryableSearchError(error: unknown): error is ApiError {
  if (error instanceof ApiTransportError) return true;
  return (
    error instanceof ApiResponseError &&
    [502, 503, 504].includes(error.status) &&
    error.code !== "scheduled_offline" &&
    error.code !== "search_starting"
  );
}

function abortableDelay(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
      return;
    }
    const timeout = window.setTimeout(finish, milliseconds);
    const abort = () => {
      window.clearTimeout(timeout);
      reject(signal?.reason ?? new DOMException("Aborted", "AbortError"));
    };
    function finish() {
      signal?.removeEventListener("abort", abort);
      resolve();
    }
    signal?.addEventListener("abort", abort, { once: true });
  });
}

async function settledTimed<T>(
  run: () => Promise<T>,
): Promise<
  { ok: true; value: T; latencyMs: number } | { ok: false; error: unknown; latencyMs: number }
> {
  const started = performance.now();
  try {
    return { ok: true, value: await run(), latencyMs: Math.round(performance.now() - started) };
  } catch (error) {
    return { ok: false, error, latencyMs: Math.round(performance.now() - started) };
  }
}

type JsonRecord = Record<string, unknown>;

function record(input: unknown): JsonRecord {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    throw new TypeError("Expected an object");
  }
  return input as JsonRecord;
}

function string(input: unknown): string {
  if (typeof input !== "string") throw new TypeError("Expected a string");
  return input;
}

// The API serializes absent optional fields as JSON null (Rust `Option`
// without `skip_serializing_if`), so null and undefined both mean absent.
function optionalString(input: unknown): string | undefined {
  return input === undefined || input === null ? undefined : string(input);
}

function optionalNullableString(input: unknown): string | null | undefined {
  return input === undefined || input === null ? input : string(input);
}

function number(input: unknown): number {
  if (typeof input !== "number" || !Number.isFinite(input))
    throw new TypeError("Expected a number");
  return input;
}

function optionalNumber(input: unknown): number | undefined {
  return input === undefined ? undefined : number(input);
}

function boolean(input: unknown): boolean {
  if (typeof input !== "boolean") throw new TypeError("Expected a boolean");
  return input;
}

function array<T>(input: unknown, decode: (value: unknown) => T): T[] {
  if (!Array.isArray(input)) throw new TypeError("Expected an array");
  return input.map(decode);
}

function literal<const T extends string>(input: unknown, values: readonly T[]): T {
  if (typeof input !== "string" || !values.includes(input as T)) {
    throw new TypeError("Expected a known literal");
  }
  return input as T;
}
