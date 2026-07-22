import type { IndexedChannel } from "./types";

export function calculateDashboardMetrics(channels: IndexedChannel[]) {
  const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);
  const indexedVideos = sum(channels.map((channel) => channel.indexed_video_count));
  const publicVideos = sum(channels.map((channel) => channel.public_video_count));
  const moments = sum(channels.map((channel) => channel.indexed_chunk_count));
  const remainingVideos = sum(
    channels.map((channel) =>
      Math.max(0, channel.public_video_count - channel.indexed_video_count),
    ),
  );
  const coveragePercent = publicVideos > 0 ? (indexedVideos / publicVideos) * 100 : 0;
  const completeChannels = channels.filter(
    (channel) =>
      channel.public_video_count > 0 && channel.indexed_video_count >= channel.public_video_count,
  ).length;
  const partialChannels = channels.filter(
    (channel) =>
      channel.indexed_video_count > 0 && channel.indexed_video_count < channel.public_video_count,
  ).length;
  const sortedIndexedVideos = channels
    .map((channel) => channel.indexed_video_count)
    .sort((left, right) => left - right);
  const sortedMoments = channels
    .map((channel) => channel.indexed_chunk_count)
    .sort((left, right) => left - right);

  return {
    channelCount: channels.length,
    indexedVideos,
    publicVideos,
    remainingVideos,
    moments,
    coveragePercent,
    completeChannels,
    partialChannels,
    avatarCount: channels.filter((channel) => Boolean(channel.avatar_url)).length,
    playlistCount: channels.filter((channel) => Boolean(channel.uploads_playlist)).length,
    averageVideosPerChannel: divide(indexedVideos, channels.length),
    averageMomentsPerChannel: divide(moments, channels.length),
    averageMomentsPerVideo: divide(moments, indexedVideos),
    indexedVideoDistribution: distribution(sortedIndexedVideos),
    momentDistribution: distribution(sortedMoments),
  };
}

export function channelCoverage(channel: IndexedChannel): number {
  if (channel.public_video_count === 0) return 0;
  return Math.min(100, (channel.indexed_video_count / channel.public_video_count) * 100);
}

function divide(numerator: number, denominator: number): number {
  return denominator > 0 ? numerator / denominator : 0;
}

function distribution(sorted: number[]) {
  if (sorted.length === 0) return { min: 0, median: 0, max: 0 };
  const middle = Math.floor(sorted.length / 2);
  const median =
    sorted.length % 2 === 0 ? (sorted[middle - 1]! + sorted[middle]!) / 2 : sorted[middle]!;
  return { min: sorted[0]!, median, max: sorted.at(-1)! };
}
