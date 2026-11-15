import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  runCopyToClipboard,
  runDashboardData,
  runRequestIndex,
  runSearchTranscripts,
  runSearchVideoMoments,
} from "./client-runtime";

const emptySearchResponse = {
  query: "rust",
  total_moments: 0,
  total_videos: 0,
  total_found: 0,
  out_of: 0,
  time_to_first_result_ms: 4,
  page: 1,
  per_page: 12,
  sort: "relevance",
  mode: "lexical",
  has_next_page: false,
  videos: [],
};

describe("browser client runtime", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("validates search responses and normalizes the legacy approximation field", async () => {
    const fetchMock = vi.fn().mockResolvedValue(Response.json(emptySearchResponse));
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      runSearchTranscripts("https://api.example.com", {
        query: "rust",
        youtuber: "",
        page: 1,
        sort: "relevance",
      }),
    ).resolves.toMatchObject({ query: "rust", totals_approximate: false });
    expect(fetchMock).toHaveBeenCalledWith(
      "https://api.example.com/api/search?q=rust&per_page=12&page=1&sort=relevance",
      expect.objectContaining({ cache: "no-store" }),
    );
  });

  it.each([
    ["lexical", ""],
    ["hybrid", "&mode=hybrid"],
  ] as const)("requests %s search with the URL its edge cache expects", async (mode, suffix) => {
    const fetchMock = vi.fn().mockResolvedValue(Response.json(emptySearchResponse));
    vi.stubGlobal("fetch", fetchMock);
    await runSearchTranscripts("", {
      query: "rust",
      youtuber: "",
      page: 1,
      sort: "relevance",
      mode,
    });
    expect(fetchMock).toHaveBeenCalledWith(
      `/api/search?q=rust&per_page=12&page=1&sort=relevance${suffix}`,
      expect.anything(),
    );
  });

  it("asks for a video's moments with the engine that found the video", async () => {
    const fetchMock = vi
      .fn()
      .mockImplementation(() => Promise.resolve(Response.json(emptySearchResponse)));
    vi.stubGlobal("fetch", fetchMock);
    await runSearchVideoMoments("", "rust", "video-1", "hybrid");
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/search?q=rust&video_id=video-1&per_page=100&mode=hybrid",
    );
    await runSearchVideoMoments("", "rust", "video-1", "lexical");
    expect(fetchMock.mock.calls[1]?.[0]).toBe("/api/search?q=rust&video_id=video-1&per_page=100");
  });

  it("accepts the hybrid response mode", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(Response.json({ ...emptySearchResponse, mode: "hybrid" })),
    );
    await expect(
      runSearchTranscripts("", { query: "why rust?", youtuber: "", page: 1, sort: "relevance" }),
    ).resolves.toMatchObject({ mode: "hybrid" });
  });

  it("accepts the opt-in semantic response mode", async () => {
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          Response.json({ ...emptySearchResponse, mode: "semantic", totals_approximate: true }),
        ),
    );
    await expect(
      runSearchTranscripts("", { query: "rust", youtuber: "", page: 1, sort: "relevance" }),
    ).resolves.toMatchObject({ mode: "semantic", totals_approximate: true });
  });

  it("accepts responses from API images that still send the duplicate top-level hits", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(Response.json({ ...emptySearchResponse, hits: [{}] })),
    );

    const response = await runSearchTranscripts("", {
      query: "rust",
      youtuber: "",
      page: 1,
      sort: "relevance",
    });
    expect(response).not.toHaveProperty("hits");
    expect(response.videos).toEqual([]);
  });

  it("rejects a successful response that violates the wire contract", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(Response.json({ ...emptySearchResponse, total_videos: "zero" })),
    );

    await expect(
      runSearchTranscripts("", {
        query: "rust",
        youtuber: "",
        page: 1,
        sort: "relevance",
      }),
    ).rejects.toThrow("Search returned an invalid response (HTTP 200)");
  });

  // Payloads as the production API serializes them (absent options are null).
  it("accepts an index request whose optional note is null", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        Response.json({
          request: {
            id: "req-1",
            handle: "@t3dotgg",
            note: null,
            requested_at: 1790793038,
            source_backend: "raw",
            status: "pending",
          },
        }),
      ),
    );

    await expect(runRequestIndex("https://api.example.com", "@t3dotgg")).resolves.toEqual({
      id: "req-1",
      handle: "@t3dotgg",
      note: undefined,
      requested_at: 1790793038,
      source_backend: "raw",
      status: "pending",
    });
  });

  it("reads an idle index status with null fields on the dashboard", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) =>
        Promise.resolve(
          url.endsWith("/api/health")
            ? Response.json({ ok: true })
            : url.endsWith("/api/youtubers")
              ? Response.json({ channels: [] })
              : Response.json({
                  channel: null,
                  handle: "",
                  request_id: null,
                  status: "idle",
                  target_backend: null,
                }),
        ),
      ),
    );

    const dashboard = await runDashboardData("https://api.example.com");
    expect(dashboard.errors).toEqual([]);
    expect(dashboard.indexStatus).toEqual({
      handle: "",
      status: "idle",
      channel: undefined,
      request_id: undefined,
      target_backend: undefined,
    });
  });

  it("uses the platform clipboard without loading a framework runtime", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });

    await runCopyToClipboard("https://example.com/result");
    expect(writeText).toHaveBeenCalledWith("https://example.com/result");
  });
});
