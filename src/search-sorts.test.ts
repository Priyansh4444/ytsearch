import { describe, expect, test } from "vitest";
import {
  isAvailableSearchSort,
  isSearchSort,
  searchSortLabel,
  searchSortOptions,
} from "./search-sorts";

describe("search sorts", () => {
  test("keeps engagement choices exclusive to the experimental UI", () => {
    expect(searchSortOptions(false).map(([value]) => value)).toEqual([
      "relevance",
      "published_desc",
      "published_asc",
    ]);
    expect(searchSortOptions(true).map(([value]) => value)).toContain("relevance_engagement");
    expect(isAvailableSearchSort("views_desc", false)).toBe(false);
    expect(isAvailableSearchSort("views_desc", true)).toBe(true);
  });

  test("recognizes every API sort and exposes unambiguous labels", () => {
    for (const [sort, label] of searchSortOptions(true)) {
      expect(isSearchSort(sort)).toBe(true);
      expect(searchSortLabel(sort)).toBe(label);
    }
    expect(isSearchSort("viral")).toBe(false);
  });
});
