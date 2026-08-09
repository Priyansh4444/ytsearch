import type { RecentSearch, SearchRequest } from "./types";
import { isSearchSort } from "./search-sorts";
import { normalizeHandle } from "./utils";

export const RECENT_SEARCHES_KEY = "ytsearch.recent-searches.v1";
const MAX_RECENT_SEARCHES = 12;

export function loadRecentSearches(storage = browserStorage()): RecentSearch[] {
  if (!storage) return [];
  try {
    const parsed: unknown = JSON.parse(storage.getItem(RECENT_SEARCHES_KEY) ?? "[]");
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isRecentSearch).slice(0, MAX_RECENT_SEARCHES);
  } catch {
    return [];
  }
}

export function saveRecentSearch(
  current: RecentSearch[],
  request: SearchRequest,
  searchedAt = new Date().toISOString(),
  storage = browserStorage(),
): RecentSearch[] {
  if (request.page !== 1) return current;
  const recent: RecentSearch = {
    query: request.query.trim(),
    youtuber: normalizeHandle(request.youtuber),
    sort: request.sort,
    searchedAt,
  };
  const key = recentKey(recent);
  const next = [recent, ...current.filter((item) => recentKey(item) !== key)].slice(
    0,
    MAX_RECENT_SEARCHES,
  );
  try {
    storage?.setItem(RECENT_SEARCHES_KEY, JSON.stringify(next));
  } catch {
    // Private browsing and quota errors must not break search.
  }
  return next;
}

export function clearRecentSearches(storage = browserStorage()): RecentSearch[] {
  try {
    storage?.removeItem(RECENT_SEARCHES_KEY);
  } catch {
    // Storage is an optional enhancement.
  }
  return [];
}

function recentKey(search: RecentSearch): string {
  return [
    search.query.trim().toLocaleLowerCase(),
    normalizeHandle(search.youtuber),
    search.sort,
  ].join("\u0000");
}

function isRecentSearch(value: unknown): value is RecentSearch {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<RecentSearch>;
  return (
    typeof candidate.query === "string" &&
    candidate.query.trim().length > 0 &&
    typeof candidate.youtuber === "string" &&
    isSearchSort(candidate.sort) &&
    typeof candidate.searchedAt === "string"
  );
}

function browserStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}
