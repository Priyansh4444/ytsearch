import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { describe, expect, test, vi } from "vitest";
import type { SearchHit, SearchVideoResult } from "../types";
import { VideoResult } from "./video-result";

const { capture, searchVideoMoments } = vi.hoisted(() => ({
  capture: vi.fn(),
  searchVideoMoments: vi.fn(),
}));
vi.mock("../analytics", () => ({ capture }));
vi.mock("../api", () => ({ searchVideoMoments }));

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
  youtube_url: "https://youtube.test/watch?v=video-1",
  embed_url: "https://youtube.test/embed/video-1",
  hits,
};

describe("VideoResult", () => {
  test("loads and reveals every transcript moment", async () => {
    searchVideoMoments.mockResolvedValue({ videos: [video] });
    render(() => <VideoResult video={video} query="moment" rank={1} />);

    expect(screen.getAllByRole("button", { name: /Moment \d/ })).toHaveLength(3);
    await fireEvent.click(screen.getByRole("button", { name: "Show all moments" }));
    await waitFor(() =>
      expect(screen.getAllByRole("button", { name: /Moment \d/ })).toHaveLength(7),
    );
    expect(searchVideoMoments).toHaveBeenCalledWith("moment", "video-1");
    expect(screen.queryByRole("button", { name: "Show all moments" })).not.toBeInTheDocument();
  });

  test("records feedback against the selected moment", async () => {
    render(() => <VideoResult video={video} query="moment" rank={2} />);

    await fireEvent.click(screen.getAllByRole("button", { name: "Mark moment useful" })[0]);
    expect(capture).toHaveBeenCalledWith(
      "result_moment_feedback",
      expect.objectContaining({ moment_id: "hit-0", rank: 2, value: "useful" }),
    );
  });
});
