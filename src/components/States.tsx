import { For, createSignal, onCleanup, onMount } from "solid-js";
import type { Component } from "solid-js";
import { indexStatus } from "../api";

export const LoadingState: Component = () => (
  <div class="grid gap-2">
    <For each={[0, 1, 2]}>
      {() => <div class="h-40 animate-pulse border border-vesper-border bg-vesper-panel" />}
    </For>
  </div>
);

export const IndexUnavailableState: Component<{ handle: string }> = (props) => {
  const tweetText = () =>
    encodeURIComponent(`Please index ${props.handle} for ytsearch ❤️ @notpronsh`);

  return (
    <div class="border border-vesper-green/30 bg-vesper-panel p-5 shadow-2xl shadow-vesper-green/5">
      <p class="text-sm uppercase tracking-[0.18em] text-vesper-green">Indexing unavailable</p>
      <p class="mt-2 text-xl font-semibold text-white">{props.handle}</p>
      <p class="mt-2 max-w-2xl text-sm text-vesper-muted">
        This feature is currently not available. To get another YouTuber indexed, tag{" "}
        <a
          class="inline-flex items-center gap-1 text-vesper-green underline decoration-vesper-green/40 underline-offset-4 transition hover:text-white"
          href={`https://twitter.com/intent/tweet?text=${tweetText()}`}
          target="_blank"
          rel="noreferrer"
        >
          <span aria-hidden="true" class="font-semibold text-white">
            𝕏
          </span>
          @notpronsh
        </a>{" "}
        on Twitter ❤️
      </p>
    </div>
  );
};

export const IndexQueuedState: Component<{ handle: string; requestId: string }> = (props) => {
  const [status, setStatus] = createSignal<"queued" | "running" | "idle">("queued");
  const refresh = () => void indexStatus(props.handle).then((value) => setStatus(value.status));
  onMount(() => {
    refresh();
    const timer = window.setInterval(refresh, 10_000);
    onCleanup(() => window.clearInterval(timer));
  });
  return (
    <div class="border border-vesper-green/30 bg-vesper-panel p-5 shadow-2xl shadow-vesper-green/5">
      <p class="text-sm uppercase tracking-[0.18em] text-vesper-green">
        {status() === "idle" ? "Index request completed" : "Currently indexing"}
      </p>
      <p class="mt-2 text-xl font-semibold text-white">{props.handle}</p>
      <p class="mt-2 max-w-2xl text-sm text-vesper-muted">
        Request {props.requestId} is{" "}
        {status() === "idle" ? "no longer queued" : "being handled by the index worker"}.
      </p>
    </div>
  );
};

export const NoChannelMatchState: Component<{ handle: string }> = (props) => (
  <div class="border border-vesper-border bg-vesper-panel p-5">
    <p class="font-semibold text-white">No indexed channel matches {props.handle}</p>
    <p class="mt-2 text-sm text-vesper-muted">
      Choose a channel from the suggestions or search without an @ filter.
    </p>
  </div>
);

export const ErrorState: Component<{ message: string; title?: string }> = (props) => (
  <div class="border border-vesper-red/40 bg-vesper-red/10 p-5 text-vesper-red">
    <p class="font-semibold">{props.title ?? "Search failed"}</p>
    <p class="mt-2 text-sm text-red-100/80">{props.message}</p>
  </div>
);

export const EmptyState: Component<{ query: string }> = (props) => (
  <div class="py-16 text-center">
    <p class="text-xl font-semibold text-white">No hits for “{props.query}”</p>
    <p class="mt-2 text-vesper-muted">Try a broader phrase or index more videos.</p>
  </div>
);

export const WelcomeState: Component = () => (
  <div class="py-16 text-center">
    <p class="text-xl font-semibold text-white">Search an indexed creator.</p>
    <p class="mx-auto mt-2 max-w-xl text-vesper-muted">
      Pick a creator, search a phrase, and jump between transcript moments inside each video.
    </p>
  </div>
);
