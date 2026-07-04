import { For } from "solid-js";
import type { Component } from "solid-js";

export const LoadingState: Component = () => (
  <div class="grid gap-2">
    <For each={[0, 1, 2]}>
      {() => <div class="h-40 animate-pulse border border-vesper-border bg-vesper-panel" />}
    </For>
  </div>
);

export const ErrorState: Component<{ message: string }> = (props) => (
  <div class="border border-vesper-red/40 bg-vesper-red/10 p-5 text-vesper-red">
    <p class="font-semibold">Search failed</p>
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
