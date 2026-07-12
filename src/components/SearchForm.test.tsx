import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, test, vi } from "vitest";
import type { IndexedChannel } from "../types";
import { SearchForm } from "./SearchForm";

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
  test("submits on Enter without reloading the page", async () => {
    const onSubmit = vi.fn();
    render(() => (
      <SearchForm
        query="solid js"
        sort="relevance"
        mode="search"
        onQueryInput={() => undefined}
        onSortChange={() => undefined}
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
        mode="channel"
        onQueryInput={() => undefined}
        onSortChange={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[channel]}
        onChannelSelect={onChannelSelect}
      />
    ));

    await fireEvent.click(screen.getByRole("button", { name: /marimo.*@marimo-team/i }));
    expect(onChannelSelect).toHaveBeenCalledWith("@marimo-team");
  });
});
