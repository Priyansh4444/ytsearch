import type { SearchResponse } from "./types";

const pluralize = (count: number, singular: string, plural = `${singular}s`) =>
  `${count} ${count === 1 ? singular : plural}`;

const completeCountLabel = (returned: number, total: number, singular: string) =>
  returned < total
    ? `${returned} shown of ${total} ${total === 1 ? singular : `${singular}s`}`
    : pluralize(total, singular);

export const searchResultCountLabel = (response: SearchResponse): string => {
  const totalsApproximate = (response as SearchResponse & { readonly totals_approximate?: boolean })
    .totals_approximate;
  const returnedVideos = response.videos.length;
  const returnedMoments = response.videos.reduce((total, video) => total + video.hits.length, 0);
  const totalVideos = Math.max(response.total_videos, returnedVideos);
  const totalMoments = Math.max(response.total_moments, returnedMoments);
  const videoLabel = completeCountLabel(returnedVideos, totalVideos, "video");
  const momentLabel = totalsApproximate
    ? `${pluralize(returnedMoments, "moment")} shown`
    : completeCountLabel(returnedMoments, totalMoments, "moment");

  return `${totalsApproximate ? "Approximately " : ""}${videoLabel} · ${momentLabel}`;
};
