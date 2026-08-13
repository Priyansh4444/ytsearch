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

const SEARCH_CACHE_LIMIT = 16;
export const API_BASE_URL = (import.meta.env.VITE_API_BASE_URL ?? "").replace(/\/$/, "");
const INDEX_QUEUE_API_BASE_URL = (
  import.meta.env.VITE_INDEX_QUEUE_API_BASE_URL ?? API_BASE_URL
).replace(/\/$/, "");
interface InflightSearch {
  readonly controller: AbortController;
  readonly promise: Promise<SearchResponse>;
  subscribers: number;
}

const inflightSearches = new Map<string, InflightSearch>();
let inflightChannels: Promise<IndexedChannel[]> | undefined;

export { ApiResponseError };

export function prepareSearchRuntime(): Promise<void> {
  return loadEffectRuntime().then(() => undefined);
}

export function searchTranscripts(
  request: SearchRequest,
  signal?: AbortSignal,
): Promise<SearchResponse> {
  if (signal?.aborted)
    return Promise.reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
  const cacheKey = JSON.stringify(request);
  let inflight = inflightSearches.get(cacheKey);
  if (!inflight) {
    const controller = new AbortController();
    const entry: InflightSearch = {
      controller,
      subscribers: 0,
      promise: loadEffectRuntime()
        .then(({ runSearchTranscripts }) =>
          runSearchTranscripts(API_BASE_URL, request, controller.signal),
        )
        .finally(() => {
          if (inflightSearches.get(cacheKey) === entry) inflightSearches.delete(cacheKey);
        }),
    };
    inflight = entry;
    inflightSearches.set(cacheKey, entry);
    if (inflightSearches.size > SEARCH_CACHE_LIMIT) {
      const oldestKey = inflightSearches.keys().next().value!;
      const oldest = inflightSearches.get(oldestKey);
      inflightSearches.delete(oldestKey);
      oldest?.controller.abort();
    }
  }

  const callerStarted = performance.now();
  inflight.subscribers += 1;
  const shared = inflight;
  return new Promise<SearchResponse>((resolve, reject) => {
    let settled = false;
    const finish = (complete: () => void) => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener("abort", abort);
      shared.subscribers -= 1;
      if (shared.subscribers === 0 && inflightSearches.get(cacheKey) === shared) {
        inflightSearches.delete(cacheKey);
        shared.controller.abort();
      }
      complete();
    };
    const abort = () =>
      finish(() => reject(signal?.reason ?? new DOMException("Aborted", "AbortError")));

    if (signal?.aborted) {
      abort();
      return;
    }
    signal?.addEventListener("abort", abort, { once: true });
    shared.promise.then(
      (result) => finish(() => resolve(result)),
      (error: unknown) => finish(() => reject(error)),
    );
  }).then((result) => ({
    ...result,
    request_duration_ms: Math.max(0, Math.round(performance.now() - callerStarted)),
  }));
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
