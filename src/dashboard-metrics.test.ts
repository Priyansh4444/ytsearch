import { describe, expect, test } from "vitest";
import {
  calculateDashboardMetrics,
  channelCountDifference,
  publicVideoCount,
  visibleCreators,
} from "./dashboard-metrics";
import type { IndexedChannel } from "./types";

const channel = (overrides: Partial<IndexedChannel> = {}): IndexedChannel => ({
  id: "creator",
  handle: "@creator",
  name: "Creator",
  channel_id: "channel",
  public_video_count: 10,
  indexed_video_count: 8,
  indexed_chunk_count: 80,
  ...overrides,
});

describe("dashboard metrics", () => {
  test("compares reported counts without treating them as confirmed missing videos", () => {
    const metrics = calculateDashboardMetrics([
      channel({ avatar_url: "avatar", uploads_playlist: "playlist" }),
      channel({ id: "second", indexed_video_count: 12, indexed_chunk_count: 120 }),
    ]);
    expect(metrics.channelCount).toBe(2);
    expect(metrics.indexedVideos).toBe(20);
    expect(metrics.publicVideos).toBe(20);
    expect(metrics.countDifference).toBe(0);
    expect(metrics.differingChannels).toBe(2);
    expect(metrics.moments).toBe(200);
    expect(metrics.avatarCount).toBe(1);
    expect(metrics.playlistCount).toBe(1);
    expect(metrics.averageMomentsPerVideo).toBe(10);
    expect(channelCountDifference(channel({ public_video_count: 5, indexed_video_count: 6 }))).toBe(
      -1,
    );
  });

  test("excludes unavailable public totals from both sides of the comparison", () => {
    const metrics = calculateDashboardMetrics([
      channel(),
      channel({ public_video_count: 0, indexed_video_count: 20 }),
    ]);
    expect(metrics.indexedVideos).toBe(28);
    expect(metrics.comparableIndexedVideos).toBe(8);
    expect(metrics.publicVideos).toBe(10);
    expect(metrics.countDifference).toBe(2);
    expect(metrics.unknownPublicChannels).toBe(1);
    expect(publicVideoCount(channel({ public_video_count: 0 }))).toBeNull();
    expect(publicVideoCount(channel({ public_video_count: Number.NaN }))).toBeNull();
    expect(channelCountDifference(channel({ public_video_count: 0 }))).toBeNull();
    expect(calculateDashboardMetrics([]).countDifference).toBeNull();
    expect(calculateDashboardMetrics([]).averageVideosPerChannel).toBe(0);
  });

  test("filters by name, handle, count differences, and unknown totals without mutating input", () => {
    const channels = [
      channel({ name: "Zulu", handle: "@zulu" }),
      channel({ name: "Alpha", handle: "@alpha", public_video_count: 0 }),
      channel({ name: "Beta", handle: "@beta", indexed_video_count: 10 }),
    ];
    const view = { query: "", filter: "all", sort: "name" } as const;
    expect(visibleCreators(channels, view).map((item) => item.name)).toEqual([
      "Alpha",
      "Beta",
      "Zulu",
    ]);
    expect(
      visibleCreators(channels, { ...view, query: " @ZULU " }).map((item) => item.name),
    ).toEqual(["Zulu"]);
    expect(
      visibleCreators(channels, { ...view, filter: "difference" }).map((item) => item.name),
    ).toEqual(["Zulu"]);
    expect(
      visibleCreators(channels, { ...view, filter: "unknown" }).map((item) => item.name),
    ).toEqual(["Alpha"]);
    expect(channels[0]?.name).toBe("Zulu");
  });

  test("sorts differences with unknown totals last and supports indexed and moment counts", () => {
    const channels = [
      channel({ name: "Unknown", public_video_count: 0, indexed_video_count: 100 }),
      channel({ name: "Negative", indexed_video_count: 12 }),
      channel({ name: "Positive", indexed_chunk_count: 200 }),
    ];
    const view = { query: "", filter: "all", sort: "difference" } as const;
    expect(visibleCreators(channels, view).map((item) => item.name)).toEqual([
      "Positive",
      "Negative",
      "Unknown",
    ]);
    expect(visibleCreators(channels, { ...view, sort: "videos" })[0]?.name).toBe("Unknown");
    expect(visibleCreators(channels, { ...view, sort: "moments" })[0]?.name).toBe("Positive");
  });
});
