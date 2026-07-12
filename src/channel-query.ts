import type { IndexedChannel } from "./types";
import { normalizeHandle } from "./utils";

export function channelSuggestions(channels: IndexedChannel[], query: string): IndexedChannel[] {
  const value = query.trim().toLowerCase();
  if (!value.startsWith("@")) return [];
  const needle = value.slice(1).split(/\s+/, 1)[0] ?? "";
  return channels.filter(
    (channel) =>
      channel.handle.toLowerCase().includes(needle) || channel.name.toLowerCase().includes(needle),
  );
}

export function channelMentionQuery(query: string): string {
  return (
    query
      .trim()
      .match(/^(@[^\s]+)(?:\s+(.+))?$/)?.[2]
      ?.trim() ?? ""
  );
}

export function resolveChannelMention(channels: IndexedChannel[], query: string) {
  const token = query.trim().match(/^(@[^\s]+)/)?.[1];
  if (!token) return undefined;
  const exact = channels.find(
    (channel) => normalizeHandle(channel.handle) === normalizeHandle(token),
  );
  return (
    exact ??
    (channelSuggestions(channels, query).length === 1
      ? channelSuggestions(channels, query)[0]
      : undefined)
  );
}
