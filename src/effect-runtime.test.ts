import * as Effect from "effect/Effect";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  captureAnalyticsEventEffect,
  dashboardDataEffect,
  runFetchSearchAvailability,
  runSearchTranscripts,
  runSearchVideoMoments,
} from "./effect-runtime";

describe("browser Effect runtime", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    window.localStorage.clear();
  });

  it("runs dashboard requests concurrently and keeps partial failures typed", async () => {
    const pending = new Map<string, (response: Response) => void>();
    const fetchMock = vi.fn(
      (input: RequestInfo | URL) =>
        new Promise<Response>((resolve) =>
          pending.set(
            typeof input === "string" ? input : input instanceof URL ? input.href : input.url,
            resolve,
          ),
        ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = Effect.runPromise(dashboardDataEffect("https://api.example.com"));
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(3));
    pending.get("https://api.example.com/api/health")?.(Response.json({ ok: true }));
    pending.get("https://api.example.com/api/youtubers")?.(Response.json({ channels: [] }));
    pending.get("https://api.example.com/api/index-status")?.(
      new Response(JSON.stringify({ error: "queue unavailable" }), { status: 503 }),
    );

    await expect(result).resolves.toMatchObject({
      apiHealthy: true,
      channelsAvailable: true,
      channels: [],
      errors: ["queue unavailable"],
    });
  });

  it("propagates boundary cancellation to fetch", async () => {
    let fetchSignal: AbortSignal | undefined;
    const fetchMock = vi.fn(
      (_input: RequestInfo | URL, init?: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          fetchSignal = init?.signal ?? undefined;
          fetchSignal?.addEventListener("abort", () => reject(fetchSignal?.reason), {
            once: true,
          });
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const controller = new AbortController();

    const result = runFetchSearchAvailability("https://api.example.com", controller.signal);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    controller.abort();

    await expect(result).rejects.toThrow();
    expect(fetchSignal?.aborted).toBe(true);
  });

  it("bypasses browser HTTP caches for search reads", async () => {
    const fetchMock = vi.fn().mockImplementation(() =>
      Promise.resolve(
        Response.json({
          query: "rust",
          total_moments: 0,
          total_videos: 0,
          totals_approximate: false,
          total_found: 0,
          out_of: 0,
          time_to_first_result_ms: 0,
          page: 1,
          per_page: 12,
          sort: "relevance",
          mode: "lexical",
          has_next_page: false,
          hits: [],
          videos: [],
        }),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    await runSearchTranscripts("", { query: "rust", page: 1, sort: "relevance" });
    await runSearchVideoMoments("", "rust", "video-1");

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({ cache: "no-store" });
    expect(fetchMock.mock.calls[1]?.[1]).toMatchObject({ cache: "no-store" });
  });

  it("captures only the narrow anonymous analytics contract", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    await Effect.runPromise(
      captureAnalyticsEventEffect(
        { apiKey: "phc_project", apiHost: "https://analytics.example.com/" },
        "search_submitted",
        { query_length: 4, omitted: undefined },
      ),
    );

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0]!;
    expect(url).toBe("https://analytics.example.com/batch/");
    const payload = JSON.parse(String(init?.body));
    expect(payload.api_key).toBe("phc_project");
    expect(payload.batch[0]).toMatchObject({
      event: "search_submitted",
      properties: {
        query_length: 4,
        $lib: "ytsearch",
        $process_person_profile: false,
        $is_identified: false,
      },
    });
    expect(JSON.stringify(payload)).not.toContain("omitted");
  });
});
