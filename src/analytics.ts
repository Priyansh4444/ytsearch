import type { SearchRequest, SearchResponse } from "./types";

const apiKey = import.meta.env.VITE_POSTHOG_KEY?.trim();
const apiHost = import.meta.env.VITE_POSTHOG_HOST?.trim() || "https://us.i.posthog.com";
let initialized = false;
type PostHogClient = typeof import("posthog-js").default;
let client: PostHogClient | undefined;

type Properties = Record<string, boolean | number | string | undefined>;

export function initializeAnalytics(): void {
  if (!apiKey || initialized) return;
  void import("posthog-js").then(({ default: posthog }) => {
    posthog.init(apiKey, {
      api_host: apiHost,
      autocapture: false,
      capture_pageview: false,
      capture_pageleave: false,
      capture_exceptions: true,
      capture_performance: {
        web_vitals: true,
        web_vitals_allowed_metrics: ["CLS", "FCP", "INP", "LCP"],
      },
      disable_session_recording: true,
      person_profiles: "identified_only",
    });
    client = posthog;
    initialized = true;
    capture("page_view", { path: window.location.pathname });
  });
}

export function capture(event: string, properties: Properties = {}): void {
  if (initialized) client?.capture(event, properties);
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
