import { For, Show, createMemo, createSignal, onCleanup } from "solid-js";
import type { Component } from "solid-js";
import { searchVideoMoments } from "../api";
import { capture } from "../analytics";
import type { SearchHit, SearchVideoResult } from "../types";
import { resultAnchorId, searchResultPermalink } from "../search-route";
import { copyToClipboard, formatDate } from "../utils";
import { HighlightedText } from "./highlighted-text";
import { ChevronDownIcon, LoaderCircleIcon, PlayIcon, ThumbsDownIcon, ThumbsUpIcon } from "./icons";
import { CopyLinkButton } from "./toolbar-actions";

export const VideoResult: Component<{ video: SearchVideoResult; query: string; rank: number }> = (
  props,
) => {
  const [allHits, setAllHits] = createSignal(props.video.hits);
  const [showAllMoments, setShowAllMoments] = createSignal(false);
  const [momentsLoading, setMomentsLoading] = createSignal(false);
  const [momentsError, setMomentsError] = createSignal<string>();
  const [selectedHitId, setSelectedHitId] = createSignal<string>();
  const [playerActive, setPlayerActive] = createSignal(false);
  // N controllers were allocated per page mount, N-1 never used. Allocate on
  // first expand only; abort-on-unmount preserved via the same cleanup.
  let momentsController: AbortController | undefined;
  onCleanup(() => momentsController?.abort());
  const getMomentsController = () => (momentsController ??= new AbortController());
  // One memo per fan-out root (read by For input, selection, badge,
  // aria-label, youtube_url): 3 slice allocs collapse to 1, and the
  // slice-then-find chain runs once per evaluation instead of per hit.
  const visible = createMemo(() => (showAllMoments() ? allHits() : allHits().slice(0, 3)));
  const selection = createMemo(() => {
    const list = visible();
    const id = selectedHitId();
    const hit = id === undefined ? undefined : list.find((candidate) => candidate.id === id);
    if (hit) return { hit, id };
    const fallback = list[0] ?? props.video.hits[0];
    return { hit: fallback, id: fallback?.id };
  });
  const selectedHit = () => selection().hit;
  const selectedId = () => selection().id;
  const hasHiddenMoments = () => props.video.match_count > allHits().length || allHits().length > 3;
  // Plain accessor (not a memo): the iframe only reads this when the player
  // branch mounts, so no URL work happens before first activation. The
  // string fast path below removed the parse cost instead of a node.
  const playerUrl = () => autoplayEmbedUrl(selectedHit()?.embed_url ?? props.video.embed_url);
  const posterThumbnail = () =>
    (props.video.thumbnail ?? selectedHit()?.thumbnail)?.replace(
      /^(https:\/\/i\.ytimg\.com\/vi\/[^/]+\/)(?:maxres|sd|hq720)default\.jpg(?:\?.*)?$/,
      "$1hqdefault.jpg",
    );
  const [copied, setCopied] = createSignal(false);
  const activatePlayer = () => {
    if (playerActive()) return;
    setPlayerActive(true);
    capture("result_player_opened", resultProperties(props));
  };
  const expandMoments = async () => {
    if (momentsLoading()) return;
    setMomentsLoading(true);
    setMomentsError(undefined);
    try {
      const response = await searchVideoMoments(
        props.query,
        props.video.video_id,
        getMomentsController().signal,
      );
      const completeVideo = response.videos.find(
        (video) => video.video_id === props.video.video_id,
      );
      if (completeVideo) setAllHits(completeVideo.hits);
      setShowAllMoments(true);
      capture("result_moments_expanded", {
        ...resultProperties(props),
        visible_moments: completeVideo?.hits.length ?? allHits().length,
        total_returned_moments: completeVideo?.hits.length ?? allHits().length,
      });
    } catch {
      setMomentsError("Unable to load all moments. Try again.");
    } finally {
      setMomentsLoading(false);
    }
  };
  const copyResultLink = async () => {
    const url = searchResultPermalink(props.video.video_id);
    window.history.replaceState({}, "", url);
    try {
      await copyToClipboard(url);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1_500);
    } catch {
      // Clipboard permission can be denied; the permalink is already in the URL.
    }
    capture("result_link_copied", resultProperties(props));
  };
  // Stable per-row handlers: the two identical `result_opened` bodies collapse
  // into one reference, so each row allocates one closure fewer (N per-hit
  // closures remain). The per-hit onSelect closure is structurally unavoidable
  // (it captures `hit`), so its body is a single call instead of three
  // statements.
  const openResult = () => capture("result_opened", resultProperties(props));
  const copyLink = () => void copyResultLink();
  const runExpand = () => void expandMoments();
  const selectHit = (hit: SearchHit) => {
    setSelectedHitId(hit.id);
    activatePlayer();
    capture("result_timestamp_selected", {
      ...resultProperties(props),
      match_kind: hit.match_kind,
    });
  };

  return (
    <article
      id={resultAnchorId(props.video.video_id)}
      tabindex="-1"
      class="search-result-card min-w-0 max-w-full overflow-hidden border border-vesper-border bg-vesper-panel transition-colors duration-150 hover:border-vesper-dim"
    >
      <div class="grid min-w-0 max-w-full gap-0 xl:grid-cols-[minmax(32rem,40rem)_1fr]">
        <div class="relative min-w-0 bg-vesper-bg">
          <Show
            when={playerActive()}
            fallback={
              <button
                type="button"
                class="pressable video-poster"
                aria-label={`Play ${props.video.title} at ${selectedHit()?.timestamp ?? "the first match"}`}
                onClick={activatePlayer}
              >
                <Show when={posterThumbnail()}>
                  {(thumbnail) => (
                    <img
                      src={thumbnail()}
                      alt=""
                      width="1280"
                      height="720"
                      loading="lazy"
                      decoding="async"
                      class="video-poster-image"
                    />
                  )}
                </Show>
                <span class="video-poster-overlay">
                  <span class="video-poster-play">
                    <PlayIcon
                      class="translate-x-px"
                      size={24}
                      stroke-width="2"
                      aria-hidden="true"
                    />
                  </span>
                </span>
              </button>
            }
          >
            <iframe
              class="aspect-video w-full"
              src={playerUrl()}
              title={`YouTube player for ${props.video.title}`}
              loading="lazy"
              allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
              referrerpolicy="strict-origin-when-cross-origin"
              allowfullscreen
            />
          </Show>
          <span class="absolute bottom-2 start-2 bg-vesper-bg/90 px-2 py-1 text-xs font-semibold tabular-nums text-vesper-yellow ring-1 ring-vesper-border">
            {selectedHit()?.timestamp ?? "title"}
          </span>
        </div>

        <div class="flex min-w-0 max-w-full flex-col gap-3 p-4">
          <div class="flex flex-col items-stretch gap-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4">
            <div class="min-w-0">
              <a
                href={props.video.youtube_url}
                target="_blank"
                rel="noreferrer"
                class="block break-words text-lg font-semibold leading-snug text-white transition-colors hover:text-vesper-yellow"
                onClick={openResult}
              >
                <HighlightedText
                  text={props.video.title}
                  highlight={props.video.title_highlight}
                  query={props.query}
                />
              </a>
              <p class="mt-1 break-words text-sm text-vesper-muted">
                {props.video.youtuber_name} · {props.video.youtuber_handle}
                <Show when={props.video.published_at}>
                  {" "}
                  · {formatDate(props.video.published_at)}
                </Show>
              </p>
              <Show when={engagementSummary(props.video)}>
                {(summary) => <p class="mt-1 text-xs tabular-nums text-vesper-dim">{summary()}</p>}
              </Show>
            </div>
            <div class="flex w-full shrink-0 items-center gap-2 sm:w-auto">
              <CopyLinkButton copied={copied()} onClick={copyLink} label="Copy result link" />
              <a
                href={selectedHit()?.youtube_url ?? props.video.youtube_url}
                target="_blank"
                rel="noreferrer"
                class="pressable inline-flex h-10 min-w-0 flex-1 items-center justify-center border border-vesper-control bg-vesper-bg px-3 text-sm font-medium text-vesper-green hover:border-vesper-green sm:flex-none"
                onClick={openResult}
              >
                Open on YouTube
              </a>
            </div>
          </div>

          <div class="grid gap-2">
            <For each={visible()}>
              {(hit) => (
                <TranscriptExcerpt
                  hit={hit}
                  query={props.query}
                  video={props.video}
                  rank={props.rank}
                  selected={hit.id === selectedId()}
                  bounded={showAllMoments()}
                  onSelect={() => selectHit(hit)}
                />
              )}
            </For>
            <Show when={!showAllMoments() && hasHiddenMoments()}>
              <button
                type="button"
                onClick={runExpand}
                disabled={momentsLoading()}
                class="pressable inline-flex h-9 items-center justify-center gap-2 border border-vesper-control bg-vesper-bg px-3 text-sm font-medium text-vesper-muted hover:border-vesper-yellow hover:text-vesper-yellow"
              >
                <Show
                  when={!momentsLoading()}
                  fallback={
                    <LoaderCircleIcon
                      class="motion-safe:animate-spin"
                      size={16}
                      aria-hidden="true"
                    />
                  }
                >
                  <ChevronDownIcon size={16} aria-hidden="true" />
                </Show>
                {momentsLoading() ? "Loading every moment…" : "Show all moments"}
              </button>
            </Show>
            <Show when={momentsError()}>
              {(message) => (
                <p class="text-sm text-red-300" role="alert">
                  {message()}
                </p>
              )}
            </Show>
          </div>
        </div>
      </div>
    </article>
  );
};

