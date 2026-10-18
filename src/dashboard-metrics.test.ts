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

  test.each([0, -1, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY])(
    "treats public metadata %s as unknown without discarding indexed work",
    (public_video_count) => {
      const metrics = calculateDashboardMetrics([channel({ public_video_count })]);
      expect(publicVideoCount(channel({ public_video_count }))).toBeNull();
      expect(metrics.countDifference).toBeNull();
      expect(metrics.knownPublicChannels).toBe(0);
      expect(metrics.unknownPublicChannels).toBe(1);
      expect(metrics.comparableIndexedVideos).toBe(0);
      expect(metrics.indexedVideos).toBe(8);
      expect(metrics.moments).toBe(80);
    },
  );

  test("preserves finite positive public metadata without rounding", () => {
    expect(publicVideoCount(channel({ public_video_count: 2.5 }))).toBe(2.5);
    expect(channelCountDifference(channel({ public_video_count: 2.5 }))).toBe(-5.5);
  });

  test("separates signed comparison totals from the entire indexed catalog", () => {
    expect(
      calculateDashboardMetrics([
        channel({ avatar_url: "avatar", uploads_playlist: "playlist" }),
        channel({ public_video_count: 7, indexed_video_count: 12, indexed_chunk_count: 120 }),
        channel({ public_video_count: 0, indexed_video_count: 20, indexed_chunk_count: 60 }),
        channel({ public_video_count: -1, indexed_video_count: 5, indexed_chunk_count: 10 }),
      ]),
    ).toEqual({
      channelCount: 4,
      indexedVideos: 45,
      publicVideos: 17,
      comparableIndexedVideos: 20,
      countDifference: -3,
      knownPublicChannels: 2,
      unknownPublicChannels: 2,
      differingChannels: 2,
      moments: 270,
      avatarCount: 1,
      playlistCount: 1,
      averageVideosPerChannel: 11.25,
      averageMomentsPerVideo: 6,
    });
  });

  test("keeps selection stable and leaves frozen inputs untouched", () => {
    const first = Object.freeze(channel({ id: "first" }));
    const second = Object.freeze(channel({ id: "second" }));
    const channels = Object.freeze([first, second]);
    const view = Object.freeze({ query: "", filter: "all", sort: "name" } as const);
    for (const sort of ["difference", "videos", "moments", "name"] as const) {
      const selected = visibleCreators(channels, { ...view, sort });
      expect(selected.map((item) => item.id)).toEqual(["first", "second"]);
      expect(selected[0]).toBe(first);
      expect(selected[1]).toBe(second);
    }
    visibleCreators(channels, view);
    calculateDashboardMetrics(channels);
    expect(channels.map((item) => item.id)).toEqual(["first", "second"]);
    expect(calculateDashboardMetrics([])).toEqual({
      channelCount: 0,
      indexedVideos: 0,
      publicVideos: 0,
      comparableIndexedVideos: 0,
      countDifference: null,
      knownPublicChannels: 0,
      unknownPublicChannels: 0,
      differingChannels: 0,
      moments: 0,
      avatarCount: 0,
      playlistCount: 0,
      averageVideosPerChannel: 0,
      averageMomentsPerVideo: 0,
    });
  });

  test("matches normalized queries across the name and handle boundary", () => {
    const channels = [channel({ id: "alpha", name: "Alpha", handle: "@alpha" }), channel()];
    expect(
      visibleCreators(channels, { query: "  HA @AL  ", filter: "all", sort: "name" }).map(
        (item) => item.id,
      ),
    ).toEqual(["alpha"]);
  });

  test("orders every sort by its domain key, then name and handle", () => {
    const channels = [
      channel({
        id: "unknown",
        name: "Zulu",
        public_video_count: 0,
        indexed_video_count: 100,
        indexed_chunk_count: 10,
      }),
      channel({ id: "negative", name: "Beta", indexed_video_count: 12, indexed_chunk_count: 200 }),
      channel({ id: "zero", name: "Alpha", indexed_video_count: 10, indexed_chunk_count: 20 }),
      channel({ id: "positive-z", name: "Same", handle: "@z", indexed_chunk_count: 100 }),
      channel({ id: "positive-a", name: "Same", handle: "@a", indexed_chunk_count: 100 }),
    ];
    const view = { query: "", filter: "all", sort: "difference" } as const;
    expect(visibleCreators(channels, view).map((item) => item.id)).toEqual([
      "positive-a",
      "positive-z",
      "zero",
      "negative",
      "unknown",
    ]);
    expect(visibleCreators(channels, { ...view, sort: "videos" }).map((item) => item.id)).toEqual([
      "unknown",
      "negative",
      "zero",
      "positive-a",
      "positive-z",
    ]);
    expect(visibleCreators(channels, { ...view, sort: "moments" }).map((item) => item.id)).toEqual([
      "negative",
      "positive-a",
      "positive-z",
      "zero",
      "unknown",
    ]);
    expect(visibleCreators(channels, { ...view, sort: "name" }).map((item) => item.id)).toEqual([
      "zero",
      "negative",
      "positive-a",
      "positive-z",
      "unknown",
    ]);
    expect(
      visibleCreators(channels, { ...view, filter: "difference" }).map((item) => item.id),
    ).toEqual(["positive-a", "positive-z", "negative"]);
    expect(
      visibleCreators(channels, { ...view, filter: "unknown" }).map((item) => item.id),
    ).toEqual(["unknown"]);
  });
});
