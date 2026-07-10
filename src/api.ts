import type { IndexQueueRequest, IndexedChannel, SearchRequest, SearchResponse } from "./types";

const DEFAULT_PER_PAGE = 12;
const API_BASE_URL = (import.meta.env.VITE_API_BASE_URL ?? "").replace(/\/$/, "");
const INDEX_QUEUE_API_BASE_URL = (
  import.meta.env.VITE_INDEX_QUEUE_API_BASE_URL ?? API_BASE_URL
).replace(/\/$/, "");

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

  const response = await fetch(`${API_BASE_URL}/api/search?${params.toString()}`, {
    signal: options.signal,
  });
  if (!response.ok) throw await apiError(response, "Search failed");
  return response.json();
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

async function apiError(response: Response, fallback: string): Promise<Error> {
  const body = (await response.json().catch(() => null)) as {
    error?: string;
  } | null;
  return new Error(body?.error ?? `${fallback} with HTTP ${response.status}`);
}
