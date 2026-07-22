import { beforeEach, describe, expect, it, vi } from "vitest";

const posthog = vi.hoisted(() => {
  const capture = vi.fn();
  const client = { capture };
  const init = vi.fn((_key: string, config: { loaded?: (loadedClient: typeof client) => void }) =>
    config.loaded?.(client),
  );
  return { capture, client, init };
});

vi.mock("posthog-js", () => ({
  default: { capture: posthog.capture, init: posthog.init },
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

    analytics.initializeAnalytics();
    analytics.captureSearchSubmitted({
      query: "private search text",
      youtuber: "@example",
      page: 1,
      sort: "relevance",
      mode: "lexical",
    });

    await vi.waitFor(() => expect(posthog.capture).toHaveBeenCalledTimes(2));
    expect(posthog.capture).toHaveBeenNthCalledWith(1, "search_submitted", {
      query_length: 19,
      query_terms: 3,
      filtered_to_channel: true,
      sort: "relevance",
      page: 1,
    });
    expect(posthog.capture).toHaveBeenNthCalledWith(2, "page_view", { path: "/" });
    expect(JSON.stringify(posthog.capture.mock.calls)).not.toContain("private search text");
  });

  it("initializes once with the privacy-preserving production settings", async () => {
    const analytics = await import("./analytics");

    analytics.initializeAnalytics();
    analytics.initializeAnalytics();

    await vi.waitFor(() => expect(posthog.init).toHaveBeenCalledOnce());
    const [key, config] = posthog.init.mock.calls[0]!;
    expect(key).toBe("phc_test_project");
    expect(config).toMatchObject({
      api_host: "https://analytics.example.com",
      autocapture: false,
      capture_pageview: false,
      capture_pageleave: false,
      disable_session_recording: true,
      person_profiles: "identified_only",
    });
  });
});
