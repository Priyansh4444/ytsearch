import { For } from "solid-js";
import type { Component } from "solid-js";

export const LoadingState: Component = () => (
  <div class="grid gap-2" role="status" aria-label="Loading search results">
    <For each={[0, 1, 2]}>
      {() => (
        <div
          class="grid overflow-hidden border border-vesper-border bg-vesper-panel motion-safe:animate-pulse xl:grid-cols-[minmax(32rem,40rem)_1fr]"
          aria-hidden="true"
        >
          <div class="aspect-video bg-vesper-panel2" />
          <div class="flex flex-col gap-4 p-4">
            <div class="h-6 w-3/4 bg-white/10" />
            <div class="h-4 w-2/5 bg-white/5" />
            <div class="mt-auto grid gap-2">
              <div class="h-16 bg-white/5" />
              <div class="h-16 bg-white/5" />
            </div>
          </div>
        </div>
      )}
    </For>
  </div>
);

export const ErrorState: Component<{ message: string; title?: string }> = (props) => (
  <div class="border border-vesper-red/40 bg-vesper-red/10 p-5 text-vesper-red" role="alert">
    <p class="font-semibold">{props.title ?? "Search failed"}</p>
    <p class="mt-2 text-sm text-red-100/80">{props.message}</p>
  </div>
);

export const EmptyState: Component<{ query: string }> = (props) => (
  <div class="py-16 text-center">
    <p class="text-xl font-semibold text-white">No moments found for “{props.query}”</p>
    <p class="mx-auto mt-2 max-w-xl text-pretty text-vesper-muted">
      Try fewer words, choose another creator, or type @creator to request indexing.
    </p>
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
