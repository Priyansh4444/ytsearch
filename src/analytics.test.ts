import { beforeEach, describe, expect, it, vi } from "vitest";

const runtime = vi.hoisted(() => ({
  capture: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("./effect-loader", () => ({
  loadEffectRuntime: () => Promise.resolve({ runCaptureAnalyticsEvent: runtime.capture }),
}));

describe("analytics", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    vi.stubEnv("VITE_POSTHOG_KEY", "phc_test_project");
    vi.stubEnv("VITE_POSTHOG_HOST", "https://analytics.example.com");
  });

  it("queues an early search event and excludes the query text", async () => {
    const analytics = await import("./analytics");

    analytics.captureSearchSubmitted({
      query: "private search text",
      youtuber: "@example",
      page: 1,
      sort: "relevance",
    });
    analytics.initializeAnalytics();

    await vi.waitFor(() => expect(runtime.capture).toHaveBeenCalledTimes(2));
    expect(runtime.capture).toHaveBeenNthCalledWith(
      1,
      { apiKey: "phc_test_project", apiHost: "https://analytics.example.com" },
      "search_submitted",
      {
        query_length: 19,
        query_terms: 3,
        filtered_to_channel: true,
        sort: "relevance",
        page: 1,
      },
    );
    expect(runtime.capture).toHaveBeenNthCalledWith(
      2,
      { apiKey: "phc_test_project", apiHost: "https://analytics.example.com" },
      "page_view",
      { path: "/" },
    );
    expect(JSON.stringify(runtime.capture.mock.calls)).not.toContain("private search text");
  });

  it("initializes once and uses the configured capture host", async () => {
    const analytics = await import("./analytics");

    analytics.initializeAnalytics();
    analytics.initializeAnalytics();

    await vi.waitFor(() => expect(runtime.capture).toHaveBeenCalledOnce());
    expect(runtime.capture).toHaveBeenCalledWith(
      { apiKey: "phc_test_project", apiHost: "https://analytics.example.com" },
      "page_view",
      { path: "/" },
    );
  });
});
