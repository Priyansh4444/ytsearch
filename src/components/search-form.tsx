import { For, Show } from "solid-js";
import type { Component } from "solid-js";
import type { IndexedChannel, SearchMode, SearchSort } from "../types";
import { ChannelAvatar, indexedCountLabel } from "./channel-profile";

export const SearchForm: Component<{
  query: string;
  sort: SearchSort;
  searchMode: SearchMode;
  semanticSearchEnabled: boolean;
  home: boolean;
  selectedChannelName?: string;
  onQueryInput: (query: string) => void;
  onSortChange: (sort: SearchSort) => void;
  onSearchModeChange: (mode: SearchMode) => void;
  mode: "search" | "request" | "channel";
  onSubmit: () => void;
  channelSuggestions: IndexedChannel[];
  onChannelSelect: (handle: string) => void;
}> = (props) => (
  <form
    class={`relative z-10 w-full ${props.home ? "mx-auto md:w-4/5 xl:w-2/3" : ""}`}
    style="view-transition-name: search-controls"
    onSubmit={(event) => {
      event.preventDefault();
      props.onSubmit();
    }}
  >
    <div class="relative">
      <div class="search-form-grid">
        <input
          value={props.query}
          onInput={(event) => props.onQueryInput(event.currentTarget.value)}
          placeholder={
            props.mode === "request"
              ? "No match yet — press Enter to request this @handle"
              : props.selectedChannelName
                ? `Search ${props.selectedChannelName}'s transcripts…`
                : "Search transcripts or type @creator"
          }
          aria-describedby="search-help"
          class={`search-query min-w-0 border bg-vesper-bg px-4 py-3 text-base text-white outline-none placeholder:text-vesper-dim transition-colors ${
            props.mode === "request"
              ? "border-vesper-green/70 focus:border-vesper-green"
              : "border-vesper-border focus:border-vesper-yellow"
          }`}
        />
        <Show when={props.mode === "search"}>
          <div class="search-options flex min-w-0 gap-2">
            <Show when={props.semanticSearchEnabled}>
              <select
                value={props.searchMode}
                onChange={(event) =>
                  props.onSearchModeChange(event.currentTarget.value as SearchMode)
                }
                aria-label="Search mode"
                class="min-w-0 flex-1 border border-vesper-border bg-vesper-bg px-3 py-3 text-sm text-white outline-none transition-colors focus:border-vesper-yellow"
              >
                <option value="lexical">Keyword</option>
                <option value="semantic">Semantic</option>
              </select>
            </Show>
            <select
              value={props.sort}
              onChange={(event) => props.onSortChange(event.currentTarget.value as SearchSort)}
              aria-label="Sort results"
              class="min-w-0 flex-1 border border-vesper-border bg-vesper-bg px-3 py-3 text-sm text-white outline-none transition-colors focus:border-vesper-yellow"
            >
              <option value="relevance">Relevant</option>
              <option value="published_desc">Newest</option>
              <option value="published_asc">Oldest</option>
            </select>
          </div>
        </Show>
        <button
          type="submit"
          class={`search-submit border px-6 py-3 font-semibold text-vesper-bg transition-colors ${
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
    <p id="search-help" class="mt-2 text-center text-xs text-vesper-dim sm:text-left">
      <Show
        when={!props.selectedChannelName}
        fallback={`Searching only ${props.selectedChannelName}. Go back to search every channel.`}
      >
        Search everything, click a creator, or begin with <span class="text-vesper-yellow">@</span>{" "}
        to filter by channel.
      </Show>
    </p>
  </form>
);
