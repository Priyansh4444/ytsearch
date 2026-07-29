import { For, Show, createEffect, createSignal } from "solid-js";
import type { Component } from "solid-js";
import type { IndexedChannel, SearchMode, SearchSort } from "../types";
import { ChannelAvatar, indexedCountLabel } from "./channel-profile";

export const SearchForm: Component<{
  query: string;
  sort: SearchSort;
  searchMode: SearchMode;
  semanticSearchEnabled: boolean;
  searchAvailable: boolean;
  home: boolean;
  selectedChannelName?: string;
  onQueryInput: (query: string) => void;
  onSortChange: (sort: SearchSort) => void;
  onSearchModeChange: (mode: SearchMode) => void;
  mode: "search" | "request" | "channel";
  onSubmit: () => void;
  channelSuggestions: IndexedChannel[];
  onChannelSelect: (handle: string) => void;
}> = (props) => {
  const [activeSuggestion, setActiveSuggestion] = createSignal(-1);
  const [suggestionsDismissed, setSuggestionsDismissed] = createSignal(false);
  const visibleSuggestions = () => props.channelSuggestions.slice(0, 8);
  const suggestionsVisible = () =>
    !suggestionsDismissed() &&
    props.query.trim().startsWith("@") &&
    visibleSuggestions().length > 0;

  createEffect(() => {
    setActiveSuggestion(-1);
    setSuggestionsDismissed(!props.query.trim().startsWith("@"));
  });

  const handleSearchKeyDown = (event: KeyboardEvent) => {
    if (!suggestionsVisible()) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveSuggestion((current) => (current + 1) % visibleSuggestions().length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveSuggestion((current) =>
        current <= 0 ? visibleSuggestions().length - 1 : current - 1,
      );
    } else if (event.key === "Escape") {
      event.preventDefault();
      setSuggestionsDismissed(true);
      setActiveSuggestion(-1);
    } else if (event.key === "Enter" && activeSuggestion() >= 0) {
      event.preventDefault();
      const channel = visibleSuggestions()[activeSuggestion()];
      if (channel) props.onChannelSelect(channel.handle);
    }
  };

  return (
    <form
      class={`relative z-10 w-full ${props.home ? "mx-auto md:w-4/5 xl:w-2/3" : ""}`}
      style="view-transition-name: search-controls"
      onSubmit={(event) => {
        event.preventDefault();
        props.onSubmit();
      }}
    >
      <label for="transcript-search" class="mb-2 block text-sm font-medium text-vesper-muted">
        Search transcripts
      </label>
      <div class="relative">
        <div class="search-form-grid">
          <input
            id="transcript-search"
            name="query"
            type="search"
            value={props.query}
            onInput={(event) => props.onQueryInput(event.currentTarget.value)}
            onKeyDown={handleSearchKeyDown}
            placeholder={
              props.selectedChannelName
                ? `e.g. databases in ${props.selectedChannelName}'s videos`
                : "e.g. battery gate or @creator"
            }
            role="combobox"
            aria-autocomplete="list"
            aria-expanded={suggestionsVisible()}
            aria-controls={suggestionsVisible() ? "channel-suggestions" : undefined}
            aria-activedescendant={
              activeSuggestion() >= 0 ? `channel-suggestion-${activeSuggestion()}` : undefined
            }
            aria-describedby="search-help"
            class={`search-query min-w-0 border bg-vesper-bg px-4 py-3 text-base text-white placeholder:text-vesper-dim transition-colors ${
              props.mode === "request"
                ? "border-vesper-green/70 focus:border-vesper-green"
                : "border-vesper-control focus:border-vesper-yellow"
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
                  class="min-w-0 flex-1 border border-vesper-control bg-vesper-bg px-3 py-3 text-base text-white transition-colors focus:border-vesper-yellow sm:text-sm"
                >
                  <option value="lexical">Keyword</option>
                  <option value="semantic">Semantic</option>
                </select>
              </Show>
              <select
                value={props.sort}
                onChange={(event) => props.onSortChange(event.currentTarget.value as SearchSort)}
                aria-label="Sort results"
                class="min-w-0 flex-1 border border-vesper-control bg-vesper-bg px-3 py-3 text-base text-white transition-colors focus:border-vesper-yellow sm:text-sm"
              >
                <option value="relevance">Relevant</option>
                <option value="published_desc">Newest</option>
                <option value="published_asc">Oldest</option>
              </select>
            </div>
          </Show>
          <button
            type="submit"
            disabled={props.mode === "search" && !props.searchAvailable}
            class={`pressable search-submit border px-6 py-3 font-semibold text-vesper-bg ${
              props.mode === "request"
                ? "border-vesper-green bg-vesper-green hover:bg-vesper-blue"
                : !props.searchAvailable
                  ? "cursor-not-allowed border-vesper-control bg-vesper-dim text-vesper-bg"
                  : "border-vesper-yellow bg-vesper-yellow hover:bg-vesper-orange"
            }`}
          >
            {props.mode === "request"
              ? "Request index"
              : props.mode === "channel"
                ? "Open channel"
                : !props.searchAvailable
                  ? "Available at noon"
                  : "Search"}
          </button>
        </div>
        <Show when={suggestionsVisible()}>
          <div
            id="channel-suggestions"
            role="listbox"
            aria-label="Matching creators"
            class="relative z-20 mt-2 border border-vesper-control bg-vesper-panel p-1 shadow-xl"
            data-testid="channel-suggestions"
          >
            <For each={visibleSuggestions()}>
              {(channel, index) => (
                <button
                  id={`channel-suggestion-${index()}`}
                  type="button"
                  role="option"
                  aria-selected={index() === activeSuggestion()}
                  class={`pressable flex w-full items-center gap-3 px-3 py-2 text-left text-sm text-vesper-muted hover:bg-vesper-panel2 hover:text-white ${
                    index() === activeSuggestion() ? "bg-vesper-panel2 text-white" : ""
                  }`}
                  onPointerEnter={() => setActiveSuggestion(index())}
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
          <Show
            when={props.mode !== "request"}
            fallback={
              <>No indexed creator matches this handle. Press Enter to request indexing approval.</>
            }
          >
            Search everything, select a creator, or begin with{" "}
            <span class="text-vesper-yellow">@</span> to filter by channel.
          </Show>
        </Show>
      </p>
    </form>
  );
};
