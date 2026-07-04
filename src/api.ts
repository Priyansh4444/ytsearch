import type { IndexedChannel, SearchRequest, SearchResponse } from "./types";

const DEFAULT_PER_PAGE = 12;
const API_BASE_URL = import.meta.env.VITE_API_BASE_URL ?? "";

export async function searchTranscripts(request: SearchRequest): Promise<SearchResponse> {
  const params = new URLSearchParams({
    q: request.query,
    per_page: String(DEFAULT_PER_PAGE),
    page: String(request.page),
    sort: request.sort,
  });
  if (request.youtuber) params.set("youtuber", request.youtuber);

  const response = await fetch(`${API_BASE_URL}/api/search?${params.toString()}`);
  if (!response.ok) throw await apiError(response, "Search failed");
  return response.json();
}

export async function indexedChannels(): Promise<IndexedChannel[]> {
  const response = await fetch(`${API_BASE_URL}/api/youtubers`);
  if (!response.ok) throw await apiError(response, "Could not load indexed channels");
  const body = (await response.json()) as { channels: IndexedChannel[] };
  return body.channels;
}

async function apiError(response: Response, fallback: string): Promise<Error> {
  const body = (await response.json().catch(() => null)) as {
    error?: string;
  } | null;
  return new Error(body?.error ?? `${fallback} with HTTP ${response.status}`);
}
