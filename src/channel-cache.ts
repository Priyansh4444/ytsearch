import type { IndexedChannel } from "./types";

export const CHANNEL_CACHE_KEY = "ytsearch.indexed-channels.v2";
// This cache is only a paint optimization. Keep it short-lived so a backend
// cutover or snapshot rollback cannot render creator records that the active
// search corpus no longer contains.
const CHANNEL_CACHE_MAX_AGE_MS = 30 * 60 * 1_000;

type CachedChannels = {
  fetchedAt: string;
  channels: IndexedChannel[];
};

export function loadCachedChannels(
  now = Date.now(),
  storage = browserStorage(),
): IndexedChannel[] | undefined {
  if (!storage) return undefined;
  try {
    const parsed = JSON.parse(
      storage.getItem(CHANNEL_CACHE_KEY) ?? "null",
    ) as CachedChannels | null;
    if (
      !parsed ||
      !Array.isArray(parsed.channels) ||
      typeof parsed.fetchedAt !== "string" ||
      now - new Date(parsed.fetchedAt).getTime() > CHANNEL_CACHE_MAX_AGE_MS
    ) {
      return undefined;
    }
    return parsed.channels;
  } catch {
    return undefined;
  }
}

export function cacheChannels(
  channels: IndexedChannel[],
  fetchedAt = new Date().toISOString(),
  storage = browserStorage(),
): void {
  try {
    storage?.setItem(CHANNEL_CACHE_KEY, JSON.stringify({ fetchedAt, channels }));
  } catch {
    // Channel caching is optional.
  }
}

function browserStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}
