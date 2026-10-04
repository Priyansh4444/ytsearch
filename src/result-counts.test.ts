import { describe, expect, test } from "vitest";
import type { SearchResponse, SearchVideoResult } from "./types";
import { searchResultCountLabel } from "./result-counts";

const video = (id: string, moments: number): SearchVideoResult => ({
  video_id: id,
  title: id,
  youtuber_handle: "@test",
  youtuber_name: "Test",
  content_kind: "video",
  match_count: moments,
  youtube_url: `https://youtube.test/watch?v=${id}`,
  embed_url: `https://youtube.test/embed/${id}`,
  hits: Array.from({ length: moments }, (_, index) => ({
    id: `${id}-${index}`,
    video_id: id,
    title: id,
    text: "match",
    match_kind: "transcript",
    youtuber_handle: "@test",
    youtuber_name: "Test",
    content_kind: "video",
    t_start_ms: index * 30_000,
    youtube_url: `https://youtube.test/watch?v=${id}`,
    embed_url: `https://youtube.test/embed/${id}`,
  })),
});

type CountedSearchResponse = SearchResponse & { totals_approximate?: boolean };

const response = (overrides: Partial<CountedSearchResponse> = {}): CountedSearchResponse => ({
  query: "rust",
  total_moments: 41,
  total_videos: 15,
  totals_approximate: false,
  total_found: 41,
  out_of: 2_000,
  time_to_first_result_ms: 2,
  page: 1,
  per_page: 12,
  sort: "relevance",
  mode: "lexical",
  has_next_page: true,
  hits: [],
  videos: [video("a", 3), video("b", 2)],
  ...overrides,
});

describe("searchResultCountLabel", () => {
  test("uses authoritative totals across pagination and every returned video", () => {
    expect(searchResultCountLabel(response())).toBe("2 shown of 15 videos · 5 shown of 41 moments");
  });

  test("does not mistake the duplicate top-level hit list for displayed moments", () => {
    expect(
      searchResultCountLabel(
        response({
          total_moments: 5,
          total_videos: 2,
          has_next_page: false,
          hits: [video("duplicate", 1).hits[0]],
        }),
      ),
    ).toBe("2 videos · 5 moments");
  });

  test("labels legacy approximate totals without claiming a complete moment count", () => {
    expect(searchResultCountLabel(response({ totals_approximate: true }))).toBe(
      "Approximately 2 shown of 15 videos · 5 moments shown",
    );
  });
});
