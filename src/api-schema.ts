import * as Schema from "effect/Schema";
import type {
  IndexQueueRequest,
  IndexedChannel,
  IndexStatus,
  SearchAvailability,
  SearchResponse,
} from "./types";

const mutableArray = <S extends Schema.Constraint>(schema: S) =>
  Schema.mutable(Schema.Array(schema));

const SearchSortSchema = Schema.Literals([
  "relevance",
  "relevance_engagement",
  "views_desc",
  "likes_desc",
  "comments_desc",
  "published_desc",
  "published_asc",
]);
const ContentKindSchema = Schema.Literals(["video", "short", "unknown"]);
const MatchKindSchema = Schema.Literals(["transcript", "title"]);

const SearchHitSchema = Schema.Struct({
  id: Schema.String,
  video_id: Schema.String,
  title: Schema.String,
  text: Schema.String,
  // Older API images serialized absent Rust `Option<String>` values as null;
  // current images omit them. Accept both wire representations.
  prev_text: Schema.optionalKey(Schema.NullOr(Schema.String)),
  next_text: Schema.optionalKey(Schema.NullOr(Schema.String)),
  text_highlight: Schema.optionalKey(Schema.String),
  title_highlight: Schema.optionalKey(Schema.String),
  match_kind: MatchKindSchema,
  youtuber_handle: Schema.String,
  youtuber_name: Schema.String,
  thumbnail: Schema.optionalKey(Schema.String),
  published_at: Schema.optionalKey(Schema.String),
  view_count: Schema.optionalKey(Schema.Number),
  like_count: Schema.optionalKey(Schema.Number),
  comment_count: Schema.optionalKey(Schema.Number),
  content_kind: ContentKindSchema,
  t_start_ms: Schema.Number,
  timestamp: Schema.optionalKey(Schema.String),
  youtube_url: Schema.String,
  embed_url: Schema.String,
});

const SearchVideoResultSchema = Schema.Struct({
  video_id: Schema.String,
  title: Schema.String,
  title_highlight: Schema.optionalKey(Schema.String),
  youtuber_handle: Schema.String,
  youtuber_name: Schema.String,
  thumbnail: Schema.optionalKey(Schema.String),
  published_at: Schema.optionalKey(Schema.String),
  view_count: Schema.optionalKey(Schema.Number),
  like_count: Schema.optionalKey(Schema.Number),
  comment_count: Schema.optionalKey(Schema.Number),
  content_kind: ContentKindSchema,
  match_count: Schema.Number,
  youtube_url: Schema.String,
  embed_url: Schema.String,
  best_timestamp: Schema.optionalKey(Schema.String),
  hits: mutableArray(SearchHitSchema),
});

export const SearchResponseSchema = Schema.Struct({
  query: Schema.String,
  total_moments: Schema.Number,
  total_videos: Schema.Number,
  totals_approximate: Schema.Boolean,
  total_found: Schema.Number,
  out_of: Schema.Number,
  time_to_first_result_ms: Schema.Number,
  page: Schema.Number,
  per_page: Schema.Number,
  sort: SearchSortSchema,
  mode: Schema.Literal("lexical"),
  has_next_page: Schema.Boolean,
  hits: mutableArray(SearchHitSchema),
  videos: mutableArray(SearchVideoResultSchema),
}) satisfies Schema.Schema<SearchResponse>;

export const SearchAvailabilitySchema = Schema.Struct({
  status: Schema.Literals(["scheduled_offline", "prewarming", "ready", "starting", "degraded"]),
  search_enabled: Schema.Boolean,
  enforced: Schema.Boolean,
  schedule_policy: Schema.Literals(["always_on", "scheduled", "observe"]),
  schedule_phase: Schema.Literals(["closed", "prewarm", "open"]),
  time_zone: Schema.Literal("America/Los_Angeles"),
  daily_window: Schema.Struct({
    opens: Schema.Literal("12:00"),
    closes: Schema.Literal("00:00"),
    label: Schema.Literal("12:00 PM–12:00 AM Pacific Time"),
  }),
  next_transition_at: Schema.String,
}) satisfies Schema.Schema<SearchAvailability>;

export const IndexedChannelSchema = Schema.Struct({
  id: Schema.String,
  handle: Schema.String,
  name: Schema.String,
  channel_id: Schema.String,
  uploads_playlist: Schema.optionalKey(Schema.String),
  public_video_count: Schema.Number,
  indexed_video_count: Schema.Number,
  indexed_chunk_count: Schema.Number,
  avatar_url: Schema.optionalKey(Schema.String),
});

export const IndexedChannelsResponseSchema = Schema.Struct({
  channels: mutableArray(IndexedChannelSchema),
}) satisfies Schema.Schema<{ channels: IndexedChannel[] }>;

export const IndexQueueResponseSchema = Schema.Struct({
  request: Schema.Struct({
    id: Schema.String,
    handle: Schema.String,
    note: Schema.optionalKey(Schema.String),
    requested_at: Schema.Number,
    source_backend: Schema.String,
    status: Schema.Literals(["pending", "queued", "running", "failed", "rejected", "idle"]),
  }),
}) satisfies Schema.Schema<{ request: IndexQueueRequest }>;

export const IndexStatusSchema = Schema.Struct({
  handle: Schema.String,
  status: Schema.Literals(["pending", "queued", "running", "failed", "rejected", "idle"]),
  channel: Schema.optionalKey(IndexedChannelSchema),
  request_id: Schema.optionalKey(Schema.String),
  target_backend: Schema.optionalKey(Schema.String),
}) satisfies Schema.Schema<IndexStatus>;

export const HealthSchema = Schema.Struct({ ok: Schema.Boolean });

export const ErrorResponseSchema = Schema.Struct({
  error: Schema.optionalKey(Schema.String),
  code: Schema.optionalKey(Schema.String),
  available_at: Schema.optionalKey(Schema.String),
});
