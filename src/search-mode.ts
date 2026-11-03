import type { SearchMode } from "./types";

const PREFERENCE_KEY = "ytsearch.search-mode.v1";

// Both are build flags so production can ship the code with neither surface until
// the semantic service it depends on is hosted.
export const hybridToggleEnabled = (): boolean =>
  import.meta.env.VITE_HYBRID_TOGGLE?.trim() === "true";
export const shareEnabled = (): boolean => import.meta.env.VITE_MOMENT_SHARING?.trim() === "true";

/** The mode a URL asks for, or undefined when this build offers no such choice. */
export function resolveSearchMode(value: string | null): SearchMode | undefined {
  if (!hybridToggleEnabled()) return undefined;
  if (value === "hybrid") return "hybrid";
  return value === "lexical" ? "lexical" : undefined;
}

export function loadSearchModePreference(storage = browserStorage()): SearchMode {
  if (!hybridToggleEnabled()) return "lexical";
  try {
    return storage?.getItem(PREFERENCE_KEY) === "hybrid" ? "hybrid" : "lexical";
  } catch {
    return "lexical";
  }
}

export function saveSearchModePreference(mode: SearchMode, storage = browserStorage()): void {
  try {
    storage?.setItem(PREFERENCE_KEY, mode);
  } catch {
    // Private browsing and quota errors must not break search.
  }
}

function browserStorage(): Storage | undefined {
  try {
    return typeof window === "undefined" ? undefined : window.localStorage;
  } catch {
    return undefined;
  }
}
