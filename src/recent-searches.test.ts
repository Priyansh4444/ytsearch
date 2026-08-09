import { describe, expect, test } from "vitest";
import {
  RECENT_SEARCHES_KEY,
  clearRecentSearches,
  loadRecentSearches,
  saveRecentSearch,
} from "./recent-searches";

const request = (query: string, page = 1) => ({
  query,
  youtuber: "@Example",
  page,
  sort: "relevance" as const,
});

describe("recent searches", () => {
  test("stores successful page-one searches and normalizes handles", () => {
    const storage = new MemoryStorage();
    const recent = saveRecentSearch(
      [],
      request("  Rust testing  "),
      "2026-07-26T20:00:00Z",
      storage,
    );

    expect(recent).toEqual([
      {
        query: "Rust testing",
        youtuber: "@Example",
        sort: "relevance",
        searchedAt: "2026-07-26T20:00:00Z",
      },
    ]);
    expect(loadRecentSearches(storage)).toEqual(recent);
  });

  test("deduplicates, moves repeats first, caps at twelve, and ignores pagination", () => {
    const storage = new MemoryStorage();
    let recent = [];
    for (let index = 0; index < 14; index += 1) {
      recent = saveRecentSearch(recent, request(`query ${index}`), undefined, storage);
    }
    expect(recent).toHaveLength(12);
    expect(recent[0]?.query).toBe("query 13");
    expect(recent.at(-1)?.query).toBe("query 2");

    recent = saveRecentSearch(recent, request("query 5"), "2026-07-26T21:00:00Z", storage);
    expect(recent[0]?.query).toBe("query 5");
    expect(recent).toHaveLength(12);
    expect(saveRecentSearch(recent, request("another", 2), undefined, storage)).toBe(recent);
  });

  test("survives corrupt and unavailable storage and can be cleared", () => {
    const storage = new MemoryStorage();
    storage.setItem(RECENT_SEARCHES_KEY, "{");
    expect(loadRecentSearches(storage)).toEqual([]);
    saveRecentSearch([], request("rust"), undefined, storage);
    expect(clearRecentSearches(storage)).toEqual([]);
    expect(storage.getItem(RECENT_SEARCHES_KEY)).toBeNull();

    const broken = {
      getItem: () => {
        throw new Error("denied");
      },
    } as unknown as Storage;
    expect(loadRecentSearches(broken)).toEqual([]);
  });
});

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  get length() {
    return this.values.size;
  }
  clear() {
    this.values.clear();
  }
  getItem(key: string) {
    return this.values.get(key) ?? null;
  }
  key(index: number) {
    return [...this.values.keys()][index] ?? null;
  }
  removeItem(key: string) {
    this.values.delete(key);
  }
  setItem(key: string, value: string) {
    this.values.set(key, value);
  }
}
