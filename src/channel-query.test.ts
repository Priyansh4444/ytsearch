import { describe, expect, test } from "vitest";
import {
  coalesceChannels,
  buildChannelKeyIndex,
  channelMentionQuery,
  channelSuggestionsByKeys,
  resolveChannelMentionByKeys,
} from "./channel-query";
import type { IndexedChannel } from "./types";

const channels: IndexedChannel[] = [
  {
    id: "1",
    handle: "@t3dotgg",
    name: "Theo - t3.gg",
    channel_id: "UC1",
    public_video_count: 1,
    indexed_video_count: 1,
    indexed_chunk_count: 1,
  },
  {
    id: "2",
    handle: "@benjicken",
    name: "Ben Jicken",
    channel_id: "UC2",
    public_video_count: 1,
    indexed_video_count: 1,
    indexed_chunk_count: 1,
  },
  {
    id: "3",
    handle: "@bensomething",
    name: "Ben Something",
    channel_id: "UC3",
    public_video_count: 1,
    indexed_video_count: 1,
    indexed_chunk_count: 1,
  },
];

// The app builds the index once per channel-list change and resolves the exact
// handle through its own normalized-handle map (see `app.tsx`), then calls these
// keyed functions. Exercise that shipped path, not a per-call convenience
// wrapper.
const index = buildChannelKeyIndex(channels);
const exactHandle = (token: string) => channels.find((channel) => channel.handle === token);

test("matches channel names and handles after @", () => {
  expect(channelSuggestionsByKeys(index, "@theo")[0]?.handle).toBe("@t3dotgg");
  expect(channelSuggestionsByKeys(index, "@t3")[0]?.handle).toBe("@t3dotgg");
  // Only an @-prefixed query is a channel query.
  expect(channelSuggestionsByKeys(index, "theo")).toEqual([]);
});

test("uses a unique match for @channel phrase searches", () => {
  expect(
    resolveChannelMentionByKeys(index, exactHandle("@t3dotgg"), "@theo shooting")?.handle,
  ).toBe("@t3dotgg");
  expect(channelMentionQuery("@theo shooting")).toBe("shooting");
});

test("does not guess ambiguous or unknown channels", () => {
  expect(resolveChannelMentionByKeys(index, undefined, "@ben")).toBeUndefined();
  expect(resolveChannelMentionByKeys(index, undefined, "@missing")).toBeUndefined();
});

test("an explicit exact match wins without consulting the suggestion scan", () => {
  // `@ben` matches two channels, so the scan alone would be ambiguous; the
  // caller's exact lookup is what disambiguates it.
  expect(resolveChannelMentionByKeys(index, channels[1], "@ben")?.handle).toBe("@benjicken");
});

describe("coalesceChannels", () => {
  const record = (handle: string, channel_id: string, indexed_video_count: number) =>
    ({
      id: handle.slice(1),
      handle,
      name: handle,
      channel_id,
      public_video_count: indexed_video_count,
      indexed_video_count,
      indexed_chunk_count: 0,
    }) as const;

  test("shows a renamed creator once, as the record holding more videos, in either order", () => {
    const oldHandle = record("@t3dotgg", "UCbRP3c757lWg9M-U7TyEkXA", 1186);
    const current = record("@theo", "UCbRP3c757lWg9M-U7TyEkXA", 1190);
    expect(coalesceChannels([oldHandle, current]).map((channel) => channel.handle)).toEqual([
      "@theo",
    ]);
    expect(coalesceChannels([current, oldHandle]).map((channel) => channel.handle)).toEqual([
      "@theo",
    ]);
  });

  test("keeps different channels, their order, and records with no channel ID", () => {
    const channels = [record("@b", "UC2", 5), record("@a", "UC1", 9), record("@c", "", 1)];
    expect(coalesceChannels([...channels, record("@d", "", 1)]).map((c) => c.handle)).toEqual([
      "@b",
      "@a",
      "@c",
      "@d",
    ]);
  });
});