const compactNumber = new Intl.NumberFormat("en", {
  notation: "compact",
  maximumFractionDigits: 1,
});

function engagementSummary(video: SearchVideoResult): string | undefined {
  // One pass, no closures, no temps: identical strings to the old
  // filter/map/join chain (same cached formatter, same order, same join).
  let out = "";
  let count = 0;
  if (typeof video.view_count === "number" && video.view_count > 0) {
    out += `${compactNumber.format(video.view_count)} views`;
    count++;
  }
  if (typeof video.like_count === "number" && video.like_count > 0) {
    if (count > 0) out += " · ";
    out += `${compactNumber.format(video.like_count)} likes`;
    count++;
  }
  if (typeof video.comment_count === "number" && video.comment_count > 0) {
    if (count > 0) out += " · ";
    out += `${compactNumber.format(video.comment_count)} comments`;
    count++;
  }
  return count > 0 ? out : undefined;
}

function autoplayEmbedUrl(value: string): string {
  // Fast path only when appending is provably identical to
  // `searchParams.set("autoplay", "1")`: an absolute embed URL with no fragment
  // and no query or fragment. Query keys can be bare, percent-encoded, or
  // repeated, so substring checks cannot prove that `autoplay` is absent.
  // Anything ambiguous keeps the parser and its `URLSearchParams.set` semantics.
  if (
    value.startsWith("https://") &&
    value.includes("/embed/") &&
    !value.includes("?") &&
    !value.includes("#")
  ) {
    return `${value}${value.includes("?") ? "&" : "?"}autoplay=1`;
  }
  const url = new URL(value);
  url.searchParams.set("autoplay", "1");
  return url.toString();
}

