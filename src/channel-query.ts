import type { IndexedChannel } from "./types";

/**
 * One circle per YouTube channel. A creator's rename leaves two records with the
 * same channel ID until the old one is published away; the one holding more videos
 * is the current one. Records without a channel ID cannot be matched and stay.
 */
export function coalesceChannels(channels: readonly IndexedChannel[]): IndexedChannel[] {
  const kept = new Map<string, IndexedChannel>();
  for (const channel of channels) {
    const key = channel.channel_id || channel.id;
    const existing = kept.get(key);
    if (!existing || channel.indexed_video_count > existing.indexed_video_count) {
      kept.set(key, channel);
    }
  }
  return [...kept.values()];
}

/** Precomputed lowercase keys for one channel list. Built once per list
 * change; the per-keystroke scans below reuse it with zero allocs per channel
 * (the previous shape ran three O(C) scans per keystroke, each allocating a
 * `@`-prefixed string per channel). There is deliberately no unkeyed
 * convenience wrapper: building the index per call is the exact trap this
 * module exists to avoid. */
type ChannelKeyIndex = {
  readonly channel: IndexedChannel;
  readonly handleKey: string;
  readonly nameKey: string;
};

export function buildChannelKeyIndex(channels: IndexedChannel[]): ChannelKeyIndex[] {
  return channels.map((channel) => ({
    channel,
    handleKey: channel.handle.toLowerCase(),
    nameKey: channel.name.toLowerCase(),
  }));
}

export function channelSuggestionsByKeys(
  index: readonly ChannelKeyIndex[],
  query: string,
): IndexedChannel[] {
  const value = query.trim().toLowerCase();
  if (!value.startsWith("@")) return [];
  const needle = value.slice(1).split(/\s+/, 1)[0] ?? "";
  const out: IndexedChannel[] = [];
  for (const entry of index) {
    if (entry.handleKey.includes(needle) || entry.nameKey.includes(needle)) {
      out.push(entry.channel);
    }
  }
  return out;
}

export function channelMentionQuery(query: string): string {
  return (
    query
      .trim()
      .match(/^(@[^\s]+)(?:\s+(.+))?$/)?.[2]
      ?.trim() ?? ""
  );
}

/** Exact lookup is done by the caller through its own normalized-handle map
 * (one alloc per keystroke instead of one per channel); the single-suggestion
 * disambiguation reuses the passed index with no second full scan. Same
 * returns for every input, including the `suggestions.length === 1` rule. */
export function resolveChannelMentionByKeys(
  index: readonly ChannelKeyIndex[],
  exact: IndexedChannel | undefined,
  query: string,
): IndexedChannel | undefined {
  if (exact) return exact;
  const suggestions = channelSuggestionsByKeys(index, query);
  return suggestions.length === 1 ? suggestions[0] : undefined;
}
