import { For, Show, createSignal } from "solid-js";
import type { Component } from "solid-js";
import { ThumbsDown, ThumbsUp } from "lucide-solid";
import { capture } from "../analytics";
import type { SearchHit, SearchVideoResult } from "../types";
import { copyToClipboard, formatDate } from "../utils";
import { HighlightedText } from "./HighlightedText";
import { CopyLinkButton } from "./ToolbarActions";

export const VideoResult: Component<{ video: SearchVideoResult; query: string; rank: number }> = (
  props,
) => {
  const visibleHits = () => props.video.hits.slice(0, 3);
  const [selectedHitId, setSelectedHitId] = createSignal<string>();
  const selectedHit = () =>
    visibleHits().find((hit) => hit.id === selectedHitId()) ??
    visibleHits()[0] ??
    props.video.hits[0];
  const [copied, setCopied] = createSignal(false);
  const copyVideoLink = async () => {
    await copyToClipboard(selectedHit()?.youtube_url ?? props.video.youtube_url);
    capture("result_link_copied", resultProperties(props));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1_500);
  };

  return (
    <article class="border border-vesper-border bg-vesper-panel transition duration-300 hover:border-vesper-dim">
      <div class="grid gap-0 xl:grid-cols-[minmax(32rem,40rem)_1fr]">
        <div class="relative bg-vesper-bg">
          <iframe
            class="aspect-video w-full"
            src={selectedHit()?.embed_url ?? props.video.embed_url}
            title={`YouTube player for ${props.video.title}`}
            loading="lazy"
            allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share"
            referrerpolicy="strict-origin-when-cross-origin"
            allowfullscreen
          />
          <span class="absolute bottom-2 left-2 bg-vesper-bg/90 px-2 py-1 text-xs font-semibold text-vesper-yellow ring-1 ring-vesper-border">
            {selectedHit()?.timestamp ?? "title"}
          </span>
        </div>

        <div class="flex flex-col gap-3 p-4">
          <div class="flex items-start justify-between gap-4">
            <div class="min-w-0">
              <a
                href={props.video.youtube_url}
                target="_blank"
                rel="noreferrer"
                class="text-lg font-semibold leading-snug text-white transition hover:text-vesper-yellow"
                onClick={() => capture("result_opened", resultProperties(props))}
              >
                <HighlightedText
                  text={props.video.title}
                  highlight={props.video.title_highlight}
                  query={props.query}
                />
              </a>
              <p class="mt-1 text-sm text-vesper-muted">
                {props.video.youtuber_name} · {props.video.youtuber_handle}
                <Show when={props.video.published_at}>
                  {" "}
                  · {formatDate(props.video.published_at)}
                </Show>
              </p>
            </div>
            <div class="flex shrink-0 items-center gap-2">
              <CopyLinkButton
                copied={copied()}
                onClick={() => void copyVideoLink()}
                label="Copy video link"
              />
              <a
                href={selectedHit()?.youtube_url ?? props.video.youtube_url}
                target="_blank"
                rel="noreferrer"
                class="inline-flex h-10 items-center border border-vesper-border bg-vesper-bg px-3 text-sm font-medium text-vesper-green transition hover:border-vesper-green"
                onClick={() => capture("result_opened", resultProperties(props))}
              >
                Open
              </a>
            </div>
          </div>

          <div class="grid gap-2">
            <For each={visibleHits()}>
              {(hit) => (
                <TranscriptExcerpt
                  hit={hit}
                  query={props.query}
                  selected={hit.id === selectedHit()?.id}
                  onSelect={() => {
                    setSelectedHitId(hit.id);
                    capture("result_timestamp_selected", {
                      ...resultProperties(props),
                      match_kind: hit.match_kind,
                    });
                  }}
                />
              )}
            </For>
          </div>
          <RelevanceFeedback video={props.video} rank={props.rank} />
        </div>
      </div>
    </article>
  );
};

const TranscriptExcerpt: Component<{
  hit: SearchHit;
  query: string;
  selected?: boolean;
  onSelect?: () => void;
}> = (props) => (
  <button
    type="button"
    onClick={props.onSelect}
    class={`border bg-vesper-bg p-3 text-left text-sm leading-6 text-white transition ${
      props.selected ? "border-vesper-yellow" : "border-vesper-border hover:border-vesper-yellow/70"
    }`}
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
);

const resultProperties = (props: { video: SearchVideoResult; rank: number }) => ({
  video_id: props.video.video_id,
  creator_handle: props.video.youtuber_handle,
  content_kind: props.video.content_kind,
  rank: props.rank,
});

const RelevanceFeedback: Component<{ video: SearchVideoResult; rank: number }> = (props) => {
  const [feedback, setFeedback] = createSignal<"useful" | "not_useful">();
  const submit = (value: "useful" | "not_useful") => {
    setFeedback(value);
    capture("result_relevance_feedback", { ...resultProperties(props), value });
  };

  return (
    <div class="flex items-center gap-1 border-t border-vesper-border pt-3">
      <button
        type="button"
        onClick={() => submit("useful")}
        class={`inline-flex size-8 items-center justify-center border transition ${feedback() === "useful" ? "border-vesper-green bg-vesper-green/15 text-vesper-green" : "border-vesper-border text-vesper-muted hover:border-vesper-green hover:text-vesper-green"}`}
        aria-label="Mark result useful"
        title="Useful result"
      >
        <ThumbsUp size={15} aria-hidden="true" />
      </button>
      <button
        type="button"
        onClick={() => submit("not_useful")}
        class={`inline-flex size-8 items-center justify-center border transition ${feedback() === "not_useful" ? "border-red-400 bg-red-400/15 text-red-300" : "border-vesper-border text-vesper-muted hover:border-red-400 hover:text-red-300"}`}
        aria-label="Mark result not useful"
        title="Not useful result"
      >
        <ThumbsDown size={15} aria-hidden="true" />
      </button>
    </div>
  );
};
