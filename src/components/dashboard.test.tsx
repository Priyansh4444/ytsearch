import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
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

  test("renders public health, coverage, and creator metrics", async () => {
    render(() => <Dashboard />);

    expect(await screen.findByRole("heading", { name: "System dashboard" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("Operational")).toBeInTheDocument());
    expect(screen.getByText("86.7%")).toBeInTheDocument();
    expect(screen.getByText("Scheduled repair")).toBeInTheDocument();
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
  });
});
