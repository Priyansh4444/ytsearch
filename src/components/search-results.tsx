import { For, Show, type Component } from "solid-js";
import { searchResultCountLabel } from "../result-counts";
import type { SearchResponse, SearchVideoResult } from "../types";
import { normalizeHandle } from "../utils";
import { EmptyState } from "./states";
import { PaginationControls } from "./pagination-controls";
import { VideoResult } from "./video-result";

export const SearchResults: Component<{
  response: SearchResponse;
  youtuber: string;
  page: number;
  onPageChange: (page: number) => void;
}> = (props) => {
  // Keyed remount: the parent `Show keyed` in app.tsx creates a fresh mount
  // per response/page, so one setup-only partition covers this render with no
  // memo node. Do NOT read this outside a keyed boundary without memoizing.
  const { videosAndUnknown, shorts } = partitionVideos(props.response.videos);
  return (
    <div class="flex flex-col gap-5">
      <div class="flex flex-wrap items-start justify-between gap-x-4 gap-y-2 border-b border-white/10 pb-3 text-sm text-vesper-muted">
        <div class="flex flex-wrap items-center gap-2">
          <span class="font-medium text-white">{props.response.query}</span>
          <span class="border border-white/10 bg-vesper-panel px-2 py-0.5 text-xs uppercase tracking-[0.12em] text-vesper-dim">
            {props.youtuber ? `Channel: ${normalizeHandle(props.youtuber)}` : "All channels"}
          </span>
        </div>
        <span class="text-xs tabular-nums sm:text-sm">
          {searchResultCountLabel(props.response)} ·{" "}
          {props.response.request_duration_ms ?? props.response.time_to_first_result_ms} ms request
          {props.response.request_duration_ms === undefined
            ? ""
            : ` · ${props.response.time_to_first_result_ms} ms search`}
        </span>
      </div>

      <Show
        when={props.response.videos.length > 0}
        fallback={<EmptyState query={props.response.query} />}
      >
        <div class="flex flex-col gap-7">
          <ResultSection
            title="Videos"
            videos={videosAndUnknown}
            query={props.response.query}
            totalLabel="current page"
          />
          <ResultSection
            title="Shorts"
            videos={shorts}
            query={props.response.query}
            totalLabel="current page"
          />
        </div>

        <PaginationControls
          response={props.response}
          currentPage={props.page}
          onPageChange={props.onPageChange}
        />
      </Show>
    </div>
  );
};

const ResultSection: Component<{
  title: string;
  videos: SearchVideoResult[];
  query: string;
  totalLabel: string;
}> = (props) => (
  <Show when={props.videos.length > 0}>
    <section class="flex flex-col gap-3">
      <h2 class="border-b border-white/10 pb-2 text-sm font-medium uppercase tracking-[0.12em] text-vesper-dim">
        {props.title} · {props.videos.length} on {props.totalLabel}
      </h2>
      <div class="grid gap-2">
        <For each={props.videos}>
          {(video, index) => <VideoResult video={video} query={props.query} rank={index() + 1} />}
        </For>
      </div>
    </section>
  </Show>
);

// Single pass. Ordering must match the previous
// `[...videos.filter("video"), ...videos.filter("unknown")]`: every `video` first
// (source order), then every `unknown`; shorts stay separate. One shared bucket
// would interleave `unknown` between `video` cards AND shift the per-section
// `rank` values that feed analytics payloads.
const partitionVideos = (videos: SearchVideoResult[]) => {
  const mains: SearchVideoResult[] = [];
  const unknown: SearchVideoResult[] = [];
  const shorts: SearchVideoResult[] = [];
  for (const video of videos) {
    if (video.content_kind === "short") shorts.push(video);
    else if (video.content_kind === "unknown") unknown.push(video);
    // Any kind outside the union is unreachable (the decoder rejects it) and
    // would land in Videos rather than being dropped.
    else mains.push(video);
  }
  // `unknown` is empty on most pages, so this second loop usually costs nothing.
  for (const video of unknown) mains.push(video);
  return { videosAndUnknown: mains, shorts };
};
