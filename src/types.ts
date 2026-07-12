export type SearchHit = {
  id: string;
  video_id: string;
  title: string;
  text: string;
  prev_text?: string;
  next_text?: string;
  text_highlight?: string;
  title_highlight?: string;
  match_kind: "transcript" | "title";
  youtuber_handle: string;
  youtuber_name: string;
  thumbnail?: string;
  published_at?: string;
  content_kind: "video" | "short" | "unknown";
  t_start_ms: number;
  timestamp?: string;
  youtube_url: string;
  embed_url: string;
};

export type SearchVideoResult = {
  video_id: string;
  title: string;
  title_highlight?: string;
  youtuber_handle: string;
  youtuber_name: string;
  thumbnail?: string;
  published_at?: string;
  content_kind: "video" | "short" | "unknown";
  match_count: number;
  youtube_url: string;
  embed_url: string;
  best_timestamp?: string;
  hits: SearchHit[];
};

export type SearchSort = "relevance" | "published_desc" | "published_asc";

export type SearchResponse = {
  query: string;
  total_moments: number;
  total_videos: number;
  total_found: number;
  out_of: number;
  time_to_first_result_ms: number;
  page: number;
  per_page: number;
  sort: SearchSort;
  has_next_page: boolean;
  hits: SearchHit[];
  videos: SearchVideoResult[];
};

export type SearchRequest = {
  query: string;
  youtuber: string;
  page: number;
  sort: SearchSort;
};

export type IndexedChannel = {
  id: string;
  handle: string;
  name: string;
  channel_id: string;
  uploads_playlist?: string;
  public_video_count: number;
  indexed_video_count: number;
  indexed_chunk_count: number;
  avatar_url?: string;
};

export type IndexQueueRequest = {
  id: string;
  handle: string;
  note?: string;
  requested_at: number;
  source_backend: string;
  status: string;
};

export type IndexStatus = {
  handle: string;
  status: "queued" | "running" | "idle";
  request_id?: string;
  target_backend?: string;
};
