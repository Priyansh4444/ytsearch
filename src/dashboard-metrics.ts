import type { IndexedChannel } from "./types";

export type CreatorFilter = "all" | "difference" | "unknown";
export type CreatorSort = "difference" | "videos" | "moments" | "name";
export type CreatorView = { query: string; filter: CreatorFilter; sort: CreatorSort };

// Zero is also the API's fallback when public metadata is unavailable. It cannot
// establish an empty public catalog, so exclude it from count comparisons.
export function publicVideoCount(channel: IndexedChannel): number | null {
  return Number.isFinite(channel.public_video_count) && channel.public_video_count > 0
    ? channel.public_video_count
    : null;
}

export function channelCountDifference(channel: IndexedChannel): number | null {
  const publicCount = publicVideoCount(channel);
  return publicCount === null ? null : publicCount - channel.indexed_video_count;
}

export function visibleCreators(channels: IndexedChannel[], view: CreatorView): IndexedChannel[] {
  const query = view.query.trim().toLocaleLowerCase();
  return channels
    .filter((channel) => {
      const matchesQuery = `${channel.name} ${channel.handle}`.toLocaleLowerCase().includes(query);
      const difference = channelCountDifference(channel);
      const matchesFilter =
        view.filter === "all" ||
        (view.filter === "difference" && difference !== null && difference !== 0) ||
        (view.filter === "unknown" && difference === null);
      return matchesQuery && matchesFilter;
    })
    .sort((left, right) => {
      let order = 0;
      if (view.sort === "difference") {
        const leftDifference = channelCountDifference(left);
        const rightDifference = channelCountDifference(right);
        if (leftDifference === null && rightDifference !== null) return 1;
        if (leftDifference !== null && rightDifference === null) return -1;
        order = (rightDifference ?? 0) - (leftDifference ?? 0);
      }
      if (view.sort === "videos") order = right.indexed_video_count - left.indexed_video_count;
      if (view.sort === "moments") order = right.indexed_chunk_count - left.indexed_chunk_count;
      return (
        order || left.name.localeCompare(right.name) || left.handle.localeCompare(right.handle)
      );
    });
}

export function calculateDashboardMetrics(channels: IndexedChannel[]) {
  const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);
  const knownChannels = channels.filter((channel) => publicVideoCount(channel) !== null);
  const indexedVideos = sum(channels.map((channel) => channel.indexed_video_count));
  const publicVideos = sum(knownChannels.map((channel) => channel.public_video_count));
  const comparableIndexedVideos = sum(knownChannels.map((channel) => channel.indexed_video_count));
  const moments = sum(channels.map((channel) => channel.indexed_chunk_count));

  return {
    channelCount: channels.length,
    indexedVideos,
    publicVideos,
    comparableIndexedVideos,
    countDifference: knownChannels.length > 0 ? publicVideos - comparableIndexedVideos : null,
    knownPublicChannels: knownChannels.length,
    unknownPublicChannels: channels.length - knownChannels.length,
    differingChannels: knownChannels.filter((channel) => channelCountDifference(channel) !== 0)
      .length,
    moments,
    avatarCount: channels.filter((channel) => Boolean(channel.avatar_url)).length,
    playlistCount: channels.filter((channel) => Boolean(channel.uploads_playlist)).length,
    averageVideosPerChannel: divide(indexedVideos, channels.length),
    averageMomentsPerVideo: divide(moments, indexedVideos),
  };
}

function divide(numerator: number, denominator: number): number {
  return denominator > 0 ? numerator / denominator : 0;
}
