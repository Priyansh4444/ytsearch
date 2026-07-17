import { beforeEach, describe, expect, test, vi } from "vitest";
import type { SearchResponse } from "./types";

const emptySearch = (query = "rust"): SearchResponse => ({
  query,
  total_moments: 0,
  total_videos: 0,
  total_found: 0,
  out_of: 0,
  time_to_first_result_ms: 1,
  page: 1,
  per_page: 12,
  sort: "relevance",
  mode: "lexical",
  has_next_page: false,
  hits: [],
  videos: [],
});

describe("frontend API client", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.restoreAllMocks();
  });

  test("encodes the complete search contract", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(emptySearch("rust & wasm")), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { searchTranscripts } = await import("./api");

    await searchTranscripts({
      query: "rust & wasm",
      youtuber: "@example",
      page: 2,
      sort: "published_desc",
      mode: "lexical",
    });

    const url = new URL(String(fetchMock.mock.calls[0]?.[0]), "http://localhost");
    expect(Object.fromEntries(url.searchParams)).toEqual({
      q: "rust & wasm",
      per_page: "12",
      page: "2",
      sort: "published_desc",
      mode: "lexical",
      youtuber: "@example",
    });
  });

  test("retries one transient gateway failure and caches the successful result", async () => {
    const response = emptySearch();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response("unavailable", { status: 503 }))
      .mockResolvedValueOnce(new Response(JSON.stringify(response), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);
    const { searchTranscripts } = await import("./api");
    const request = {
      query: "rust",
      youtuber: "",
      page: 1,
      sort: "relevance" as const,
      mode: "lexical" as const,
    };

    await expect(searchTranscripts(request)).resolves.toEqual(response);
    await expect(searchTranscripts(request)).resolves.toEqual(response);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  test("surfaces backend JSON errors to the interface", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ error: "invalid video id" }), {
          status: 400,
          headers: { "Content-Type": "application/json" },
        }),
      ),
    );
    const { searchVideoMoments } = await import("./api");

    await expect(searchVideoMoments("query", "bad/id")).rejects.toThrow("invalid video id");
  });

  test("rejects malformed successful JSON with actionable context", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("not-json", { status: 200 })));
    const { indexedChannels } = await import("./api");

    await expect(indexedChannels()).rejects.toThrow(
      "Channel list returned an invalid response (HTTP 200)",
    );
  });
});
