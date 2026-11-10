import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, test, vi } from "vitest";
import type { SearchHit, SearchResponse, SearchVideoResult } from "../types";
import { SearchResults } from "./search-results";

vi.mock("../analytics", () => ({ capture: vi.fn() }));
vi.mock("../api", () => ({ searchVideoMoments: vi.fn() }));
vi.mock("../client-runtime-loader", () => ({
  loadClientRuntime: () => Promise.resolve({ runCopyToClipboard: vi.fn() }),
}));

const hit = (id: string): SearchHit => ({
  id,
  video_id: id,
  title: "Test video",
  text: `Moment for ${id}`,
  match_kind: "transcript",
  youtuber_handle: "@creator",
  youtuber_name: "Creator",
  content_kind: "video",
  t_start_ms: 0,
  timestamp: "0:00",
  youtube_url: `https://youtube.test/watch?v=${id}`,
  embed_url: `https://youtube.test/embed/${id}`,
});

const video = (
  videoId: string,
  content_kind: SearchVideoResult["content_kind"],
): SearchVideoResult => ({
  video_id: videoId,
  title: `Title ${videoId}`,
  youtuber_handle: "@creator",
  youtuber_name: "Creator",
  content_kind,
  match_count: 1,
  youtube_url: `https://youtube.test/watch?v=${videoId}`,
  embed_url: `https://youtube.test/embed/${videoId}`,
  hits: [hit(videoId)],
});

const response = (videos: SearchVideoResult[]): SearchResponse => ({
  query: "moment",
  total_moments: videos.length,
  total_videos: videos.length,
  totals_approximate: false,
  total_found: videos.length,
  out_of: videos.length,
  time_to_first_result_ms: 5,
  page: 1,
  per_page: 10,
  sort: "relevance",
  mode: "lexical",
  has_next_page: false,
  videos,
});

const renderedCardIds = () => Array.from(document.querySelectorAll("article"), (card) => card.id);

describe("SearchResults sections", () => {
  test("lists every video before any unknown, keeping shorts separate", () => {
    render(() => (
      <SearchResults
        response={response([
          video("v1", "video"),
          video("u1", "unknown"),
          video("v2", "video"),
          video("s1", "short"),
          video("u2", "unknown"),
        ])}
        youtuber=""
        page={1}
        sort="relevance"
        onPageChange={() => undefined}
        onSortChange={() => undefined}
      />
    ));

    // The old `[...filter("video"), ...filter("unknown")]` concatenation put
    // every unknown after every video; a single shared bucket would interleave
    // them and shift the per-section rank values.
    expect(renderedCardIds()).toEqual(["video-v1", "video-v2", "video-u1", "video-u2", "video-s1"]);
    expect(screen.getByRole("heading", { name: /^Videos · 4$/ })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /^Shorts · 1$/ })).toBeInTheDocument();
  });

  test("keeps the Videos section for unknown-only pages and omits an empty Shorts section", () => {
    render(() => (
      <SearchResults
        response={response([video("u1", "unknown")])}
        youtuber=""
        page={1}
        sort="relevance"
        onPageChange={() => undefined}
        onSortChange={() => undefined}
      />
    ));

    expect(renderedCardIds()).toEqual(["video-u1"]);
    expect(screen.getByRole("heading", { name: /^Videos · 1$/ })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /^Shorts/ })).not.toBeInTheDocument();
  });

  test("renders only shorts when the page has no long-form videos", () => {
    render(() => (
      <SearchResults
        response={response([video("s1", "short")])}
        youtuber=""
        page={1}
        sort="relevance"
        onPageChange={() => undefined}
        onSortChange={() => undefined}
      />
    ));

    expect(renderedCardIds()).toEqual(["video-s1"]);
    expect(screen.queryByRole("heading", { name: /^Videos/ })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: /^Shorts · 1$/ })).toBeInTheDocument();
  });

  test("shows the empty state for a page with no videos at all", () => {
    render(() => (
      <SearchResults response={response([])} youtuber="" page={1} onPageChange={() => undefined} />
    ));

    expect(renderedCardIds()).toEqual([]);
  });
});

describe("search timing", () => {
  test("labels an edge-cached response instead of repeating its old search time", () => {
    render(() => (
      <SearchResults
        response={{ ...response([video("a", "video")]), request_duration_ms: 4, edge_cached: true }}
        page={1}
        sort="relevance"
        onPageChange={() => undefined}
        onSortChange={() => undefined}
      />
    ));
    expect(screen.getByText(/4 ms request · cached/)).toBeTruthy();
    expect(screen.queryByText(/5 ms search/)).toBeNull();
  });
});

describe("hybrid fallback", () => {
  const results = (
    requestedMode: "lexical" | "hybrid",
    answered: SearchResponse["mode"],
    sort: "relevance" | "published_desc",
    onRetry = vi.fn(),
  ) =>
    render(() => (
      <SearchResults
        response={{ ...response([video("a", "video")]), mode: answered }}
        youtuber=""
        page={1}
        sort={sort}
        requestedMode={requestedMode}
        onPageChange={() => undefined}
        onSortChange={() => undefined}
        onRetry={onRetry}
      />
    ));

  test("a hybrid search answered by keywords says so and offers another try", () => {
    const onRetry = vi.fn();
    results("hybrid", "lexical", "relevance", onRetry);
    expect(screen.getByTestId("mode-fallback")).toHaveTextContent("these are keyword results");
    fireEvent.click(screen.getByRole("button", { name: "Try hybrid again" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });

  test("a sort hybrid cannot serve explains that, with nothing to retry", () => {
    results("hybrid", "lexical", "published_desc");
    expect(screen.getByTestId("mode-fallback")).toHaveTextContent(
      "only works with the Relevant order",
    );
    expect(screen.queryByRole("button", { name: "Try hybrid again" })).toBeNull();
  });

  test.each([
    ["hybrid", "hybrid"],
    ["lexical", "lexical"],
  ] as const)("no note when %s was asked and %s answered", (asked, answered) => {
    results(asked, answered, "relevance");
    expect(screen.queryByTestId("mode-fallback")).toBeNull();
  });
});
