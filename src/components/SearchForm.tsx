import { For, Show } from "solid-js";
import type { Component } from "solid-js";
import type { IndexedChannel, SearchSort } from "../types";
import { ChannelAvatar, indexedCountLabel } from "./ChannelProfile";

export const SearchForm: Component<{
  query: string;
  sort: SearchSort;
  onQueryInput: (query: string) => void;
  onSortChange: (sort: SearchSort) => void;
  mode: "search" | "request" | "channel";
  onSubmit: () => void;
  channelSuggestions: IndexedChannel[];
  onChannelSelect: (handle: string) => void;
}> = (props) => (
  <form
    class="transition-all duration-500 ease-out"
    onSubmit={(event) => {
      event.preventDefault();
      props.onSubmit();
    }}
  >
    <div class="relative">
      <div
        class={`flex flex-col gap-3 rounded-sm transition-all duration-500 ease-out sm:flex-row ${
          props.mode === "request" ? "scale-[1.01]" : "scale-100"
        }`}
      >
        <input
          value={props.query}
          onInput={(event) => props.onQueryInput(event.currentTarget.value)}
          placeholder={
            props.mode === "request" ? "Find a channel or request @handle" : "Search transcripts..."
          }
          class={`h-12 min-w-0 flex-1 border bg-vesper-bg px-4 text-base text-white outline-none placeholder:text-vesper-dim transition duration-300 ${
            props.mode === "request"
              ? "border-vesper-green/70 focus:border-vesper-green"
              : "border-vesper-border focus:border-vesper-yellow"
          }`}
        />
        <Show when={props.mode === "search"}>
          <select
            value={props.sort}
            onChange={(event) => props.onSortChange(event.currentTarget.value as SearchSort)}
            class="h-12 border border-vesper-border bg-vesper-bg px-3 text-base text-white outline-none transition focus:border-vesper-yellow"
          >
            <option value="relevance">Relevant</option>
            <option value="published_desc">Newest</option>
            <option value="published_asc">Oldest</option>
          </select>
        </Show>
        <button
          type="submit"
          class={`h-12 border px-6 font-semibold text-vesper-bg transition duration-300 ${
            props.mode === "request"
              ? "border-vesper-green bg-vesper-green hover:bg-vesper-blue"
              : "border-vesper-yellow bg-vesper-yellow hover:bg-vesper-orange"
          }`}
        >
          {props.mode === "request"
            ? "Request index"
            : props.mode === "channel"
              ? "Open channel"
              : "Search"}
        </button>
      </div>
      <Show when={props.query.trim().startsWith("@") && props.channelSuggestions.length > 0}>
        <div class="absolute inset-x-0 top-14 z-20 border border-vesper-border bg-vesper-panel p-1 shadow-xl">
          <For each={props.channelSuggestions.slice(0, 8)}>
            {(channel) => (
              <button
                type="button"
                class="flex w-full items-center gap-3 px-3 py-2 text-left text-sm text-vesper-muted transition hover:bg-vesper-panel2 hover:text-white"
                onClick={() => props.onChannelSelect(channel.handle)}
              >
                <ChannelAvatar channel={channel} size="compact" />
                <span class="min-w-0 flex-1">
                  <span class="block truncate font-medium text-white">{channel.name}</span>
                  <span class="block truncate text-xs text-vesper-dim">
                    {channel.handle} · {indexedCountLabel(channel)}
                  </span>
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  </form>
);
