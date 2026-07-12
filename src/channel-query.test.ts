import { expect, test } from "bun:test";
import { channelMentionQuery, channelSuggestions, resolveChannelMention } from "./channel-query";
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

test("matches channel names and handles after @", () => {
  expect(channelSuggestions(channels, "@theo")[0]?.handle).toBe("@t3dotgg");
  expect(channelSuggestions(channels, "@t3")[0]?.handle).toBe("@t3dotgg");
});

test("uses a unique match for @channel phrase searches", () => {
  expect(resolveChannelMention(channels, "@theo shooting")?.handle).toBe("@t3dotgg");
  expect(channelMentionQuery("@theo shooting")).toBe("shooting");
});

test("does not guess ambiguous or unknown channels", () => {
  expect(resolveChannelMention(channels, "@ben")).toBeUndefined();
  expect(resolveChannelMention(channels, "@missing")).toBeUndefined();
});
