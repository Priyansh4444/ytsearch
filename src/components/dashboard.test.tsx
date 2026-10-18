import { fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { beforeEach, describe, expect, test, vi } from "vitest";
import type { DashboardData } from "../types";
import { Dashboard } from "./dashboard";

const { dashboardData } = vi.hoisted(() => ({
  dashboardData: vi.fn<() => Promise<DashboardData>>(),
}));

vi.mock("../api", () => ({
  API_BASE_URL: "https://api.example.test",
  dashboardData,
}));

const snapshot: DashboardData = {
  channels: [
    {
      id: "channel-a",
      handle: "@alpha",
      name: "Alpha",
      channel_id: "UC0000000000000000000001",
      uploads_playlist: "UU0000000000000000000001",
      public_video_count: 10,
      indexed_video_count: 8,
      indexed_chunk_count: 80,
      avatar_url: "https://yt3.ggpht.com/alpha",
    },
    {
      id: "channel-b",
      handle: "@beta",
      name: "Beta",
      channel_id: "UC0000000000000000000002",
      public_video_count: 5,
      indexed_video_count: 5,
      indexed_chunk_count: 25,
    },
  ],
  channelsAvailable: true,
  indexStatus: { handle: "", status: "idle" },
  apiHealthy: true,
  fetchedAt: "2026-07-30T20:00:00.000Z",
  latencyMs: { health: 12, channels: 20, indexStatus: 8 },
  errors: [],
};

describe("Dashboard", () => {
  beforeEach(() => {
    dashboardData.mockReset();
    dashboardData.mockResolvedValue(snapshot);
  });

  test("renders public health and honest count comparisons", async () => {
    render(() => <Dashboard />);

    expect(await screen.findByRole("heading", { name: "System dashboard" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("Operational")).toBeInTheDocument());
    expect(screen.getByText("Idle")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Understanding the count difference" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/A count difference does not identify missing/)).toBeInTheDocument();
    expect(screen.getByText("8 / 10")).toBeInTheDocument();
    expect(
      screen.queryByText(/Scheduled repair|Caught up|Fully indexed|videos remaining/),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Alpha/ })).toHaveAttribute("href", "/alpha");
    expect(document.title).toBe("Dashboard · YouTube Search");
  });

  test("refreshes on demand and reports partial failures", async () => {
    dashboardData.mockResolvedValueOnce(snapshot).mockResolvedValueOnce({
      ...snapshot,
      apiHealthy: false,
      errors: ["Health check unavailable"],
    });
    render(() => <Dashboard />);

    const refresh = await screen.findByRole("button", { name: "Refresh metrics" });
    fireEvent.click(refresh);
    expect(await screen.findByRole("alert")).toHaveTextContent("Health check unavailable");
    expect(dashboardData).toHaveBeenCalledTimes(2);
    expect(screen.getByRole("link", { name: /Alpha/ })).toHaveAttribute("href", "/alpha");
  });
  test("searches and filters creators, sorts the table, and reports no matches", async () => {
    dashboardData.mockResolvedValue({
      ...snapshot,
      channels: [
        snapshot.channels[0],
        { ...snapshot.channels[1], public_video_count: 12, indexed_video_count: 12 },
      ],
    });
    render(() => <Dashboard />);
    const table = await screen.findByRole("table", { name: "Creator catalog" });
    const creatorNames = () =>
      within(table)
        .getAllByRole("link")
        .map((link) => link.textContent);
    expect(creatorNames()[0]).toContain("Alpha");

    fireEvent.input(screen.getByRole("searchbox", { name: "Find a creator" }), {
      target: { value: " @BETA " },
    });
    await waitFor(() => expect(creatorNames()).toHaveLength(1));
    expect(creatorNames()[0]).toContain("Beta");
    fireEvent.input(screen.getByRole("searchbox"), { target: { value: "" } });
    fireEvent.change(screen.getByRole("combobox", { name: "Filter creators" }), {
      target: { value: "difference" },
    });
    await waitFor(() => expect(creatorNames()).toHaveLength(1));
    expect(creatorNames()[0]).toContain("Alpha");
    fireEvent.change(screen.getByRole("combobox", { name: "Filter creators" }), {
      target: { value: "all" },
    });
    fireEvent.change(screen.getByRole("combobox", { name: "Sort creators" }), {
      target: { value: "videos" },
    });
    await waitFor(() => expect(creatorNames()).toHaveLength(2));
    expect(creatorNames()[0]).toContain("Beta");
    fireEvent.change(screen.getByRole("combobox", { name: "Sort creators" }), {
      target: { value: "name" },
    });
    await waitFor(() => expect(creatorNames()[0]).toContain("Alpha"));
    fireEvent.input(screen.getByRole("searchbox"), { target: { value: "no such creator" } });
    expect(await screen.findByText("No creators match these filters.")).toBeInTheDocument();
  });

  test("keeps creator view controls through a metrics refresh", async () => {
    render(() => <Dashboard />);
    const search = await screen.findByRole("searchbox");
    fireEvent.input(search, { target: { value: "Beta" } });
    fireEvent.change(screen.getByRole("combobox", { name: "Sort creators" }), {
      target: { value: "videos" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Refresh metrics" }));
    await waitFor(() => expect(dashboardData).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("searchbox")).toHaveValue("Beta");
    expect(screen.getByRole("combobox", { name: "Sort creators" })).toHaveValue("videos");
    expect(screen.queryByRole("link", { name: /Alpha/ })).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Beta/ })).toBeInTheDocument();
  });

  test("shows unavailable public totals as unknown rather than complete coverage", async () => {
    dashboardData.mockResolvedValue({
      ...snapshot,
      channels: [{ ...snapshot.channels[0], public_video_count: 0 }],
    });
    render(() => <Dashboard />);
    expect(await screen.findByText("8 / Unknown")).toBeInTheDocument();
    expect(screen.getByText(/Public totals unknown for 1 creators/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("combobox", { name: "Filter creators" }), {
      target: { value: "unknown" },
    });
    expect(screen.getByRole("link", { name: /Alpha/ })).toBeInTheDocument();
    expect(screen.queryByText("100%")).not.toBeInTheDocument();
  });

  test("does not present an unavailable catalog as an empty indexed archive", async () => {
    dashboardData.mockResolvedValue({
      ...snapshot,
      channels: [],
      channelsAvailable: false,
      errors: ["Catalog check unavailable"],
    });
    render(() => <Dashboard />);
    expect(
      await screen.findByText("Creator data is unavailable. Refresh to try again."),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Catalog check unavailable");
    expect(screen.queryByText("0 of 0 creators")).not.toBeInTheDocument();
  });
});
