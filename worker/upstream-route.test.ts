import { describe, expect, test } from "vitest";
import { isIndexQueueRoute, upstreamApiBase } from "./upstream-route";

describe("upstream API routing", () => {
  test("keeps search and read APIs on the experiment backend", () => {
    for (const path of ["/api/search", "/api/youtubers", "/api/channel-images/UC123"]) {
      expect(isIndexQueueRoute(path)).toBe(false);
      expect(upstreamApiBase(path, "https://raw.example", "https://queue.example")).toBe(
        "https://raw.example",
      );
    }
  });

  test("shares every index queue contract with production", () => {
    for (const path of [
      "/api/index-requests",
      "/api/index-requests/request-1/approve",
      "/api/index-status",
      "/api/index-activity",
    ]) {
      expect(isIndexQueueRoute(path)).toBe(true);
      expect(upstreamApiBase(path, "https://raw.example", "https://queue.example")).toBe(
        "https://queue.example",
      );
    }
  });

  test("falls back to the search API when no shared queue URL is configured", () => {
    expect(upstreamApiBase("/api/index-status", "https://api.example")).toBe("https://api.example");
  });
});
