import { beforeEach, describe, expect, it, vi } from "vitest";
import { runCopyToClipboard, runSearchTranscripts } from "./client-runtime";

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
  hits: [],
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
