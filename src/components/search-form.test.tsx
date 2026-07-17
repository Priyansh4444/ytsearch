import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, test, vi } from "vitest";
import type { IndexedChannel } from "../types";
import { SearchForm } from "./search-form";

const channel: IndexedChannel = {
  id: "channel-1",
  handle: "@marimo-team",
  name: "marimo",
  channel_id: "UC1",
  public_video_count: 100,
  indexed_video_count: 80,
  indexed_chunk_count: 800,
};

describe("SearchForm", () => {
  test("does not advertise semantic search when the experiment is disabled", () => {
    render(() => (
      <SearchForm
        query="database failures"
        sort="relevance"
        searchMode="lexical"
        semanticSearchEnabled={false}
        home={false}
        mode="search"
        onQueryInput={() => undefined}
        onSortChange={() => undefined}
        onSearchModeChange={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[]}
        onChannelSelect={() => undefined}
      />
    ));

    expect(screen.queryByRole("combobox", { name: "Search mode" })).not.toBeInTheDocument();
  });

  test("submits on Enter without reloading the page", async () => {
    const onSubmit = vi.fn();
    render(() => (
      <SearchForm
        query="solid js"
        sort="relevance"
        searchMode="lexical"
        semanticSearchEnabled={false}
        home={false}
        mode="search"
        onQueryInput={() => undefined}
        onSortChange={() => undefined}
        onSearchModeChange={() => undefined}
        onSubmit={onSubmit}
        channelSuggestions={[]}
        onChannelSelect={() => undefined}
      />
    ));

    await fireEvent.submit(screen.getByRole("button", { name: "Search" }).closest("form")!);
    expect(onSubmit).toHaveBeenCalledOnce();
  });

  test("selects the matching channel suggestion", async () => {
    const onChannelSelect = vi.fn();
    render(() => (
      <SearchForm
        query="@marimo"
        sort="relevance"
        searchMode="lexical"
        semanticSearchEnabled={false}
        home={false}
        mode="channel"
        onQueryInput={() => undefined}
        onSortChange={() => undefined}
        onSearchModeChange={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[channel]}
        onChannelSelect={onChannelSelect}
      />
    ));

    await fireEvent.click(screen.getByRole("button", { name: /marimo.*@marimo-team/i }));
    expect(onChannelSelect).toHaveBeenCalledWith("@marimo-team");
  });

  test("offers semantic search without making it the default", async () => {
    const onSearchModeChange = vi.fn();
    render(() => (
      <SearchForm
        query="database failures"
        sort="relevance"
        searchMode="lexical"
        semanticSearchEnabled={true}
        home={false}
        mode="search"
        onQueryInput={() => undefined}
        onSortChange={() => undefined}
        onSearchModeChange={onSearchModeChange}
        onSubmit={() => undefined}
        channelSuggestions={[]}
        onChannelSelect={() => undefined}
      />
    ));

    const selector = screen.getByRole("combobox", { name: "Search mode" });
    expect(selector).toHaveValue("lexical");
    await fireEvent.change(selector, { target: { value: "semantic" } });
    expect(onSearchModeChange).toHaveBeenCalledWith("semantic");
  });
});
