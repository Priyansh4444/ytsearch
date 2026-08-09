import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { beforeEach, describe, expect, test, vi } from "vitest";
import type { IndexedChannel, SearchAvailability, SearchResponse, SearchRequest } from "./types";

const mocks = vi.hoisted(() => ({
  fetchSearchAvailability: vi.fn<() => Promise<SearchAvailability>>(),
  indexStatus: vi.fn(),
  indexedChannels: vi.fn<() => Promise<IndexedChannel[]>>(),
  requestIndex: vi.fn(),
  searchTranscripts: vi.fn<(request: SearchRequest) => Promise<SearchResponse>>(),
  capture: vi.fn(),
  captureSearchLoaded: vi.fn(),
  captureSearchSubmitted: vi.fn(),
  initializeAnalytics: vi.fn(),
}));

vi.mock("./api", () => ({
  API_BASE_URL: "https://api.example.test",
  fetchSearchAvailability: mocks.fetchSearchAvailability,
  indexStatus: mocks.indexStatus,
  indexedChannels: mocks.indexedChannels,
  requestIndex: mocks.requestIndex,
  searchTranscripts: mocks.searchTranscripts,
}));
vi.mock("./analytics", () => ({
  capture: mocks.capture,
  captureSearchLoaded: mocks.captureSearchLoaded,
  captureSearchSubmitted: mocks.captureSearchSubmitted,
  initializeAnalytics: mocks.initializeAnalytics,
}));
vi.mock("../shared/service-schedule", () => ({
  evaluateSearchSchedule: () => ({
    phase: "open",
    nextTransitionAt: "2026-07-31T07:00:00.000Z",
  }),
  nextPublicOpening: () => "2026-07-31T19:00:00.000Z",
}));

import App from "./app";

const channel: IndexedChannel = {
  id: "channel-1",
  handle: "@example",
  name: "Example",
  channel_id: "UC0000000000000000000001",
  public_video_count: 10,
  indexed_video_count: 8,
  indexed_chunk_count: 80,
  avatar_url: "https://yt3.ggpht.com/example=s800",
};

const ready: SearchAvailability = {
  status: "ready",
  search_enabled: true,
  enforced: true,
  schedule_phase: "open",
  time_zone: "America/Los_Angeles",
  daily_window: {
    opens: "12:00",
    closes: "00:00",
    label: "12:00 PM–12:00 AM Pacific Time",
  },
  next_transition_at: "2026-07-31T07:00:00.000Z",
};

const response: SearchResponse = {
  query: "rust",
  total_moments: 1,
  total_videos: 1,
  totals_approximate: false,
  total_found: 1,
  out_of: 80,
  time_to_first_result_ms: 4,
  page: 1,
  per_page: 12,
  sort: "relevance",
  mode: "lexical",
  has_next_page: false,
  hits: [],
  videos: [
    {
      video_id: "video-1",
      title: "Rust ownership explained",
      youtuber_handle: "@example",
      youtuber_name: "Example",
      published_at: "2026-07-30T00:00:00Z",
      content_kind: "video",
      match_count: 1,
      youtube_url: "https://youtube.com/watch?v=video-1&t=1s",
      embed_url: "https://youtube.com/embed/video-1",
      best_timestamp: "0:01",
      hits: [
        {
          id: "hit-1",
          video_id: "video-1",
          title: "Rust ownership explained",
          text: "Ownership keeps Rust memory safe.",
          text_highlight: "Ownership keeps <mark>Rust</mark> memory safe.",
          match_kind: "transcript",
          youtuber_handle: "@example",
          youtuber_name: "Example",
          published_at: "2026-07-30T00:00:00Z",
          content_kind: "video",
          t_start_ms: 1_000,
          timestamp: "0:01",
          youtube_url: "https://youtube.com/watch?v=video-1&t=1s",
          embed_url: "https://youtube.com/embed/video-1",
        },
      ],
    },
  ],
};

describe("App", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/");
    mocks.fetchSearchAvailability.mockReset().mockResolvedValue(ready);
    mocks.indexStatus
      .mockReset()
      .mockImplementation((handle?: string) =>
        Promise.resolve(
          handle === "@newcreator"
            ? { handle, status: "pending", request_id: "request-1" }
            : { handle: handle ?? "", status: "idle" },
        ),
      );
    mocks.indexedChannels.mockReset().mockResolvedValue([channel]);
    mocks.requestIndex.mockReset().mockResolvedValue({
      id: "request-1",
      handle: "@newcreator",
      requested_at: 1,
      source_backend: "raw",
      status: "pending",
    });
    mocks.searchTranscripts.mockReset().mockResolvedValue(response);
    mocks.capture.mockReset();
    mocks.captureSearchLoaded.mockReset();
    mocks.captureSearchSubmitted.mockReset();
    mocks.initializeAnalytics.mockReset();
  });

  test("loads channels and opens a creator from the home swarm", async () => {
    render(() => <App />);

    expect(await screen.findByRole("link", { name: "Search Example" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: "Search Example" }));
    expect(window.location.pathname).toBe("/example");
    expect(
      screen.getByText(
        (content, element) => element?.tagName === "P" && content.includes("@example"),
      ),
    ).toBeInTheDocument();
    expect(mocks.capture).toHaveBeenCalledWith("channel_opened", {
      handle: "@example",
      source: "channel_picker",
    });
  });

  test("submits a transcript search and renders timestamped results", async () => {
    render(() => <App />);

    const input = screen.getByRole("combobox", { name: "Search transcripts" });
    await waitFor(() => expect(screen.getByText(/Search is available now/)).toBeInTheDocument());
    fireEvent.input(input, { target: { value: "rust" } });
    fireEvent.submit(input.closest("form")!);

    expect(
      await screen.findByRole("link", { name: /Rust ownership explained/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Ownership keeps Rust memory safe/i }),
    ).toBeInTheDocument();
    expect(mocks.searchTranscripts).toHaveBeenCalledWith(
      {
        query: "rust",
        youtuber: "",
        page: 1,
        sort: "relevance",
      },
      expect.any(AbortSignal),
    );
    await waitFor(() => expect(mocks.captureSearchLoaded).toHaveBeenCalled());
  });

  test("submits an indexing request only after confirmation", async () => {
    render(() => <App />);
    const input = screen.getByRole("combobox", { name: "Search transcripts" });
    fireEvent.input(input, { target: { value: "@newcreator" } });
    fireEvent.submit(input.closest("form")!);

    expect(await screen.findByRole("dialog")).toHaveTextContent("@newcreator");
    expect(mocks.requestIndex).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Submit request" }));

    await waitFor(() => expect(mocks.requestIndex).toHaveBeenCalledWith("@newcreator"));
    expect(await screen.findByText("Awaiting approval")).toBeInTheDocument();
    expect(
      screen.getByText(/request-1 is waiting for administrator approval/i),
    ).toBeInTheDocument();
  });
});
