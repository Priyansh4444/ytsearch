import type { IndexedChannel } from "./types";
import { normalizeHandle } from "./utils";

/** Convenience entry point that builds the index per call. Fine for one-shot
 * use; hot paths (per keystroke) must build the index once and call
 * `channelSuggestionsByKeys` instead. */
export function channelSuggestions(channels: IndexedChannel[], query: string): IndexedChannel[] {
  return channelSuggestionsByKeys(buildChannelKeyIndex(channels), query);
}

/** Precomputed lowercase keys for one channel list. Built once per list
 * change; all per-keystroke scans below reuse it with zero allocs per
 * channel (the largest per-keystroke term: 3 O(C) scans with per-channel
 * normalize + toLowerCase before). */
export type ChannelKeyIndex = {
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
  for (let i = 0; i < index.length; i++) {
    const entry = index[i]!;
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

/** Convenience entry point that builds the index per call (see
 * `channelSuggestions`). Hot paths pass a prebuilt index to
 * `resolveChannelMentionByKeys`. */
export function resolveChannelMention(channels: IndexedChannel[], query: string) {
  const token = query.trim().match(/^(@[^\s]+)/)?.[1];
  if (!token) return undefined;
  const exact = channels.find(
    (channel) => normalizeHandle(channel.handle) === normalizeHandle(token),
  );
  return resolveChannelMentionByKeys(buildChannelKeyIndex(channels), exact, query);
}

/** Keyed variant: exact lookup is done by the caller through a normalized
 * map (one alloc per keystroke instead of one per channel); the
 * single-suggestion disambiguation reuses the same index with no second
 * full normalize scan. Same returns for every input, including the
 * `suggestions.length === 1` rule. Callers resolve `exact` through their own
 * normalized-handle map, so no second channel scan happens here. */
export function resolveChannelMentionByKeys(
  index: readonly ChannelKeyIndex[],
  exact: IndexedChannel | undefined,
  query: string,
): IndexedChannel | undefined {
  if (exact) return exact;
  const suggestions = channelSuggestionsByKeys(index, query);
  return suggestions.length === 1 ? suggestions[0] : undefined;
}
