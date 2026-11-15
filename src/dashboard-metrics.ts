import type { IndexedChannel } from "./types";

export type CreatorFilter = "all" | "difference" | "unknown";
export type CreatorSort = "difference" | "videos" | "moments" | "name";
export type CreatorView = Readonly<{
  query: string;
  filter: CreatorFilter;
  sort: CreatorSort;
}>;

export type DashboardMetrics = Readonly<{
  channelCount: number;
  indexedVideos: number;
  publicVideos: number;
  comparableIndexedVideos: number;
  countDifference: number | null;
  knownPublicChannels: number;
  unknownPublicChannels: number;
  differingChannels: number;
  moments: number;
  avatarCount: number;
  playlistCount: number;
  averageVideosPerChannel: number;
  averageMomentsPerVideo: number;
}>;

type CountTotals = Readonly<{
  indexedVideos: number;
  publicVideos: number;
  comparableIndexedVideos: number;
  knownPublicChannels: number;
  differingChannels: number;
  moments: number;
  avatarCount: number;
  playlistCount: number;
}>;

const EMPTY_TOTALS: CountTotals = {
  indexedVideos: 0,
  publicVideos: 0,
  comparableIndexedVideos: 0,
  knownPublicChannels: 0,
  differingChannels: 0,
  moments: 0,
  avatarCount: 0,
  playlistCount: 0,
};

// Zero is also the API's fallback when public metadata is unavailable. It cannot
// establish an empty public catalog, so exclude it from count comparisons.
export function publicVideoCount(channel: Readonly<IndexedChannel>): number | null {
  return Number.isFinite(channel.public_video_count) && channel.public_video_count > 0
    ? channel.public_video_count
    : null;
}

export function channelCountDifference(channel: Readonly<IndexedChannel>): number | null {
  const publicCount = publicVideoCount(channel);
  return publicCount === null ? null : publicCount - channel.indexed_video_count;
}

export function visibleCreators(
  channels: readonly IndexedChannel[],
  view: CreatorView,
): IndexedChannel[] {
  const query = view.query.trim().toLocaleLowerCase();
  return channels
    .filter(
      (channel) =>
        `${channel.name} ${channel.handle}`.toLocaleLowerCase().includes(query) &&
        matchesFilter(channelCountDifference(channel), view.filter),
    )
    .sort(
      (left, right) =>
        sortOrder(left, right, view.sort) ||
        left.name.localeCompare(right.name) ||
        left.handle.localeCompare(right.handle),
    );
}

function matchesFilter(difference: number | null, filter: CreatorFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "difference":
      return difference !== null && difference !== 0;
    case "unknown":
      return difference === null;
  }
}

function sortOrder(
  left: Readonly<IndexedChannel>,
  right: Readonly<IndexedChannel>,
  sort: CreatorSort,
): number {
  switch (sort) {
    case "difference": {
      const leftDifference = channelCountDifference(left);
      const rightDifference = channelCountDifference(right);
      if (leftDifference === null && rightDifference !== null) return 1;
      if (leftDifference !== null && rightDifference === null) return -1;
      return (rightDifference ?? 0) - (leftDifference ?? 0);
    }
    case "videos":
      return right.indexed_video_count - left.indexed_video_count;
    case "moments":
      return right.indexed_chunk_count - left.indexed_chunk_count;
    case "name":
      return 0;
  }
}

function accumulateCounts(totals: CountTotals, channel: Readonly<IndexedChannel>): CountTotals {
  const publicCount = publicVideoCount(channel);
  const known = publicCount !== null;
  return {
    indexedVideos: totals.indexedVideos + channel.indexed_video_count,
    publicVideos: totals.publicVideos + (publicCount ?? 0),
    comparableIndexedVideos:
      totals.comparableIndexedVideos + (known ? channel.indexed_video_count : 0),
    knownPublicChannels: totals.knownPublicChannels + (known ? 1 : 0),
    differingChannels:
      totals.differingChannels + (known && publicCount !== channel.indexed_video_count ? 1 : 0),
    moments: totals.moments + channel.indexed_chunk_count,
    avatarCount: totals.avatarCount + (channel.avatar_url ? 1 : 0),
    playlistCount: totals.playlistCount + (channel.uploads_playlist ? 1 : 0),
  };
}

export function calculateDashboardMetrics(
  channels: readonly Readonly<IndexedChannel>[],
): DashboardMetrics {
  const totals = channels.reduce(accumulateCounts, EMPTY_TOTALS);
  return {
    channelCount: channels.length,
    indexedVideos: totals.indexedVideos,
    publicVideos: totals.publicVideos,
    comparableIndexedVideos: totals.comparableIndexedVideos,
    countDifference:
      totals.knownPublicChannels > 0 ? totals.publicVideos - totals.comparableIndexedVideos : null,
    knownPublicChannels: totals.knownPublicChannels,
    unknownPublicChannels: channels.length - totals.knownPublicChannels,
    differingChannels: totals.differingChannels,
    moments: totals.moments,
    avatarCount: totals.avatarCount,
    playlistCount: totals.playlistCount,
    averageVideosPerChannel: divide(totals.indexedVideos, channels.length),
    averageMomentsPerVideo: divide(totals.moments, totals.indexedVideos),
  };
}

function divide(numerator: number, denominator: number): number {
  return denominator > 0 ? numerator / denominator : 0;
}
