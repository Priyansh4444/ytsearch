import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";
import {
  ErrorResponseSchema,
  HealthSchema,
  IndexedChannelsResponseSchema,
  IndexQueueResponseSchema,
  IndexStatusSchema,
  SearchAvailabilitySchema,
  SearchResponseSchema,
} from "./api-schema";
import { ApiDecodeError, ApiResponseError, ApiTransportError, type ApiError } from "./api-error";
import type {
  DashboardData,
  IndexQueueRequest,
  IndexStatus,
  IndexedChannel,
  SearchAvailability,
  SearchRequest,
  SearchResponse,
} from "./types";

type AnalyticsProperties = Record<string, boolean | number | string | undefined>;
type AnalyticsConfig = {
  apiKey: string;
  apiHost: string;
};

const DEFAULT_PER_PAGE = 12;
const RETRY_DELAY_MS = 250;
const ANALYTICS_ID_KEY = "ytsearch.analytics-id.v1";

const decodeSearchResponseSchema = Schema.decodeUnknownEffect(SearchResponseSchema);

// Older search API responses do not include this field. Normalize them before
// schema decoding so the response remains compatible with the current type.
const decodeSearchResponse = (input: unknown) => {
  if (
    typeof input === "object" &&
    input !== null &&
    !Object.prototype.hasOwnProperty.call(input, "totals_approximate")
  ) {
    return decodeSearchResponseSchema({ ...input, totals_approximate: false });
  }
  return decodeSearchResponseSchema(input);
};
const decodeAvailability = Schema.decodeUnknownEffect(SearchAvailabilitySchema);
const decodeIndexedChannels = Schema.decodeUnknownEffect(IndexedChannelsResponseSchema);
const decodeIndexQueueResponse = Schema.decodeUnknownEffect(IndexQueueResponseSchema);
const decodeIndexStatus = Schema.decodeUnknownEffect(IndexStatusSchema);
const decodeHealth = Schema.decodeUnknownEffect(HealthSchema);
const decodeErrorResponse = Schema.decodeUnknownOption(ErrorResponseSchema);

export const searchTranscriptsEffect = (
  apiBaseUrl: string,
  request: SearchRequest,
): Effect.Effect<SearchResponse, ApiError> => {
  const params = new URLSearchParams({
    q: request.query,
    per_page: String(DEFAULT_PER_PAGE),
    page: String(request.page),
    sort: request.sort,
  });
  if (request.youtuber) params.set("youtuber", request.youtuber);
  const url = `${apiBaseUrl}/api/search?${params.toString()}`;
  const attempt = requestJsonEffect(
    url,
    { cache: "no-store" },
    decodeSearchResponse,
    "Search failed",
    "Search returned an invalid response",
  );

  return attempt.pipe(
    Effect.catchIf(isRetryableSearchError, () =>
      Effect.sleep(RETRY_DELAY_MS).pipe(Effect.andThen(attempt)),
    ),
  );
};

export const fetchSearchAvailabilityEffect = (
  apiBaseUrl: string,
): Effect.Effect<SearchAvailability, ApiError> =>
  requestJsonEffect(
    `${apiBaseUrl}/api/availability`,
    { cache: "no-store" },
    decodeAvailability,
    "Could not load search availability",
    "Search availability returned an invalid response",
  );

export const searchVideoMomentsEffect = (
  apiBaseUrl: string,
  query: string,
  videoId: string,
): Effect.Effect<SearchResponse, ApiError> => {
  const params = new URLSearchParams({ q: query, video_id: videoId, per_page: "100" });
  return requestJsonEffect(
    `${apiBaseUrl}/api/search?${params.toString()}`,
    { cache: "no-store" },
    decodeSearchResponse,
    "Could not load every matching moment",
    "Moment search returned an invalid response",
  );
};

export const indexedChannelsEffect = (
  apiBaseUrl: string,
): Effect.Effect<IndexedChannel[], ApiError> =>
  requestJsonEffect(
    `${apiBaseUrl}/api/youtubers`,
    undefined,
    decodeIndexedChannels,
    "Could not load indexed channels",
    "Channel list returned an invalid response",
  ).pipe(Effect.map((body) => body.channels));

export const requestIndexEffect = (
  apiBaseUrl: string,
  handle: string,
): Effect.Effect<IndexQueueRequest, ApiError> =>
  requestJsonEffect(
    `${apiBaseUrl}/api/index-requests`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ handle }),
    },
    decodeIndexQueueResponse,
    "Could not queue index request",
    "Queue returned an invalid response",
  ).pipe(Effect.map((body) => body.request));

