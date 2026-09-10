import { expect, test } from "vitest";
import {
  buildChannelKeyIndex,
  channelMentionQuery,
  channelSuggestions,
  channelSuggestionsByKeys,
  resolveChannelMention,
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

// The app builds the index once per channel-list change and calls the keyed
// functions; cover that shipped path directly rather than only the wrappers
// that build an index per call.
test("keyed lookups match the convenience wrappers", () => {
  const index = buildChannelKeyIndex(channels);
  const exactOf = (token: string) =>
    channels.find((channel) => `@${channel.handle.replace(/^@/, "")}` === token);

  expect(channelSuggestionsByKeys(index, "@theo")[0]?.handle).toBe("@t3dotgg");
  expect(channelSuggestionsByKeys(index, "no-at-prefix")).toEqual([]);
  expect(resolveChannelMentionByKeys(index, exactOf("@t3dotgg"), "@theo shooting")?.handle).toBe(
    "@t3dotgg",
  );
  expect(resolveChannelMentionByKeys(index, undefined, "@ben")).toBeUndefined();

  // An explicit exact match wins without consulting the suggestion scan.
  expect(resolveChannelMentionByKeys(index, channels[1], "@ben")?.handle).toBe("@benjicken");
});
