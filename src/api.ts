import type {
  IndexQueueRequest,
  IndexStatus,
  IndexedChannel,
  SearchRequest,
  SearchResponse,
} from "./types";

const DEFAULT_PER_PAGE = 12;
export const API_BASE_URL = (import.meta.env.VITE_API_BASE_URL ?? "").replace(/\/$/, "");
const INDEX_QUEUE_API_BASE_URL = (
  import.meta.env.VITE_INDEX_QUEUE_API_BASE_URL ?? API_BASE_URL
).replace(/\/$/, "");
const SEARCH_CACHE_TTL_MS = 60_000;
const searchCache = new Map<string, { expiresAt: number; response: SearchResponse }>();

export async function searchTranscripts(
  request: SearchRequest,
  options: { signal?: AbortSignal } = {},
): Promise<SearchResponse> {
  const params = new URLSearchParams({
    q: request.query,
    per_page: String(DEFAULT_PER_PAGE),
    page: String(request.page),
    sort: request.sort,
  });
  if (request.youtuber) params.set("youtuber", request.youtuber);

  const url = `${API_BASE_URL}/api/search?${params.toString()}`;
  const cached = searchCache.get(url);
  if (cached && cached.expiresAt > Date.now()) return cached.response;

  const response = await fetch(url, {
    signal: options.signal,
  });
  if (!response.ok) throw await apiError(response, "Search failed");
  const body = (await response.json()) as SearchResponse;
  searchCache.set(url, { response: body, expiresAt: Date.now() + SEARCH_CACHE_TTL_MS });
  return body;
}

export async function indexedChannels(): Promise<IndexedChannel[]> {
  const response = await fetch(`${API_BASE_URL}/api/youtubers`);
  if (!response.ok) throw await apiError(response, "Could not load indexed channels");
  const body = (await response.json()) as { channels: IndexedChannel[] };
  return body.channels;
}

export async function requestIndex(handle: string): Promise<IndexQueueRequest> {
  const response = await fetch(`${INDEX_QUEUE_API_BASE_URL}/api/index-requests`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ handle }),
  });
  if (!response.ok) throw await apiError(response, "Could not queue index request");
  const body = (await response.json()) as { request: IndexQueueRequest };
  return body.request;
}

export async function indexStatus(handle: string): Promise<IndexStatus> {
  const response = await fetch(
    `${INDEX_QUEUE_API_BASE_URL}/api/index-status?handle=${encodeURIComponent(handle)}`,
  );
  if (!response.ok) throw await apiError(response, "Could not load indexing status");
  return (await response.json()) as IndexStatus;
}

async function apiError(response: Response, fallback: string): Promise<Error> {
  const body = (await response.json().catch(() => null)) as {
    error?: string;
  } | null;
  return new Error(body?.error ?? `${fallback} with HTTP ${response.status}`);
}
