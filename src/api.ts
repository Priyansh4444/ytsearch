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
const INDEX_STATUS_CACHE_TTL_MS = 5_000;
const searchCache = new Map<string, { expiresAt: number; response: SearchResponse }>();
const indexStatusCache = new Map<string, { expiresAt: number; response: IndexStatus }>();

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

  let response = await fetch(url, { signal: options.signal });
  if (response.status === 502 || response.status === 503 || response.status === 504) {
    await new Promise((resolve) => window.setTimeout(resolve, 250));
    response = await fetch(url, { signal: options.signal });
  }
  if (!response.ok) throw await apiError(response, "Search failed");
  const body = await responseJson<SearchResponse>(response, "Search returned an invalid response");
  searchCache.set(url, { response: body, expiresAt: Date.now() + SEARCH_CACHE_TTL_MS });
  return body;
}

export async function indexedChannels(): Promise<IndexedChannel[]> {
  const response = await fetch(`${API_BASE_URL}/api/youtubers`);
  if (!response.ok) throw await apiError(response, "Could not load indexed channels");
  const body = await responseJson<{ channels: IndexedChannel[] }>(
    response,
    "Channel list returned an invalid response",
  );
  return body.channels;
}

export async function requestIndex(handle: string): Promise<IndexQueueRequest> {
  const response = await fetch(`${INDEX_QUEUE_API_BASE_URL}/api/index-requests`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ handle }),
  });
  if (!response.ok) throw await apiError(response, "Could not queue index request");
  const body = await responseJson<{ request: IndexQueueRequest }>(
    response,
    "Queue returned an invalid response",
  );
  indexStatusCache.clear();
  return body.request;
}

export async function indexStatus(handle?: string): Promise<IndexStatus> {
  const params = handle ? `?handle=${encodeURIComponent(handle)}` : "";
  const url = `${INDEX_QUEUE_API_BASE_URL}/api/index-status${params}`;
  const cached = indexStatusCache.get(url);
  if (cached && cached.expiresAt > Date.now()) return cached.response;
  const response = await fetch(url);
  if (!response.ok) throw await apiError(response, "Could not load indexing status");
  const body = await responseJson<IndexStatus>(
    response,
    "Index status returned an invalid response",
  );
  indexStatusCache.set(url, { response: body, expiresAt: Date.now() + INDEX_STATUS_CACHE_TTL_MS });
  return body;
}

export function clearIndexStatusCache(): void {
  indexStatusCache.clear();
}

async function apiError(response: Response, fallback: string): Promise<Error> {
  const body = await response.text().catch(() => "");
  try {
    const parsed = JSON.parse(body) as { error?: string };
    return new Error(parsed.error ?? `${fallback} with HTTP ${response.status}`);
  } catch {
    return new Error(`${fallback} with HTTP ${response.status}`);
  }
}

async function responseJson<T>(response: Response, message: string): Promise<T> {
  const body = await response.text();
  try {
    return JSON.parse(body) as T;
  } catch {
    throw new Error(`${message} (HTTP ${response.status})`);
  }
}
