import type { SearchRequest, SearchResponse } from "./types";

const apiKey = import.meta.env.VITE_POSTHOG_KEY?.trim();
const apiHost = import.meta.env.VITE_POSTHOG_HOST?.trim() || "https://us.i.posthog.com";
let initialized = false;

type Properties = Record<string, boolean | number | string | undefined>;
const pendingEvents: Array<{ event: string; properties: Properties }> = [];
const MAX_PENDING_EVENTS = 50;
const ANALYTICS_ID_KEY = "ytsearch.analytics-id.v1";

export function initializeAnalytics(): void {
  if (!apiKey || initialized) return;
  initialized = true;
  for (const pending of pendingEvents.splice(0)) send(pending.event, pending.properties);
  send("page_view", { path: window.location.pathname });
}

export function capture(event: string, properties: Properties = {}): void {
  if (!apiKey) return;
  if (initialized) {
    send(event, properties);
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

function send(event: string, properties: Properties): void {
  const distinctId = analyticsDistinctId();
  const timestamp = new Date().toISOString();
  const payload = {
    api_key: apiKey!,
    batch: [
      {
        event,
        distinct_id: distinctId,
        timestamp,
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
          ...Object.fromEntries(
            Object.entries(properties).filter((entry) => entry[1] !== undefined),
          ),
        },
      },
    ],
    sent_at: timestamp,
  };
  void fetch(`${apiHost.replace(/\/$/, "")}/batch/`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
    keepalive: true,
  }).catch(() => undefined);
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

function searchProperties(request: SearchRequest): Properties {
  return {
    query_length: request.query.length,
    query_terms: request.query.trim().split(/\s+/).filter(Boolean).length,
    filtered_to_channel: Boolean(request.youtuber),
    sort: request.sort,
    page: request.page,
  };
}
