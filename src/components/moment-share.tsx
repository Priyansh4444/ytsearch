import { Show, createMemo, createSignal } from "solid-js";
import type { Component } from "solid-js";
import { capture } from "../analytics";
import {
  formatClockTime,
  movedWindow,
  sharedMomentPath,
  xPostUrl,
  youtubeMomentUrl,
  type SharedMoment,
} from "../../shared/share-link";
import type { SearchHit } from "../types";
import { copyToClipboard } from "../utils";

const DEFAULT_WINDOW_SECONDS = 30;
const LEAD_IN_SECONDS = 2;
const STEP_SECONDS = 5;
const MIN_WINDOW_SECONDS = 5;

type Bound = "start" | "end";
type Copied = "none" | "page" | "youtube" | "failed";

export const MomentSharePanel: Component<{
  videoId: string;
  hit: SearchHit;
  query: string;
  hybrid: boolean;
  analytics: Record<string, boolean | number | string | undefined>;
}> = (props) => {
  const initialStart = Math.max(0, Math.floor(props.hit.t_start_ms / 1000) - LEAD_IN_SECONDS);
  const [shareWindow, setShareWindow] = createSignal({
    start: initialStart,
    end: initialStart + DEFAULT_WINDOW_SECONDS,
  });
  const start = () => shareWindow().start;
  const end = () => shareWindow().end;
  const seconds = () => end() - start();
  const [copied, setCopied] = createSignal<Copied>("none");

  const moment = createMemo((): SharedMoment => ({
    videoId: props.videoId,
    startSeconds: start(),
    endSeconds: end(),
    query: props.query,
    hybrid: props.hybrid,
  }));
  const url = createMemo(() => `${window.location.origin}${sharedMomentPath(moment())}`);
  const youtube = createMemo(() => youtubeMomentUrl(moment()));

  const move = (bound: Bound, delta: number) => {
    setCopied("none");
    setShareWindow((current) => movedWindow(current, bound, delta, MIN_WINDOW_SECONDS));
  };

  const copy = async (kind: "page" | "youtube") => {
    try {
      await copyToClipboard(kind === "page" ? url() : youtube());
      setCopied(kind);
      capture("moment_share_copied", {
        ...props.analytics,
        target: kind,
        window_seconds: seconds(),
      });
    } catch {
      setCopied("failed");
    }
  };

  return (
    <div class="moment-share" data-testid="moment-share">
      <div class="moment-share-window">
        <span class="text-vesper-muted">Clip from</span>
        <WindowBound
          label="start"
          seconds={start()}
          onEarlier={() => move("start", -STEP_SECONDS)}
          onLater={() => move("start", STEP_SECONDS)}
        />
        <span class="text-vesper-muted">to</span>
        <WindowBound
          label="end"
          seconds={end()}
          onEarlier={() => move("end", -STEP_SECONDS)}
          onLater={() => move("end", STEP_SECONDS)}
        />
        <span class="tabular-nums text-vesper-dim">{seconds()} seconds</span>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => void copy("page")}
          class="pressable moment-share-primary"
        >
          {copied() === "page" ? "Link copied" : "Copy link"}
        </button>
        <a
          href={xPostUrl(props.hit.text, url())}
          target="_blank"
          rel="noreferrer"
          class="pressable moment-share-secondary"
          onClick={() =>
            capture("moment_share_sent", { ...props.analytics, window_seconds: seconds() })
          }
        >
          Post on X
        </a>
        <button
          type="button"
          onClick={() => void copy("youtube")}
          class="pressable moment-share-secondary"
        >
          {copied() === "youtube" ? "YouTube link copied" : "Copy YouTube link"}
        </button>
        <a href={url()} target="_blank" rel="noreferrer" class="moment-share-preview">
          Preview
        </a>
      </div>
      <p class="text-xs leading-5 text-vesper-dim">
        The link opens a clip that stops on its own, with these words quoted.
      </p>
      <Show when={copied() === "failed"}>
        <p class="break-all text-xs text-vesper-muted" role="alert">
          Could not reach your clipboard. Copy this instead: {url()}
        </p>
      </Show>
    </div>
  );
};

const WindowBound: Component<{
  label: Bound;
  seconds: number;
  onEarlier: () => void;
  onLater: () => void;
}> = (props) => (
  <span class="moment-share-bound">
    <button
      type="button"
      onClick={props.onEarlier}
      aria-label={`Move ${props.label} 5 seconds earlier`}
      title="5 seconds earlier"
      class="pressable moment-share-nudge"
    >
      −
    </button>
    <span class="min-w-12 text-center font-semibold tabular-nums text-white">
      {formatClockTime(props.seconds)}
    </span>
    <button
      type="button"
      onClick={props.onLater}
      aria-label={`Move ${props.label} 5 seconds later`}
      title="5 seconds later"
      class="pressable moment-share-nudge"
    >
      +
    </button>
  </span>
);