export const indexStatusEffect = (
  apiBaseUrl: string,
  handle?: string,
): Effect.Effect<IndexStatus, ApiError> => {
  const params = handle ? `?handle=${encodeURIComponent(handle)}` : "";
  return requestJsonEffect(
    `${apiBaseUrl}/api/index-status${params}`,
    undefined,
    decodeIndexStatus,
    "Could not load indexing status",
    "Index status returned an invalid response",
  );
};

export const dashboardDataEffect = (apiBaseUrl: string): Effect.Effect<DashboardData, never> =>
  Effect.gen(function* () {
    const [health, channels, status] = yield* Effect.all(
      [
        settledTimed(
          requestJsonEffect(
            `${apiBaseUrl}/api/health`,
            { cache: "no-store" },
            decodeHealth,
            "Health check failed",
            "Health check returned an invalid response",
          ),
        ),
        settledTimed(indexedChannelsEffect(apiBaseUrl)),
        settledTimed(indexStatusEffect(apiBaseUrl)),
      ] as const,
      { concurrency: "unbounded" },
    );
    const errors = [health, channels, status].flatMap((result) =>
      result.ok ? [] : [errorMessage(result.error)],
    );

    return {
      channels: channels.ok ? channels.value : [],
      channelsAvailable: channels.ok,
      indexStatus: status.ok ? status.value : undefined,
      apiHealthy: health.ok && health.value.ok,
      fetchedAt: new Date().toISOString(),
      latencyMs: {
        health: health.ok ? health.latencyMs : undefined,
        channels: channels.ok ? channels.latencyMs : undefined,
        indexStatus: status.ok ? status.latencyMs : undefined,
      },
      errors,
    };
  });

export const captureAnalyticsEventEffect = (
  config: AnalyticsConfig,
  event: string,
  properties: AnalyticsProperties,
): Effect.Effect<void> =>
  Effect.gen(function* () {
    const distinctId = yield* Effect.sync(analyticsDistinctId);
    const compact = Object.fromEntries(
      Object.entries(properties).filter((entry) => entry[1] !== undefined),
    );
    const payload = {
      api_key: config.apiKey,
      batch: [
        {
          event,
          distinct_id: distinctId,
          timestamp: new Date().toISOString(),
          uuid: crypto.randomUUID(),
          properties: {
            distinct_id: distinctId,
            $device_id: distinctId,
            $lib: "ytsearch",
            $process_person_profile: false,
            $is_identified: false,
            $current_url: window.location.href,
            $pathname: window.location.pathname,
            $host: window.location.host,
            ...compact,
          },
        },
      ],
      sent_at: new Date().toISOString(),
    };
    const response = yield* Effect.tryPromise({
      try: (signal) =>
        fetch(`${config.apiHost.replace(/\/$/, "")}/batch/`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
          keepalive: true,
          signal,
        }),
      catch: () => undefined,
    });
    if (response && !response.ok) yield* Effect.fail(undefined);
  }).pipe(Effect.catch(() => Effect.void));

export const copyToClipboardEffect = (value: string): Effect.Effect<void, Error> => {
  if (navigator.clipboard?.writeText) {
    return Effect.tryPromise({
      try: () => navigator.clipboard.writeText(value),
      catch: (cause) =>
        cause instanceof Error ? cause : new Error("Could not copy to the clipboard", { cause }),
    });
  }

  return Effect.try({
    try: () => {
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
    },
    catch: (cause) =>
      cause instanceof Error ? cause : new Error("Could not copy to the clipboard", { cause }),
  });
};

export const runSearchTranscripts = (
  apiBaseUrl: string,
  request: SearchRequest,
  signal?: AbortSignal,
): Promise<SearchResponse> =>
  Effect.runPromise(searchTranscriptsEffect(apiBaseUrl, request), { signal });

export const runFetchSearchAvailability = (
  apiBaseUrl: string,
  signal?: AbortSignal,
): Promise<SearchAvailability> =>
  Effect.runPromise(fetchSearchAvailabilityEffect(apiBaseUrl), { signal });

export const runSearchVideoMoments = (
  apiBaseUrl: string,
  query: string,
  videoId: string,
  signal?: AbortSignal,
): Promise<SearchResponse> =>
  Effect.runPromise(searchVideoMomentsEffect(apiBaseUrl, query, videoId), { signal });

export const runIndexedChannels = (
  apiBaseUrl: string,
  signal?: AbortSignal,
): Promise<IndexedChannel[]> => Effect.runPromise(indexedChannelsEffect(apiBaseUrl), { signal });

export const runRequestIndex = (
  apiBaseUrl: string,
  handle: string,
  signal?: AbortSignal,
): Promise<IndexQueueRequest> =>
  Effect.runPromise(requestIndexEffect(apiBaseUrl, handle), { signal });

