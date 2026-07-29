import { For, Show } from "solid-js";
import type { Component } from "solid-js";
import type { RecentSearch } from "../types";

export const RecentSearches: Component<{
  searches: RecentSearch[];
  onSelect: (search: RecentSearch) => void;
  onClear: () => void;
}> = (props) => (
  <Show when={props.searches.length > 0}>
    <section class="relative z-10 mx-auto w-full md:w-4/5 xl:w-2/3" aria-label="Recent searches">
      <div class="mb-2 flex items-center justify-between gap-3 text-xs text-vesper-dim">
        <h2 class="font-medium uppercase tracking-[0.14em]">
          Recent searches <span class="normal-case tracking-normal">· private to this device</span>
        </h2>
        <button
          type="button"
          class="pressable text-xs text-vesper-muted hover:text-vesper-yellow"
          onClick={props.onClear}
        >
          Clear history
        </button>
      </div>
      <div class="flex flex-wrap gap-2">
        <For each={props.searches.slice(0, 6)}>
          {(search) => (
            <button
              type="button"
              class="pressable flex min-h-10 max-w-56 items-center gap-1.5 border border-vesper-control bg-vesper-panel px-2.5 py-1.5 text-left text-xs text-white hover:border-vesper-yellow"
              onClick={() => props.onSelect(search)}
            >
              <span class="truncate">{search.query}</span>
              <Show when={search.youtuber || search.sort !== "relevance"}>
                <span class="shrink-0 text-vesper-dim">
                  {search.youtuber || sortLabel(search.sort)}
                </span>
              </Show>
            </button>
          )}
        </For>
      </div>
    </section>
  </Show>
);

function sortLabel(sort: RecentSearch["sort"]): string {
  return sort === "published_desc" ? "Newest" : "Oldest";
}
