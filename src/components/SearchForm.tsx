import type { Component } from "solid-js";
import type { SearchSort } from "../types";

export const SearchForm: Component<{
  query: string;
  sort: SearchSort;
  onQueryInput: (query: string) => void;
  onSortChange: (sort: SearchSort) => void;
  onSubmit: () => void;
}> = (props) => (
  <form
    class="transition-all duration-500 ease-out"
    onSubmit={(event) => {
      event.preventDefault();
      props.onSubmit();
    }}
  >
    <div class="flex flex-col gap-3 sm:flex-row">
      <input
        value={props.query}
        onInput={(event) => props.onQueryInput(event.currentTarget.value)}
        placeholder="Search transcripts..."
        class="h-12 min-w-0 flex-1 border border-vesper-border bg-vesper-bg px-4 text-base text-white outline-none placeholder:text-vesper-dim transition focus:border-vesper-yellow"
      />
      <select
        value={props.sort}
        onChange={(event) => props.onSortChange(event.currentTarget.value as SearchSort)}
        class="h-12 border border-vesper-border bg-vesper-bg px-3 text-base text-white outline-none transition focus:border-vesper-yellow"
      >
        <option value="relevance">Relevant</option>
        <option value="published_desc">Newest</option>
        <option value="published_asc">Oldest</option>
      </select>
      <button
        type="submit"
        class="h-12 border border-vesper-yellow bg-vesper-yellow px-6 font-semibold text-vesper-bg transition hover:bg-vesper-orange"
      >
        Search
      </button>
    </div>
  </form>
);
