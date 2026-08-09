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
