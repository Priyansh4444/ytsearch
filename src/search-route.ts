import { evaluateSearchSchedule } from "../shared/service-schedule";
import type { SearchAvailability, SearchRequest, SearchSort } from "./types";
import { isAvailableSearchSort } from "./search-sorts";
import { creatorFromPath, normalizeHandle } from "./utils";

export const provisionalAvailability = (now = new Date()): SearchAvailability => {
  const schedule = evaluateSearchSchedule(now);
  return {
    status:
      schedule.phase === "open"
        ? "starting"
        : schedule.phase === "prewarm"
          ? "prewarming"
          : "scheduled_offline",
    search_enabled: schedule.phase === "open",
    enforced: true,
    schedule_policy: "scheduled",
    schedule_phase: schedule.phase,
    time_zone: "America/Los_Angeles",
    daily_window: {
      opens: "12:00",
      closes: "00:00",
      label: "12:00 PM–12:00 AM Pacific Time",
    },
    next_transition_at: schedule.nextTransitionAt,
  };
};

export const searchStateFromLocation = (): SearchRequest | undefined => {
  const params = new URLSearchParams(window.location.search);
  const query = params.get("q")?.trim();
  if (!query) return undefined;
  const sort = (params.get("sort") ?? "relevance") as SearchSort;
  const page = Number(params.get("page") ?? "1");
  return {
    query,
    youtuber: normalizeHandle(params.get("youtuber") ?? creatorFromPath() ?? ""),
    page: Number.isFinite(page) && page > 0 ? Math.floor(page) : 1,
    sort: isAvailableSearchSort(sort) ? sort : "relevance",
  };
};

export const writeSearchUrl = (request: SearchRequest): void => {
  const params = new URLSearchParams();
  params.set("q", request.query);
  if (request.youtuber) params.set("youtuber", normalizeHandle(request.youtuber));
  if (request.sort !== "relevance") params.set("sort", request.sort);
  if (request.page > 1) params.set("page", String(request.page));
  window.history.pushState({}, "", `/?${params.toString()}`);
};

export const resultAnchorId = (videoId: string): string => `video-${encodeURIComponent(videoId)}`;

export const resultAnchorFromHash = (hash: string): string | undefined => {
  const value = hash.startsWith("#") ? hash.slice(1) : hash;
  if (!value.startsWith("video-")) return undefined;
  try {
    const videoId = decodeURIComponent(value.slice("video-".length));
    return videoId ? resultAnchorId(videoId) : undefined;
  } catch {
    return undefined;
  }
};

export const searchResultPermalink = (
  videoId: string,
  location: Pick<Location, "href"> = window.location,
): string => {
  const url = new URL(location.href);
  url.hash = resultAnchorId(videoId);
  return url.toString();
};

export const scrollToSearchResult = (
  location: Pick<Location, "hash"> = window.location,
): boolean => {
  const id = resultAnchorFromHash(location.hash);
  if (!id) return false;
  const element = document.getElementById(id);
  if (!(element instanceof HTMLElement)) return false;
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  element.scrollIntoView({
    behavior: reduceMotion ? "auto" : "smooth",
    block: "start",
  });
  element.focus({ preventScroll: true });
  return true;
};
