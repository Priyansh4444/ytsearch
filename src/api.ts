import type {
  DashboardData,
  IndexQueueRequest,
  IndexStatus,
  IndexedChannel,
  SearchRequest,
  SearchResponse,
  SearchAvailability,
} from "./types";

const DEFAULT_PER_PAGE = 12;
export const API_BASE_URL = (import.meta.env.VITE_API_BASE_URL ?? "").replace(/\/$/, "");
const SEMANTIC_API_BASE_URL = (import.meta.env.VITE_SEMANTIC_API_BASE_URL ?? API_BASE_URL).replace(
  /\/$/,
  "",
);
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
    mode: request.mode,
  });
  if (request.youtuber) params.set("youtuber", request.youtuber);

  const searchBaseUrl = request.mode === "semantic" ? SEMANTIC_API_BASE_URL : API_BASE_URL;
  const url = `${searchBaseUrl}/api/search?${params.toString()}`;
  const cached = searchCache.get(url);
  if (cached && cached.expiresAt > Date.now()) return cached.response;

  let response = await fetch(url, { signal: options.signal });
  if (response.status === 502 || response.status === 503 || response.status === 504) {
    const firstError = await apiError(response.clone(), "Search failed");
    if (
      firstError instanceof ApiResponseError &&
      ["scheduled_offline", "search_starting"].includes(firstError.code ?? "")
    ) {
      throw firstError;
    }
    await new Promise((resolve) => window.setTimeout(resolve, 250));
    response = await fetch(url, { signal: options.signal });
  }
  if (!response.ok) throw await apiError(response, "Search failed");
  const body = await responseJson<SearchResponse>(response, "Search returned an invalid response");
  searchCache.set(url, { response: body, expiresAt: Date.now() + SEARCH_CACHE_TTL_MS });
  return body;
}

export async function fetchSearchAvailability(): Promise<SearchAvailability> {
  const response = await fetch(`${API_BASE_URL}/api/availability`, { cache: "no-store" });
  if (!response.ok) throw await apiError(response, "Could not load search availability");
  return responseJson<SearchAvailability>(
    response,
    "Search availability returned an invalid response",
  );
}

export async function searchVideoMoments(query: string, videoId: string): Promise<SearchResponse> {
  const params = new URLSearchParams({
    q: query,
    video_id: videoId,
    per_page: "100",
  });
  const response = await fetch(`${API_BASE_URL}/api/search?${params.toString()}`);
  if (!response.ok) throw await apiError(response, "Could not load every matching moment");
  return responseJson<SearchResponse>(response, "Moment search returned an invalid response");
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

export async function dashboardData(): Promise<DashboardData> {
  const timed = async <T>(operation: () => Promise<T>) => {
    const started = performance.now();
    const value = await operation();
    return { value, latencyMs: Math.round(performance.now() - started) };
  };
  const [health, channels, status] = await Promise.allSettled([
    timed(async () => {
      const response = await fetch(`${API_BASE_URL}/api/health`, { cache: "no-store" });
      if (!response.ok) throw await apiError(response, "Health check failed");
      return responseJson<{ ok: boolean }>(response, "Health check returned an invalid response");
    }),
    timed(indexedChannels),
    timed(() => {
      clearIndexStatusCache();
      return indexStatus();
    }),
  ]);
  const errors = [health, channels, status]
    .filter((result): result is PromiseRejectedResult => result.status === "rejected")
    .map((result) =>
      result.reason instanceof Error ? result.reason.message : String(result.reason),
    );

  return {
    channels: channels.status === "fulfilled" ? channels.value.value : [],
    channelsAvailable: channels.status === "fulfilled",
    indexStatus: status.status === "fulfilled" ? status.value.value : undefined,
    apiHealthy: health.status === "fulfilled" && health.value.value.ok,
    fetchedAt: new Date().toISOString(),
    latencyMs: {
      health: health.status === "fulfilled" ? health.value.latencyMs : undefined,
      channels: channels.status === "fulfilled" ? channels.value.latencyMs : undefined,
      indexStatus: status.status === "fulfilled" ? status.value.latencyMs : undefined,
    },
    errors,
  };
}

export class ApiResponseError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: string,
    readonly availableAt?: string,
  ) {
    super(message);
    this.name = "ApiResponseError";
  }
}

async function apiError(response: Response, fallback: string): Promise<Error> {
  const body = await response.text().catch(() => "");
  try {
    const parsed = JSON.parse(body) as { error?: string; code?: string; available_at?: string };
    return new ApiResponseError(
      parsed.error ?? `${fallback} with HTTP ${response.status}`,
      response.status,
      parsed.code,
      parsed.available_at,
    );
  } catch {
    return new ApiResponseError(`${fallback} with HTTP ${response.status}`, response.status);
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
