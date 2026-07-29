import { afterEach, describe, expect, it, vi } from "vitest";
import { searchAvailability } from "./search-availability";

const healthUrl = "https://typesense.internal/health";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("search availability", () => {
  it.each([
    ["2026-01-15T19:44:00Z", "scheduled_offline", "closed"],
    ["2026-01-15T19:45:00Z", "prewarming", "prewarm"],
  ] as const)("returns %s as %s", async (now, status, phase) => {
    const result = await searchAvailability(
      {
        SEARCH_SCHEDULE_MODE: "enforce",
        TYPESENSE_HEALTH_URL: healthUrl,
      },
      new Date(now),
    );

    expect(result).toMatchObject({
      status,
      schedule_phase: phase,
      search_enabled: false,
      enforced: true,
    });
  });

  it.each([
    [true, "ready", true],
    [false, "starting", false],
  ] as const)("reports open health=%s as %s", async (healthy, status, searchEnabled) => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response(null, { status: healthy ? 200 : 503 })),
    );

    const result = await searchAvailability(
      {
        SEARCH_SCHEDULE_MODE: "enforce",
        TYPESENSE_HEALTH_URL: healthUrl,
      },
      new Date("2026-01-15T20:00:00Z"),
    );

    expect(result).toMatchObject({
      status,
      schedule_phase: "open",
      search_enabled: searchEnabled,
    });
  });

  it("reports degraded without enforcing the schedule when the always-warm origin is unhealthy", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 503 })));

    const result = await searchAvailability(
      {
        SEARCH_SCHEDULE_MODE: "observe",
        TYPESENSE_HEALTH_URL: healthUrl,
      },
      new Date("2026-01-15T18:00:00Z"),
    );

    expect(result).toMatchObject({
      status: "degraded",
      schedule_phase: "closed",
      search_enabled: false,
      enforced: false,
    });
  });
});
