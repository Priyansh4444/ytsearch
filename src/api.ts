import { ApiResponseError } from "./api-error";
import { loadEffectRuntime } from "./effect-loader";
import type {
  DashboardData,
  IndexQueueRequest,
  IndexStatus,
  IndexedChannel,
  SearchAvailability,
  SearchRequest,
  SearchResponse,
} from "./types";

const SEARCH_CACHE_MS = 10_000;
const SEARCH_CACHE_LIMIT = 16;
export const API_BASE_URL = (import.meta.env.VITE_API_BASE_URL ?? "").replace(/\/$/, "");
const INDEX_QUEUE_API_BASE_URL = (
  import.meta.env.VITE_INDEX_QUEUE_API_BASE_URL ?? API_BASE_URL
).replace(/\/$/, "");
const searchCache = new Map<string, { expiresAt: number; response: Promise<SearchResponse> }>();
let inflightChannels: Promise<IndexedChannel[]> | undefined;

export { ApiResponseError };

export function searchTranscripts(
  request: SearchRequest,
  signal?: AbortSignal,
): Promise<SearchResponse> {
  const cacheKey = JSON.stringify(request);
  const cached = searchCache.get(cacheKey);
  if (cached && cached.expiresAt > Date.now()) return cached.response;
  searchCache.delete(cacheKey);

  const response = loadEffectRuntime()
    .then(({ runSearchTranscripts }) => runSearchTranscripts(API_BASE_URL, request, signal))
    .catch((error) => {
      if (searchCache.get(cacheKey)?.response === response) searchCache.delete(cacheKey);
      throw error;
    });
  searchCache.set(cacheKey, { expiresAt: Date.now() + SEARCH_CACHE_MS, response });
  if (searchCache.size > SEARCH_CACHE_LIMIT) searchCache.delete(searchCache.keys().next().value!);
  return response;
}

export function fetchSearchAvailability(signal?: AbortSignal): Promise<SearchAvailability> {
  return loadEffectRuntime().then(({ runFetchSearchAvailability }) =>
    runFetchSearchAvailability(API_BASE_URL, signal),
  );
}

export function searchVideoMoments(
  query: string,
  videoId: string,
  signal?: AbortSignal,
): Promise<SearchResponse> {
  return loadEffectRuntime().then(({ runSearchVideoMoments }) =>
    runSearchVideoMoments(API_BASE_URL, query, videoId, signal),
  );
}

export function indexedChannels(signal?: AbortSignal): Promise<IndexedChannel[]> {
  if (inflightChannels) return inflightChannels;
  const pending = loadEffectRuntime()
    .then(({ runIndexedChannels }) => runIndexedChannels(API_BASE_URL, signal))
    .finally(() => {
      if (inflightChannels === pending) inflightChannels = undefined;
    });
  inflightChannels = pending;
  return pending;
}

export function requestIndex(handle: string, signal?: AbortSignal): Promise<IndexQueueRequest> {
  return loadEffectRuntime().then(({ runRequestIndex }) =>
    runRequestIndex(INDEX_QUEUE_API_BASE_URL, handle, signal),
  );
}

export function indexStatus(handle?: string, signal?: AbortSignal): Promise<IndexStatus> {
  return loadEffectRuntime().then(({ runIndexStatus }) =>
    runIndexStatus(INDEX_QUEUE_API_BASE_URL, handle, signal),
  );
}

export function dashboardData(signal?: AbortSignal): Promise<DashboardData> {
  return loadEffectRuntime().then(({ runDashboardData }) => runDashboardData(API_BASE_URL, signal));
}
