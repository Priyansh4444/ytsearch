import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { describe, expect, test, vi } from "vitest";
import type { SearchHit, SearchVideoResult } from "../types";
import { VideoResult } from "./video-result";

const { capture, searchVideoMoments, runCopyToClipboard } = vi.hoisted(() => ({
  capture: vi.fn(),
  searchVideoMoments: vi.fn(),
  runCopyToClipboard: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../analytics", () => ({ capture }));
vi.mock("../api", () => ({ searchVideoMoments }));
vi.mock("../client-runtime-loader", () => ({
  loadClientRuntime: () => Promise.resolve({ runCopyToClipboard }),
}));

const hits: SearchHit[] = Array.from({ length: 7 }, (_, index) => ({
  id: `hit-${index}`,
  video_id: "video-1",
  title: "Test video",
  text: `Moment ${index + 1}`,
  match_kind: "transcript",
  youtuber_handle: "@creator",
  youtuber_name: "Creator",
  content_kind: "video",
  t_start_ms: index * 1_000,
  timestamp: `0:0${index}`,
  youtube_url: `https://youtube.test/watch?v=video-1&t=${index}`,
  embed_url: `https://youtube.test/embed/video-1?start=${index}`,
}));

const video: SearchVideoResult = {
  video_id: "video-1",
  title: "Test video",
  youtuber_handle: "@creator",
  youtuber_name: "Creator",
  content_kind: "video",
  match_count: hits.length,
  thumbnail: "https://i.ytimg.com/vi/video-1/maxresdefault.jpg",
  youtube_url: "https://youtube.test/watch?v=video-1",
  embed_url: "https://youtube.test/embed/video-1",
  hits,
};

describe("VideoResult", () => {
  test("defers the YouTube player until the poster is activated", async () => {
    render(() => <VideoResult video={video} query="moment" rank={1} />);

    expect(screen.queryByTitle("YouTube player for Test video")).not.toBeInTheDocument();
    expect(document.querySelector("img")).toHaveAttribute(
      "src",
      "https://i.ytimg.com/vi/video-1/hqdefault.jpg",
    );
    fireEvent.click(screen.getByRole("button", { name: "Play Test video at 0:00" }));
    expect(await screen.findByTitle("YouTube player for Test video")).toHaveAttribute(
      "src",
      "https://youtube.test/embed/video-1?start=0&autoplay=1",
    );
  });

  test("loads and reveals every transcript moment", async () => {
    searchVideoMoments.mockResolvedValue({ videos: [video] });
    render(() => <VideoResult video={video} query="moment" rank={1} />);

    expect(screen.getAllByRole("button", { name: /Moment \d/ })).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Show all moments" }));
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: /Moment \d/ })).toHaveLength(7),
    );
    expect(searchVideoMoments).toHaveBeenCalledWith("moment", "video-1", expect.any(AbortSignal));
    expect(screen.queryByRole("button", { name: "Show all moments" })).not.toBeInTheDocument();
  });

  test("does not offer to load moments when every match is already visible", () => {
    const completeVideo = { ...video, match_count: 3, hits: hits.slice(0, 3) };

    render(() => <VideoResult video={completeVideo} query="moment" rank={1} />);

    expect(screen.queryByRole("button", { name: "Show all moments" })).not.toBeInTheDocument();
  });

  test("offers to load moments when the backend count exceeds returned snippets", () => {
    const partialVideo = { ...video, match_count: 4, hits: hits.slice(0, 2) };

    render(() => <VideoResult video={partialVideo} query="moment" rank={1} />);

    expect(screen.getByRole("button", { name: "Show all moments" })).toBeInTheDocument();
  });

  test("offers to load moments when nearby snippets collapse below the backend count", () => {
    const coalescedVideo = { ...video, match_count: 3, hits: hits.slice(0, 1) };

    render(() => <VideoResult video={coalescedVideo} query="moment" rank={1} />);

    expect(screen.getByRole("button", { name: "Show all moments" })).toBeInTheDocument();
  });

  test("renders every backend-provided whole-token highlight", () => {
    const highlightedHit = {
      ...hits[0],
      text: "includes things like CSS which I think is dead now",
      text_highlight:
        "includes things like <mark>CSS</mark> <mark>which</mark> <mark>I</mark> <mark>think</mark> <mark>is</mark> <mark>dead</mark> <mark>now</mark>",
    };
    const highlightedVideo = {
      ...video,
      match_count: 1,
      hits: [highlightedHit],
    };

    render(() => (
      <VideoResult video={highlightedVideo} query="Css which i think is dead now" rank={1} />
    ));

    expect(Array.from(document.querySelectorAll("mark"), (mark) => mark.textContent)).toEqual([
      "CSS",
      "which",
      "I",
      "think",
      "is",
      "dead",
      "now",
    ]);
  });

  test("records feedback against the selected moment", () => {
    render(() => <VideoResult video={video} query="moment" rank={2} />);

    fireEvent.click(screen.getAllByRole("button", { name: "Mark moment useful" })[0]);
    expect(capture).toHaveBeenCalledWith(
      "result_moment_feedback",
      expect.objectContaining({ moment_id: "hit-0", rank: 2, value: "useful" }),
    );
  });

  test("selects the clicked moment and points the player at it", async () => {
    render(() => <VideoResult video={video} query="moment" rank={1} />);

    fireEvent.click(screen.getByRole("button", { name: /Moment 3/ }));

    expect(await screen.findByTitle("YouTube player for Test video")).toHaveAttribute(
      "src",
      "https://youtube.test/embed/video-1?start=2&autoplay=1",
    );
    expect(screen.getByText("0:02")).toBeInTheDocument();
    // The selection event carries the video-level payload plus the match kind
    // (there is no per-hit id in the payload shape).
    expect(capture).toHaveBeenCalledWith(
      "result_timestamp_selected",
      expect.objectContaining({ video_id: "video-1", rank: 1, match_kind: "transcript" }),
    );
  });

  test("falls back to the first visible moment when the selected id disappears", async () => {
    searchVideoMoments.mockResolvedValue({
      videos: [
        { ...video, hits: hits.slice(3).map((entry) => ({ ...entry, id: `other-${entry.id}` })) },
      ],
    });
    render(() => <VideoResult video={video} query="moment" rank={1} />);

    fireEvent.click(screen.getByRole("button", { name: /Moment 2/ }));
    fireEvent.click(screen.getByRole("button", { name: "Show all moments" }));

    // The expansion replaced every hit id, so the stale selection must resolve
    // to the first visible hit rather than to nothing.
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: /Moment \d/ })).toHaveLength(4),
    );
    expect(screen.getByText("0:03")).toBeInTheDocument();
  });

  test("forces autoplay on when the embed URL already sets it", async () => {
    render(() => (
      <VideoResult
        video={{
          ...video,
          // `playerUrl` prefers the selected hit's embed URL, so drop the hits
          // to exercise the video-level URL under test.
          hits: [],
          match_count: 0,
          embed_url: "https://www.youtube.com/embed/video-1?autoplay=0&rel=0",
        }}
        query="moment"
        rank={1}
      />
    ));

    fireEvent.click(screen.getByRole("button", { name: /Play Test video/ }));

    expect(await screen.findByTitle("YouTube player for Test video")).toHaveAttribute(
      "src",
      "https://www.youtube.com/embed/video-1?autoplay=1&rel=0",
    );
  });

  test("keeps an existing fragment after the autoplay parameter", async () => {
    render(() => (
      <VideoResult
        video={{
          ...video,
          hits: [],
          match_count: 0,
          embed_url: "https://www.youtube.com/embed/video-1#t=30",
        }}
        query="moment"
        rank={1}
      />
    ));

    fireEvent.click(screen.getByRole("button", { name: /Play Test video/ }));

    expect(await screen.findByTitle("YouTube player for Test video")).toHaveAttribute(
      "src",
      "https://www.youtube.com/embed/video-1?autoplay=1#t=30",
    );
  });

  test("aborts an in-flight moments request when the row unmounts", async () => {
    searchVideoMoments.mockReset();
    let signal: AbortSignal | undefined;
    searchVideoMoments.mockImplementation(
      (_query: string, _videoId: string, passed: AbortSignal) => {
        signal = passed;
        return new Promise(() => undefined);
      },
    );
    const { unmount } = render(() => <VideoResult video={video} query="moment" rank={1} />);

    fireEvent.click(screen.getByRole("button", { name: "Show all moments" }));
    await waitFor(() => expect(signal).toBeDefined());
    // The lazy controller is created on first expand; unmounting must abort it
    // so an in-flight request cannot resolve into a disposed row.
    expect(signal?.aborted).toBe(false);

    unmount();
    expect(signal?.aborted).toBe(true);
  });

  test("unmounts a row that never expanded without side effects", () => {
    searchVideoMoments.mockReset();
    const { unmount } = render(() => <VideoResult video={video} query="moment" rank={1} />);

    expect(() => unmount()).not.toThrow();
    expect(searchVideoMoments).not.toHaveBeenCalled();
  });

  test("copies a permalink that points at the current search result", async () => {
    window.history.replaceState({}, "", "/?q=moment&page=2");
    render(() => <VideoResult video={video} query="moment" rank={1} />);

    expect(document.getElementById("video-video-1")).toBeInstanceOf(HTMLElement);
    fireEvent.click(screen.getByRole("button", { name: "Copy result link" }));
    await waitFor(() =>
      expect(runCopyToClipboard).toHaveBeenCalledWith(
        `${window.location.origin}/?q=moment&page=2#video-video-1`,
      ),
    );
    expect(window.location.hash).toBe("#video-video-1");
    expect(capture).toHaveBeenCalledWith(
      "result_link_copied",
      expect.objectContaining({ video_id: "video-1", rank: 1 }),
    );
  });

  test("still writes the result permalink when clipboard access is denied", async () => {
    runCopyToClipboard.mockRejectedValueOnce(new Error("Clipboard copy was rejected"));
    window.history.replaceState({}, "", "/?q=moment");
    render(() => <VideoResult video={video} query="moment" rank={1} />);

    fireEvent.click(screen.getByRole("button", { name: "Copy result link" }));
    await waitFor(() => expect(window.location.hash).toBe("#video-video-1"));
    await waitFor(() =>
      expect(capture).toHaveBeenCalledWith(
        "result_link_copied",
        expect.objectContaining({ video_id: "video-1" }),
      ),
    );
  });
});