export const runIndexStatus = (
  apiBaseUrl: string,
  handle?: string,
  signal?: AbortSignal,
): Promise<IndexStatus> => Effect.runPromise(indexStatusEffect(apiBaseUrl, handle), { signal });

export const runDashboardData = (
  apiBaseUrl: string,
  signal?: AbortSignal,
): Promise<DashboardData> => Effect.runPromise(dashboardDataEffect(apiBaseUrl), { signal });

export const runCaptureAnalyticsEvent = (
  config: AnalyticsConfig,
  event: string,
  properties: AnalyticsProperties,
): Promise<void> => Effect.runPromise(captureAnalyticsEventEffect(config, event, properties));

export const runCopyToClipboard = (value: string): Promise<void> =>
  Effect.runPromise(copyToClipboardEffect(value));

function requestJsonEffect<A, E>(
  input: RequestInfo | URL,
  init: RequestInit | undefined,
  decode: (input: unknown) => Effect.Effect<A, E>,
  responseFallback: string,
  decodeMessage: string,
): Effect.Effect<A, ApiError> {
  return fetchEffect(input, init, responseFallback).pipe(
    Effect.flatMap((response): Effect.Effect<A, ApiError> => {
      if (response.ok) return decodeResponseEffect(response, decode, decodeMessage);
      return apiResponseErrorEffect(response, responseFallback);
    }),
  );
}

function fetchEffect(
  input: RequestInfo | URL,
  init: RequestInit | undefined,
  operation: string,
): Effect.Effect<Response, ApiTransportError> {
  return Effect.tryPromise({
    try: (signal) => fetch(input, { ...init, signal }),
    catch: (cause) => new ApiTransportError(operation, cause),
  });
}

function decodeResponseEffect<A, E>(
  response: Response,
  decode: (input: unknown) => Effect.Effect<A, E>,
  operation: string,
): Effect.Effect<A, ApiDecodeError> {
  return Effect.tryPromise({
    try: () => response.json() as Promise<unknown>,
    catch: (cause) => new ApiDecodeError(operation, response.status, cause),
  }).pipe(
    Effect.flatMap(decode),
    Effect.mapError((cause) =>
      cause instanceof ApiDecodeError
        ? cause
        : new ApiDecodeError(operation, response.status, cause),
    ),
  );
}

function apiResponseErrorEffect(
  response: Response,
  fallback: string,
): Effect.Effect<never, ApiResponseError | ApiTransportError> {
  return Effect.tryPromise({
    try: () => response.text(),
    catch: (cause) => new ApiTransportError(`${fallback} error response`, cause),
  }).pipe(
    Effect.flatMap((body) => {
      const parsed = parseErrorBody(body);
      return Effect.fail(
        new ApiResponseError(
          parsed?.error ?? `${fallback} with HTTP ${response.status}`,
          response.status,
          parsed?.code,
          parsed?.available_at,
        ),
      );
    }),
  );
}

function parseErrorBody(body: string): typeof ErrorResponseSchema.Type | undefined {
  try {
    const decoded = decodeErrorResponse(JSON.parse(body));
    return decoded._tag === "Some" ? decoded.value : undefined;
  } catch {
    return undefined;
  }
}

function isRetryableSearchError(error: ApiError): boolean {
  if (error instanceof ApiTransportError) return true;
  return (
    error instanceof ApiResponseError &&
    [502, 503, 504].includes(error.status) &&
    error.code !== "scheduled_offline" &&
    error.code !== "search_starting"
  );
}

type SettledTimed<A> =
  | { readonly ok: true; readonly value: A; readonly latencyMs: number }
  | { readonly ok: false; readonly error: unknown; readonly latencyMs: number };

function settledTimed<A, E>(effect: Effect.Effect<A, E>): Effect.Effect<SettledTimed<A>> {
  return Effect.suspend(() => {
    const started = performance.now();
    return Effect.match(effect, {
      onFailure: (error) => ({
        ok: false as const,
        error,
        latencyMs: Math.round(performance.now() - started),
      }),
      onSuccess: (value) => ({
        ok: true as const,
        value,
        latencyMs: Math.round(performance.now() - started),
      }),
    });
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function analyticsDistinctId(): string {
  try {
    const stored = window.localStorage.getItem(ANALYTICS_ID_KEY);
    if (stored) return stored;
    const generated = crypto.randomUUID();
    window.localStorage.setItem(ANALYTICS_ID_KEY, generated);
    return generated;
  } catch {
    return crypto.randomUUID();
  }
}