const TranscriptExcerpt: Component<{
  hit: SearchHit;
  query: string;
  video: SearchVideoResult;
  rank: number;
  selected?: boolean;
  // Only the expanded list is long enough for offscreen excerpts to justify
  // content-visibility; collapsed cards keep their exact previous render path.
  bounded?: boolean;
  onSelect?: () => void;
}> = (props) => {
  const [feedback, setFeedback] = createSignal<"useful" | "not_useful">();
  const submitFeedback = (value: "useful" | "not_useful") => {
    setFeedback(value);
    capture("result_moment_feedback", {
      ...resultProperties(props),
      moment_id: props.hit.id,
      moment_start_ms: props.hit.t_start_ms,
      match_kind: props.hit.match_kind,
      value,
    });
  };

  return (
    <div
      class={`${
        props.bounded ? "moment-row " : ""
      }grid min-w-0 max-w-full grid-cols-[minmax(0,1fr)_auto] items-stretch border bg-vesper-bg transition-colors ${
        props.selected
          ? "border-vesper-yellow"
          : "border-vesper-border hover:border-vesper-yellow/70"
      }`}
    >
      <button
        type="button"
        onClick={props.onSelect}
        class="pressable min-w-0 overflow-wrap-anywhere p-3 text-left text-sm leading-6 text-white"
      >
        <Show
          when={props.hit.match_kind === "transcript"}
          fallback={
            <p class="text-vesper-muted">
              <HighlightedText
                text={props.hit.title}
                highlight={props.hit.title_highlight}
                query={props.query}
              />
            </p>
          }
        >
          <Show when={props.hit.prev_text}>
            <span class="text-vesper-muted">{props.hit.prev_text} </span>
          </Show>
          <span class="text-white">
            <HighlightedText
              text={props.hit.text}
              highlight={props.hit.text_highlight}
              query={props.query}
            />
          </span>
          <Show when={props.hit.next_text}>
            <span class="text-vesper-muted"> {props.hit.next_text}</span>
          </Show>
        </Show>
      </button>
      <div class="flex w-10 flex-col border-s border-vesper-border">
        <MomentFeedbackButton
          value="useful"
          selected={feedback() === "useful"}
          onSelect={submitFeedback}
        />
        <MomentFeedbackButton
          value="not_useful"
          selected={feedback() === "not_useful"}
          onSelect={submitFeedback}
        />
      </div>
    </div>
  );
};

const resultProperties = (props: { video: SearchVideoResult; rank: number }) => ({
  video_id: props.video.video_id,
  creator_handle: props.video.youtuber_handle,
  content_kind: props.video.content_kind,
  rank: props.rank,
});

const MomentFeedbackButton: Component<{
  value: "useful" | "not_useful";
  selected: boolean;
  onSelect: (value: "useful" | "not_useful") => void;
}> = (props) => {
  const useful = () => props.value === "useful";
  return (
    <button
      type="button"
      onClick={() => props.onSelect(props.value)}
      aria-pressed={props.selected ? "true" : "false"}
      class={`pressable grid min-h-10 flex-1 place-items-center ${
        props.selected
          ? useful()
            ? "bg-vesper-green/15 text-vesper-green"
            : "bg-red-400/15 text-red-300"
          : useful()
            ? "text-vesper-muted hover:text-vesper-green"
            : "border-t border-vesper-border text-vesper-muted hover:text-red-300"
      }`}
      aria-label={useful() ? "Mark moment useful" : "Mark moment not useful"}
      title={useful() ? "Useful moment" : "Not useful moment"}
    >
      <Show when={useful()} fallback={<ThumbsDownIcon size={15} aria-hidden="true" />}>
        <ThumbsUpIcon size={15} aria-hidden="true" />
      </Show>
    </button>
  );
};
