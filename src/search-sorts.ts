import type { SearchSort } from "./types";

const BASE_SORTS = ["relevance", "published_desc", "published_asc"] as const;
const ENGAGEMENT_SORTS = [
  "relevance_engagement",
  "views_desc",
  "likes_desc",
  "comments_desc",
] as const;

export const engagementSortsEnabled = import.meta.env.VITE_ENGAGEMENT_SORTS === "true";

export const searchSortOptions = (engagement = engagementSortsEnabled) =>
  engagement
    ? ([
        ["relevance", "Relevant"],
        ["relevance_engagement", "Relevant + engagement"],
        ["views_desc", "Most viewed"],
        ["likes_desc", "Most liked"],
        ["comments_desc", "Most discussed"],
        ["published_desc", "Newest"],
        ["published_asc", "Oldest"],
      ] as const)
    : ([
        ["relevance", "Relevant"],
        ["published_desc", "Newest"],
        ["published_asc", "Oldest"],
      ] as const);

export function isSearchSort(value: unknown): value is SearchSort {
  return typeof value === "string" && ALL_SORTS.has(value);
}

// Module-scope set: membership check without rebuilding a 7-element array on
// every route parse.
const ALL_SORTS: ReadonlySet<string> = new Set<string>([...BASE_SORTS, ...ENGAGEMENT_SORTS]);

export function isAvailableSearchSort(
  value: unknown,
  engagement = engagementSortsEnabled,
): value is SearchSort {
  return (
    isSearchSort(value) && (engagement || !(ENGAGEMENT_SORTS as readonly string[]).includes(value))
  );
}

export function searchSortLabel(sort: SearchSort): string {
  return searchSortOptions(true).find(([value]) => value === sort)?.[1] ?? "Relevant";
}
