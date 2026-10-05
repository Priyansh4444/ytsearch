import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
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
  test("submits on Enter without reloading the page", async () => {
    const onSubmit = vi.fn();
    render(() => (
      <SearchForm
        query="solid js"
        searchAvailable={true}
        home={false}
        mode="search"
        onQueryInput={() => undefined}
        onSubmit={onSubmit}
        channelSuggestions={[]}
        onChannelSelect={() => undefined}
      />
    ));

    fireEvent.submit(screen.getByRole("button", { name: "Search" }).closest("form")!);
    expect(onSubmit).toHaveBeenCalledOnce();
  });

  test("selects the matching channel suggestion", async () => {
    const onChannelSelect = vi.fn();
    render(() => (
      <SearchForm
        query="@marimo"
        searchAvailable={true}
        home={false}
        mode="channel"
        onQueryInput={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[channel]}
        onChannelSelect={onChannelSelect}
      />
    ));

    fireEvent.click(screen.getByRole("option", { name: /marimo.*@marimo-team/i }));
    expect(onChannelSelect).toHaveBeenCalledWith("@marimo-team");
  });

  test("names the search field and supports suggestion keyboard navigation", async () => {
    const onChannelSelect = vi.fn();
    render(() => (
      <SearchForm
        query="@marimo"
        searchAvailable={true}
        home={true}
        mode="channel"
        onQueryInput={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[channel]}
        onChannelSelect={onChannelSelect}
      />
    ));

    const search = screen.getByRole("combobox", { name: "Search transcripts" });
    expect(search).toHaveAttribute("aria-expanded", "true");
    fireEvent.keyDown(search, { key: "ArrowDown" });
    await waitFor(() =>
      expect(search).toHaveAttribute("aria-activedescendant", "channel-suggestion-0"),
    );
    fireEvent.keyDown(search, { key: "Enter" });
    expect(onChannelSelect).toHaveBeenCalledWith("@marimo-team");
  });

  test("disables transcript search while leaving channel mode usable", () => {
    const { unmount } = render(() => (
      <SearchForm
        query="solid js"
        searchAvailable={false}
        home={true}
        mode="search"
        onQueryInput={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[]}
        onChannelSelect={() => undefined}
      />
    ));

    expect(screen.getByRole("button", { name: "Available at noon" })).toBeDisabled();
    unmount();

    render(() => (
      <SearchForm
        query="@example"
        searchAvailable={false}
        home={true}
        mode="channel"
        onQueryInput={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[]}
        onChannelSelect={() => undefined}
      />
    ));
    expect(screen.getByRole("button", { name: "Open channel" })).toBeEnabled();
  });

  test("reports and prevents duplicate submission while searching", () => {
    render(() => (
      <SearchForm
        query="solid js"
        searchAvailable={true}
        busy={true}
        home={false}
        mode="search"
        onQueryInput={() => undefined}
        onSubmit={() => undefined}
        channelSuggestions={[]}
        onChannelSelect={() => undefined}
      />
    ));

    const button = screen.getByRole("button", { name: "Searching…" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("aria-busy", "true");
  });
});
