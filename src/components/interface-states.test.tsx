import { fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { RecentSearch, SearchAvailability } from "../types";
import { AvailabilityNotice } from "./availability-notice";
import { EmptyState, ErrorState, LoadingState, WelcomeState } from "./states";
import { RecentSearches } from "./recent-searches";

const localTime = (iso: string): string =>
  new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(iso));

const availability = (
  status: SearchAvailability["status"],
  searchEnabled = status === "ready",
): SearchAvailability => ({
  status,
  search_enabled: searchEnabled,
  enforced: true,
  schedule_phase: searchEnabled ? "open" : "closed",
  time_zone: "America/Los_Angeles",
  daily_window: {
    opens: "12:00",
    closes: "00:00",
    label: "12:00 PM–12:00 AM Pacific Time",
  },
  next_transition_at: "2026-07-31T07:00:00.000Z",
});

describe("interface states", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-30T19:00:00.000Z"));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  test.each([
    [
      "ready",
      `Search is available now. Expires at ${localTime("2026-07-31T07:00:00.000Z")} in 12h 0m 0s.`,
    ],
    [
      "scheduled_offline",
      `Search returns at ${localTime("2026-07-31T19:00:00.000Z")} in 24h 0m 0s.`,
    ],
    [
      "prewarming",
      `Search is warming up and will be available at ${localTime("2026-07-31T19:00:00.000Z")} in 24h 0m 0s.`,
    ],
    ["starting", "Search is starting. Please check back shortly."],
    ["degraded", "search service is currently having trouble"],
  ] as const)("describes %s availability", (status, message) => {
    render(() => <AvailabilityNotice availability={availability(status)} />);
    expect(screen.getByRole("status")).toHaveTextContent(message);
  });

  test("shows times in the viewer's local time without a time zone", () => {
    render(() => <AvailabilityNotice availability={availability("ready")} />);
    expect(screen.getByRole("status").textContent).not.toMatch(/\b(PST|PDT|Pacific)\b/);
  });

  test("renders accessible loading, error, empty, and welcome states", () => {
    const { unmount } = render(() => <LoadingState />);
    expect(screen.getByRole("status", { name: "Loading search results" })).toBeInTheDocument();
    unmount();

    const error = render(() => (
      <ErrorState title="Search response error" message="Search returned an unexpected response." />
    ));
    expect(screen.getByRole("alert")).toHaveTextContent("Search response error");
    expect(screen.getByRole("alert")).toHaveTextContent("unexpected response");
    error.unmount();

    const empty = render(() => <EmptyState query="missing phrase" />);
    expect(screen.getByText(/No moments found for “missing phrase”/)).toBeInTheDocument();
    empty.unmount();

    render(() => <WelcomeState />);
    expect(screen.getByText("Search an indexed creator.")).toBeInTheDocument();
  });
});

describe("availability notice time zones", () => {
  const zones = [
    ["America/Los_Angeles", "12:00 AM"],
    ["America/New_York", "3:00 AM"],
    ["Europe/Berlin", "9:00 AM"],
    ["Asia/Tokyo", "4:00 PM"],
  ] as const;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-30T19:00:00.000Z"));
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  const withTimeZone = <T,>(timeZone: string, run: () => T): T => {
    const previous = process.env.TZ;
    process.env.TZ = timeZone;
    try {
      return run();
    } finally {
      process.env.TZ = previous;
    }
  };

  test.each(zones)("shows the expiry in the viewer's %s zone", (timeZone, expiresAt) => {
    withTimeZone(timeZone, () => {
      render(() => <AvailabilityNotice availability={availability("ready")} />);
      const status = screen.getByRole("status");
      expect(status).toHaveTextContent(
        `Search is available now. Expires at ${expiresAt} in 12h 0m 0s.`,
      );
      expect(status.textContent).not.toMatch(/\b(PST|PDT|Pacific)\b/);
    });
  });

  test.each(zones)("shows the next opening in the viewer's %s zone", (timeZone) => {
    const opensAt = withTimeZone(timeZone, () =>
      new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(
        new Date("2026-07-31T19:00:00.000Z"),
      ),
    );
    withTimeZone(timeZone, () => {
      render(() => <AvailabilityNotice availability={availability("scheduled_offline")} />);
      expect(screen.getByRole("status")).toHaveTextContent(
        `Search returns at ${opensAt} in 24h 0m 0s.`,
      );
    });
  });
});

describe("recent searches", () => {
  const searches: RecentSearch[] = [
    {
      query: "rust ownership",
      youtuber: "@example",
      sort: "relevance",
      searchedAt: "2026-07-30T00:00:00.000Z",
    },
    {
      query: "workers",
      youtuber: "",
      sort: "published_desc",
      searchedAt: "2026-07-29T00:00:00.000Z",
    },
  ];

  test("selects and clears device-local history", () => {
    const onSelect = vi.fn();
    const onClear = vi.fn();
    render(() => <RecentSearches searches={searches} onSelect={onSelect} onClear={onClear} />);

    fireEvent.click(screen.getByRole("button", { name: /rust ownership/i }));
    expect(onSelect).toHaveBeenCalledWith(searches[0]);
    fireEvent.click(screen.getByRole("button", { name: "Clear history" }));
    expect(onClear).toHaveBeenCalledOnce();
    expect(screen.getByText("Newest")).toBeInTheDocument();
  });

  test("does not render an empty history section", () => {
    render(() => (
      <RecentSearches searches={[]} onSelect={() => undefined} onClear={() => undefined} />
    ));
    expect(screen.queryByRole("region", { name: "Recent searches" })).not.toBeInTheDocument();
  });
});
