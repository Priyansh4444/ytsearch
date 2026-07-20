import { describe, expect, test } from "vitest";
import { calculateDashboardMetrics, channelCoverage } from "./dashboard-metrics";
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
  test("aggregates catalog, coverage, metadata, and distributions", () => {
    const metrics = calculateDashboardMetrics([
      channel({ avatar_url: "avatar", uploads_playlist: "playlist" }),
      channel({ id: "complete", indexed_video_count: 10, indexed_chunk_count: 120 }),
    ]);

    expect(metrics.channelCount).toBe(2);
    expect(metrics.indexedVideos).toBe(18);
    expect(metrics.publicVideos).toBe(20);
    expect(metrics.remainingVideos).toBe(2);
    expect(metrics.moments).toBe(200);
    expect(metrics.coveragePercent).toBe(90);
    expect(metrics.completeChannels).toBe(1);
    expect(metrics.partialChannels).toBe(1);
    expect(metrics.avatarCount).toBe(1);
    expect(metrics.indexedVideoDistribution).toEqual({ min: 8, median: 9, max: 10 });
    expect(metrics.momentDistribution).toEqual({ min: 80, median: 100, max: 120 });
  });

  test("handles empty and over-indexed public counts safely", () => {
    expect(calculateDashboardMetrics([]).coveragePercent).toBe(0);
    expect(channelCoverage(channel({ public_video_count: 5, indexed_video_count: 6 }))).toBe(100);
  });
});
