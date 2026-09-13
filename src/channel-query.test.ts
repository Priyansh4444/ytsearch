import { expect, test } from "vitest";
import {
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
