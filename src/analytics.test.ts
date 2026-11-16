import { beforeEach, describe, expect, it, vi } from "vitest";

describe("analytics", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    vi.stubEnv("VITE_POSTHOG_KEY", "phc_test_project");
    vi.stubEnv("VITE_POSTHOG_HOST", "https://analytics.example.com");
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 200 })));
  });

  it("queues an early search event and excludes the query text", async () => {
    // Search pages carry the query in the URL; it must not leak through it.
    window.history.pushState({}, "", "/?q=private%20search%20text#result-video1");
    const analytics = await import("./analytics");

    analytics.captureSearchSubmitted({
      query: "private search text",
      youtuber: "@example",
      page: 1,
      sort: "relevance",
    });
    analytics.initializeAnalytics();

    const fetchMock = vi.mocked(fetch);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
    const firstBody = fetchMock.mock.calls[0]?.[1]?.body;
    if (typeof firstBody !== "string") throw new TypeError("Expected a JSON request body");
    const firstPayload = JSON.parse(firstBody);
    expect(firstPayload.batch[0]).toMatchObject({
      event: "search_submitted",
      properties: {
        query_length: 19,
        query_terms: 3,
        filtered_to_channel: true,
        sort: "relevance",
        page: 1,
      },
    });
    const sent = JSON.stringify(fetchMock.mock.calls);
    expect(sent).not.toContain("private search text");
    expect(sent).not.toContain("private%20search%20text");
    expect(firstPayload.batch[0].properties.$current_url).toBe(`${window.location.origin}/`);
    window.history.pushState({}, "", "/");
  });

  it("reports browser request time without stale cached backend time", async () => {
    const analytics = await import("./analytics");
    analytics.initializeAnalytics();
    analytics.captureSearchLoaded(
      { query: "react", youtuber: "", page: 1, sort: "relevance", mode: "lexical" },
      {
        query: "react",
        total_videos: 0,
        total_moments: 0,
        totals_approximate: false,
        total_found: 0,
        out_of: 100,
        time_to_first_result_ms: 4605,
        request_duration_ms: 72,
        edge_cached: true,
        page: 1,
        per_page: 20,
        sort: "relevance",
        mode: "lexical",
        has_next_page: false,
        videos: [],
      },
    );
    const body = vi.mocked(fetch).mock.calls.at(-1)?.[1]?.body;
    if (typeof body !== "string") throw new TypeError("Expected analytics body");
    const properties = JSON.parse(body).batch[0].properties;
    expect(properties.request_duration_ms).toBe(72);
    expect(properties.edge_cached).toBe(true);
    expect(properties).not.toHaveProperty("time_to_first_result_ms");
  });

  it("initializes once and uses the configured capture host", async () => {
    const analytics = await import("./analytics");

    analytics.initializeAnalytics();
    analytics.initializeAnalytics();

    const fetchMock = vi.mocked(fetch);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    expect(fetchMock).toHaveBeenCalledWith(
      "https://analytics.example.com/batch/",
      expect.objectContaining({ method: "POST", keepalive: true }),
    );
  });
});
