import type { SearchRequest, SearchResponse } from "./types";

const apiKey = import.meta.env.VITE_POSTHOG_KEY?.trim();
const apiHost = import.meta.env.VITE_POSTHOG_HOST?.trim() || "https://us.i.posthog.com";
let initialized = false;
let initializing = false;
type PostHogClient = InstanceType<typeof import("posthog-js-lite").default>;
let client: Pick<PostHogClient, "capture"> | undefined;

type Properties = Record<string, boolean | number | string | undefined>;
const pendingEvents: Array<{ event: string; properties: Properties }> = [];
const MAX_PENDING_EVENTS = 50;

export function initializeAnalytics(): void {
  if (!apiKey || initialized || initializing) return;
  initializing = true;
  void import("posthog-js-lite")
    .then(({ default: PostHog }) => {
      const posthog = new PostHog(apiKey, {
        host: apiHost,
        autocapture: false,
        captureHistoryEvents: false,
        disableRemoteFeatureFlags: true,
        persistence: "localStorage",
        preloadFeatureFlags: false,
      });
      client = posthog;
      initialized = true;
      initializing = false;
      for (const pending of pendingEvents.splice(0)) {
        posthog.capture(pending.event, compactProperties(pending.properties));
      }
      posthog.capture("page_view", { path: window.location.pathname });
    })
    .catch(() => {
      initializing = false;
      pendingEvents.length = 0;
    });
}

export function capture(event: string, properties: Properties = {}): void {
  if (!apiKey) return;
  if (initialized) {
    client?.capture(event, compactProperties(properties));
    return;
  }
  if (pendingEvents.length < MAX_PENDING_EVENTS) pendingEvents.push({ event, properties });
}

export function captureSearchSubmitted(request: SearchRequest): void {
  capture("search_submitted", searchProperties(request));
}

export function captureSearchLoaded(request: SearchRequest, response: SearchResponse): void {
  capture("search_loaded", {
    ...searchProperties(request),
    total_videos: response.total_videos,
    total_moments: response.total_moments,
    returned_videos: response.videos.length,
    time_to_first_result_ms: response.time_to_first_result_ms,
  });
}

function searchProperties(request: SearchRequest): Properties {
  return {
    query_length: request.query.length,
    query_terms: request.query.trim().split(/\s+/).filter(Boolean).length,
    filtered_to_channel: Boolean(request.youtuber),
    sort: request.sort,
    page: request.page,
  };
}

function compactProperties(properties: Properties): Record<string, boolean | number | string> {
  return Object.fromEntries(
    Object.entries(properties).filter(
      (entry): entry is [string, boolean | number | string] => entry[1] !== undefined,
    ),
  );
}
