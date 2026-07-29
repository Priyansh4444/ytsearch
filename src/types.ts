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
export type SearchMode = "lexical" | "semantic";

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
  mode: SearchMode;
  has_next_page: boolean;
  hits: SearchHit[];
  videos: SearchVideoResult[];
};

export type SearchRequest = {
  query: string;
  youtuber: string;
  page: number;
  sort: SearchSort;
  mode: SearchMode;
};

export type SearchAvailabilityStatus =
  | "scheduled_offline"
  | "prewarming"
  | "ready"
  | "starting"
  | "degraded";

export type SearchAvailability = {
  status: SearchAvailabilityStatus;
  search_enabled: boolean;
  enforced: boolean;
  schedule_phase: "closed" | "prewarm" | "open";
  time_zone: "America/Los_Angeles";
  daily_window: {
    opens: "12:00";
    closes: "00:00";
    label: "12:00 PM–12:00 AM Pacific Time";
  };
  next_transition_at: string;
};

export type RecentSearch = Omit<SearchRequest, "page"> & {
  searchedAt: string;
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
  status: IndexRequestStatus;
};

export type IndexRequestStatus = "pending" | "queued" | "running" | "failed" | "rejected" | "idle";

export type IndexStatus = {
  handle: string;
  status: IndexRequestStatus;
  channel?: IndexedChannel;
  request_id?: string;
  target_backend?: string;
};

export type DashboardData = {
  channels: IndexedChannel[];
  channelsAvailable: boolean;
  indexStatus?: IndexStatus;
  apiHealthy: boolean;
  fetchedAt: string;
  latencyMs: {
    health?: number;
    channels?: number;
    indexStatus?: number;
  };
  errors: string[];
};
