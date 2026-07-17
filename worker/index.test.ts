import { describe, expect, test } from "vitest";
import { shouldWakeIndexer } from "./wake-policy";

describe("index request approval wake gate", () => {
  test("does not wake the indexer for a new public request", () => {
    expect(shouldWakeIndexer("POST", "/api/index-requests", true)).toBe(false);
  });

  test("wakes the indexer after an approved request", () => {
    expect(shouldWakeIndexer("POST", "/api/index-requests/request-1/approve", true)).toBe(true);
  });

  test("does not wake the indexer when approval fails upstream", () => {
    expect(shouldWakeIndexer("POST", "/api/index-requests/request-1/approve", false)).toBe(false);
  });
});
