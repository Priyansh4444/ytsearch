import { beforeEach, describe, expect, test, vi } from "vitest";
import type { SearchResponse } from "./types";

const emptySearch = (query = "rust"): SearchResponse => ({
  query,
  total_moments: 0,
  total_videos: 0,
  totals_approximate: false,
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
    vi.useRealTimers();
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
    });

    const url = new URL(String(fetchMock.mock.calls[0]?.[0]), "http://localhost");
    expect(Object.fromEntries(url.searchParams)).toEqual({
      q: "rust & wasm",
      per_page: "12",
      page: "2",
      sort: "published_desc",
      youtuber: "@example",
    });
  });

  test("accepts search responses from backends that omit legacy approximate totals", async () => {
    const response = { ...emptySearch() };
    delete (response as Partial<SearchResponse>).totals_approximate;
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json(response)));
    const { searchTranscripts } = await import("./api");

    await expect(
      searchTranscripts({
        query: "rust",
        youtuber: "",
        page: 1,
        sort: "relevance",
      }),
    ).resolves.toMatchObject({ totals_approximate: false });
  });

  test("accepts null context at transcript boundaries from older API images", async () => {
    const boundaryHit = {
      id: "cursor-video_0",
      video_id: "cursor-video",
      title: "Cursor",
      text: "Cursor transcript boundary",
      prev_text: null,
      next_text: null,
      match_kind: "transcript" as const,
      youtuber_handle: "@cursor_ai",
      youtuber_name: "Cursor",
      content_kind: "video" as const,
      t_start_ms: 0,
      youtube_url: "https://www.youtube.com/watch?v=cursor-video&t=0s",
      embed_url: "https://www.youtube-nocookie.com/embed/cursor-video?start=0&rel=0",
    };
    const response = {
      ...emptySearch("curspr"),
      total_moments: 1,
      total_videos: 1,
      total_found: 1,
      hits: [boundaryHit],
      videos: [
        {
          video_id: "cursor-video",
          title: "Cursor",
          youtuber_handle: "@cursor_ai",
          youtuber_name: "Cursor",
          content_kind: "video",
          match_count: 1,
          youtube_url: boundaryHit.youtube_url,
          embed_url: boundaryHit.embed_url,
          hits: [boundaryHit],
        },
      ],
    };
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json(response)));
    const { searchTranscripts } = await import("./api");

    await expect(
      searchTranscripts({
        query: "curspr",
        youtuber: "@cursor_ai",
        page: 1,
        sort: "relevance",
      }),
    ).resolves.toMatchObject({
      hits: [{ prev_text: null, next_text: null }],
    });
  });

  test("retries one transient gateway failure", async () => {
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
    };

    await expect(searchTranscripts(request)).resolves.toMatchObject(response);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  test("coalesces the fetch while measuring each caller's own wait", async () => {
    vi.useFakeTimers();
    let resolveFetch: ((response: Response) => void) | undefined;
    const fetchMock = vi.fn(
      () =>
        new Promise<Response>((resolve) => {
          resolveFetch = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { searchTranscripts } = await import("./api");
    const request = {
      query: "rust",
      youtuber: "",
      page: 1,
      sort: "relevance" as const,
    };

    const first = searchTranscripts(request);
    await vi.dynamicImportSettled();
    vi.advanceTimersByTime(250);
    const second = searchTranscripts(request);
    expect(first).not.toBe(second);
    expect(fetchMock).toHaveBeenCalledOnce();

    vi.advanceTimersByTime(50);
    resolveFetch?.(Response.json(emptySearch()));
    await expect(first).resolves.toMatchObject({ request_duration_ms: 300 });
    await expect(second).resolves.toMatchObject({ request_duration_ms: 50 });

    const third = searchTranscripts(request);
    await vi.dynamicImportSettled();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    vi.advanceTimersByTime(20);
    resolveFetch?.(Response.json(emptySearch()));
    await expect(third).resolves.toMatchObject({ request_duration_ms: 20 });
  });

  test("isolates cancellation between callers sharing one fetch", async () => {
    let resolveFetch: ((response: Response) => void) | undefined;
    const fetchMock = vi.fn(
      () =>
        new Promise<Response>((resolve) => {
          resolveFetch = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { searchTranscripts } = await import("./api");
    const request = {
      query: "rust",
      youtuber: "",
      page: 1,
      sort: "relevance" as const,
    };
    const firstController = new AbortController();
    const secondController = new AbortController();

    const first = searchTranscripts(request, firstController.signal);
    const second = searchTranscripts(request, secondController.signal);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    firstController.abort();
    await expect(first).rejects.toMatchObject({ name: "AbortError" });

    resolveFetch?.(Response.json(emptySearch()));
    await expect(second).resolves.toMatchObject(emptySearch());
    const sharedSignal = fetchMock.mock.calls[0]?.[1]?.signal as AbortSignal;
    expect(sharedSignal.aborted).toBe(false);
  });

  test("aborts shared work only after every caller cancels", async () => {
    const fetchMock = vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
      return new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => reject(init.signal?.reason), { once: true });
      });
    });
    vi.stubGlobal("fetch", fetchMock);
    const { searchTranscripts } = await import("./api");
    const request = {
      query: "rust",
      youtuber: "",
      page: 1,
      sort: "relevance" as const,
    };
    const firstController = new AbortController();
    const secondController = new AbortController();
    const first = searchTranscripts(request, firstController.signal);
    const second = searchTranscripts(request, secondController.signal);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    secondController.abort();
    await expect(second).rejects.toMatchObject({ name: "AbortError" });
    const sharedSignal = fetchMock.mock.calls[0]?.[1]?.signal;
    expect(sharedSignal).toBeInstanceOf(AbortSignal);
    expect(sharedSignal?.aborted).toBe(false);
    firstController.abort();
    await expect(first).rejects.toMatchObject({ name: "AbortError" });
    expect(sharedSignal?.aborted).toBe(true);
  });

  test("does not start shared work for an already-aborted caller", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const { searchTranscripts } = await import("./api");
    const controller = new AbortController();
    controller.abort();

    await expect(
      searchTranscripts(
        { query: "rust", youtuber: "", page: 1, sort: "relevance" },
        controller.signal,
      ),
    ).rejects.toMatchObject({ name: "AbortError" });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("coalesces concurrent indexed-channel refreshes", async () => {
    let resolveFetch: ((response: Response) => void) | undefined;
    const fetchMock = vi.fn(
      () =>
        new Promise<Response>((resolve) => {
          resolveFetch = resolve;
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { indexedChannels } = await import("./api");

    const first = indexedChannels();
    const second = indexedChannels();
    expect(first).toBe(second);
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    resolveFetch?.(Response.json({ channels: [] }));
    await expect(first).resolves.toEqual([]);
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

  test("does not retry a structured scheduled-offline response", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          code: "scheduled_offline",
          error: "Search returns at noon.",
          available_at: "2026-07-27T19:00:00.000Z",
        }),
        { status: 503 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    const { searchTranscripts } = await import("./api");

    await expect(
      searchTranscripts({
        query: "rust",
        youtuber: "",
        page: 1,
        sort: "relevance",
      }),
    ).rejects.toMatchObject({
      code: "scheduled_offline",
      availableAt: "2026-07-27T19:00:00.000Z",
    });
    expect(fetchMock).toHaveBeenCalledOnce();
  });

  test("rejects malformed successful JSON with actionable context", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("not-json", { status: 200 })));
    const { indexedChannels } = await import("./api");

    await expect(indexedChannels()).rejects.toThrow(
      "Channel list returned an invalid response (HTTP 200)",
    );
  });

  test("rejects well-formed JSON that violates the response schema", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ channels: [{ id: 42 }] })));
    const { indexedChannels } = await import("./api");

    await expect(indexedChannels()).rejects.toThrow(
      "Channel list returned an invalid response (HTTP 200)",
    );
  });
});
