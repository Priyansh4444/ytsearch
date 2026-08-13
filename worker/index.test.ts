import { afterEach, describe, expect, test, vi } from "vitest";
import worker from "./index";
import { shouldWakeIndexer } from "./wake-policy";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("index request approval wake gate", () => {
  test("does not wake the indexer for a new public request", () => {
    expect(shouldWakeIndexer("POST", "/api/index-requests", true)).toBe(false);
  });

  test("wakes the indexer after an approved request", () => {
    expect(shouldWakeIndexer("POST", "/api/index-requests/request-1/approve", true)).toBe(true);
  });

  test("does not wake the indexer when approval fails upstream", () => {
    expect(shouldWakeIndexer("POST", "/api/index-requests/request-1/approve", false)).toBe(false);
  });
});

describe("scheduled repair wake", () => {
  test("puts a durable wake on the queue with schedule context in logs", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const log = vi.spyOn(console, "log").mockImplementation(() => undefined);
    const scheduledTime = Date.UTC(2026, 6, 22, 6, 17);

    await worker.scheduled(
      {
        cron: "17 */6 * * *",
        scheduledTime,
        noRetry: vi.fn(),
      },
      testEnv({ INDEX_WAKE_QUEUE: { send } }),
    );

    expect(send).toHaveBeenCalledOnce();
    expect(send).toHaveBeenCalledWith({ reason: "scheduled-repair" });
    expect(log).toHaveBeenCalledWith(
      JSON.stringify({
        event: "indexer_repair_wake_enqueued",
        cron: "17 */6 * * *",
        scheduled_time: scheduledTime,
      }),
    );
    log.mockRestore();
  });
});

describe("scheduled search availability", () => {
  test("serves static assets without consulting Fly", async () => {
    const assetResponse = new Response("app");
    const assetFetch = vi.fn().mockResolvedValue(assetResponse);
    vi.stubGlobal("fetch", vi.fn());

    const response = await worker.fetch(
      new Request("https://search.pronsh.dev/"),
      testEnv({ ASSETS: { fetch: assetFetch } }),
    );

    expect(response).toBe(assetResponse);
    expect(assetFetch).toHaveBeenCalledOnce();
    expect(fetch).not.toHaveBeenCalled();
  });

  test("blocks off-hours searches without contacting Fly or the API", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T17:00:00Z"));
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(console, "log").mockImplementation(() => undefined);

    const response = await worker.fetch(
      new Request("https://search.pronsh.dev/api/search?q=rust"),
      testEnv(),
    );
    const body = await response.json<{ code: string; available_at: string }>();

    expect(response.status).toBe(503);
    expect(response.headers.get("Retry-After")).toBe("7200");
    expect(body).toEqual({
      code: "scheduled_offline",
      error: "Search is available daily from 12:00 PM to 12:00 AM Pacific Time.",
      available_at: "2026-07-26T19:00:00.000Z",
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("bypasses every persistent cache for dynamic searches", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T20:00:00Z"));
    const fetchMock = vi
      .fn()
      .mockResolvedValue(
        Response.json({ query: "rust", videos: [] }, { headers: { "x-upstream": "yes" } }),
      );
    vi.stubGlobal("fetch", fetchMock);

    const response = await worker.fetch(
      new Request("https://search.pronsh.dev/api/search?q=rust"),
      testEnv(),
    );

    expect(response.status).toBe(200);
    expect(response.headers.get("x-upstream")).toBe("yes");
    expect(response.headers.get("Cache-Control")).toBe("no-store");
    expect(response.headers.get("CDN-Cache-Control")).toBe("no-store");
    expect(response.headers.get("Cloudflare-CDN-Cache-Control")).toBe("no-store");
    expect(fetchMock).toHaveBeenCalledOnce();
    expect((fetchMock.mock.calls[0][0] as Request).url).toContain(
      "ytsearch-api-raw-staging.fly.dev/api/search?q=rust",
    );
    expect(fetchMock.mock.calls[0][1]).toEqual({ cache: "no-store" });
  });

  test("also bypasses caches for development-origin search responses", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T20:00:00Z"));
    const fetchMock = vi.fn().mockResolvedValue(Response.json({ query: "rust", videos: [] }));
    vi.stubGlobal("fetch", fetchMock);

    await worker.fetch(
      new Request("https://search.pronsh.dev/api/search?q=rust", {
        headers: { Origin: "http://localhost:5173" },
      }),
      testEnv(),
    );

    expect(fetchMock).toHaveBeenCalledOnce();
    expect(fetchMock.mock.calls[0][1]).toEqual({ cache: "no-store" });
  });

  test("returns normalized availability without provider identifiers", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T17:00:00Z"));
    vi.stubGlobal("fetch", vi.fn());

    const response = await worker.fetch(
      new Request("https://search.pronsh.dev/api/availability"),
      testEnv(),
    );
    const text = await response.text();

    expect(JSON.parse(text)).toMatchObject({
      status: "scheduled_offline",
      search_enabled: false,
      enforced: true,
      schedule_policy: "scheduled",
      schedule_phase: "closed",
      time_zone: "America/Los_Angeles",
    });
    expect(text).not.toContain("8147deb94937e8");
    expect(text).not.toContain("fly-token");
  });

  test("starts only the configured Raw machine during prewarm", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ state: "stopped" }))
      .mockResolvedValueOnce(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);
    const log = vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.scheduled(scheduledController("*/5 * * * *", "2026-07-26T18:45:00Z"), testEnv());

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(String(fetchMock.mock.calls[0]?.[0])).toContain(
      "/apps/ytsearch-raw-production/machines/8147deb94937e8",
    );
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain(
      "/apps/ytsearch-raw-production/machines/8147deb94937e8/start",
    );
    expect(fetchMock.mock.calls.join(" ")).not.toContain("d8de466f330468");
    expect(log).toHaveBeenCalledWith(expect.stringContaining("raw_search_schedule_started"));
  });

  test("stops the indexer before Raw after midnight", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ state: "started" }))
      .mockResolvedValueOnce(Response.json([indexerMachine("started")]))
      .mockResolvedValueOnce(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);
    const log = vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.scheduled(scheduledController("*/5 * * * *", "2026-07-27T07:00:00Z"), testEnv());

    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(String(fetchMock.mock.calls[2]?.[0])).toContain(
      "/apps/ytsearch-indexer-raw-production/machines/current-indexer/stop",
    );
    expect(fetchMock.mock.calls[2]?.[1]).toEqual({
      method: "POST",
      headers: { Authorization: "Bearer fly-token" },
      signal: expect.any(AbortSignal),
    });
    expect(fetchMock.mock.calls.join(" ")).not.toContain(
      "/apps/ytsearch-raw-production/machines/8147deb94937e8/stop",
    );
    expect(log).toHaveBeenCalledWith(expect.stringContaining("indexer_schedule_stop_requested"));
  });

  test("waits for an indexer stop to finish before stopping Raw", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ state: "started" }))
      .mockResolvedValueOnce(Response.json([indexerMachine("stopping")]));
    vi.stubGlobal("fetch", fetchMock);
    const log = vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.scheduled(scheduledController("*/5 * * * *", "2026-07-27T07:05:00Z"), testEnv());

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls.some((call) => String(call[0]).endsWith("/stop"))).toBe(false);
    expect(log).toHaveBeenCalledWith(
      expect.stringContaining("raw_search_schedule_stop_deferred_indexer"),
    );
  });

  test("stops Raw after midnight when the indexer is stopped", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ state: "started" }))
      .mockResolvedValueOnce(Response.json([indexerMachine("stopped")]))
      .mockResolvedValueOnce(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.scheduled(scheduledController("*/5 * * * *", "2026-07-27T07:00:00Z"), testEnv());

    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(String(fetchMock.mock.calls[2]?.[0])).toContain(
      "/apps/ytsearch-raw-production/machines/8147deb94937e8/stop",
    );
  });

  test("restarts a stopped Raw machine after midnight in always-on mode", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json({ state: "stopped" }))
      .mockResolvedValueOnce(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.scheduled(
      scheduledController("*/5 * * * *", "2026-07-27T07:00:00Z"),
      testEnv({ SEARCH_SCHEDULE_MODE: "always_on" }),
    );

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain(
      "/apps/ytsearch-raw-production/machines/8147deb94937e8/start",
    );
  });

  test("accepts an indexer wake after midnight in always-on mode", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-27T07:00:00Z"));
    const ack = vi.fn();
    const retry = vi.fn();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json([indexerMachine("stopped")]))
      .mockResolvedValueOnce(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.queue(
      {
        messages: [{ body: { reason: "scheduled-repair" }, attempts: 1, ack, retry }],
      } as unknown as MessageBatch<WakeMessage>,
      testEnv({ SEARCH_SCHEDULE_MODE: "always_on" }),
    );

    expect(ack).toHaveBeenCalledOnce();
    expect(retry).not.toHaveBeenCalled();
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain(
      "/apps/ytsearch-indexer-raw-production/machines/current-indexer/start",
    );
  });

  test("acknowledges and defers an off-hours indexer wake", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T17:00:00Z"));
    const ack = vi.fn();
    const retry = vi.fn();
    const log = vi.spyOn(console, "log").mockImplementation(() => undefined);
    vi.stubGlobal("fetch", vi.fn());

    await worker.queue(
      {
        messages: [{ body: { reason: "index-request" }, attempts: 1, ack, retry }],
      } as unknown as MessageBatch<WakeMessage>,
      testEnv(),
    );

    expect(ack).toHaveBeenCalledOnce();
    expect(retry).not.toHaveBeenCalled();
    expect(fetch).not.toHaveBeenCalled();
    expect(log).toHaveBeenCalledWith(expect.stringContaining("indexer_wake_deferred"));
  });

  test("resolves and starts the current queue Machine during open hours", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T20:00:00Z"));
    const ack = vi.fn();
    const retry = vi.fn();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(Response.json([indexerMachine("stopped")]))
      .mockResolvedValueOnce(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.queue(
      {
        messages: [{ body: { reason: "index-request" }, attempts: 1, ack, retry }],
      } as unknown as MessageBatch<WakeMessage>,
      testEnv(),
    );

    expect(ack).toHaveBeenCalledOnce();
    expect(retry).not.toHaveBeenCalled();
    expect(String(fetchMock.mock.calls[0]?.[0])).toBe(
      "https://api.machines.dev/v1/apps/ytsearch-indexer-raw-production/machines",
    );
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain(
      "/apps/ytsearch-indexer-raw-production/machines/current-indexer/start",
    );
  });

  test("retries instead of choosing between duplicate queue Machines", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T20:00:00Z"));
    const ack = vi.fn();
    const retry = vi.fn();
    vi.stubGlobal(
      "fetch",
      vi
        .fn()
        .mockResolvedValue(
          Response.json([
            indexerMachine("stopped"),
            indexerMachine("stopped", "duplicate-indexer"),
          ]),
        ),
    );
    vi.spyOn(console, "error").mockImplementation(() => undefined);

    await worker.queue(
      {
        messages: [{ body: { reason: "index-request" }, attempts: 1, ack, retry }],
      } as unknown as MessageBatch<WakeMessage>,
      testEnv(),
    );

    expect(ack).not.toHaveBeenCalled();
    expect(retry).toHaveBeenCalledWith({ delaySeconds: 30 });
  });

  test("ignores a bulk repair Machine that targets Raw", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-26T20:00:00Z"));
    const ack = vi.fn();
    const retry = vi.fn();
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(
        Response.json([
          indexerMachine("stopped"),
          {
            id: "bulk-repair",
            state: "stopped",
            config: { env: { QUEUE_TARGET_BACKEND: "raw" } },
          },
        ]),
      )
      .mockResolvedValueOnce(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);
    vi.spyOn(console, "log").mockImplementation(() => undefined);

    await worker.queue(
      {
        messages: [{ body: { reason: "index-request" }, attempts: 1, ack, retry }],
      } as unknown as MessageBatch<WakeMessage>,
      testEnv(),
    );

    expect(ack).toHaveBeenCalledOnce();
    expect(retry).not.toHaveBeenCalled();
    expect(String(fetchMock.mock.calls[1]?.[0])).toContain(
      "/apps/ytsearch-indexer-raw-production/machines/current-indexer/start",
    );
  });
});

type WakeMessage = {
  reason: "index-request" | "scheduled-repair" | "availability-open";
};

function testEnv(overrides: Record<string, unknown> = {}): Env {
  return {
    INDEX_WAKE_QUEUE: { send: vi.fn().mockResolvedValue(undefined) },
    ASSETS: { fetch: vi.fn().mockResolvedValue(new Response("asset")) },
    PUBLIC_APP_ORIGIN: "https://search.pronsh.dev",
    API_BASE_URL: "https://ytsearch-api-raw-staging.fly.dev",
    INDEX_QUEUE_API_BASE_URL: "https://ytsearch-api-raw-staging.fly.dev",
    FLY_API_BASE_URL: "https://api.machines.dev",
    FLY_APP_NAME: "ytsearch-indexer-raw-production",
    FLY_RAW_SEARCH_APP_NAME: "ytsearch-raw-production",
    FLY_RAW_SEARCH_MACHINE_ID: "8147deb94937e8",
    RAW_SEARCH_HEALTH_URL: "https://ytsearch-raw-production.fly.dev/health",
    SEARCH_SCHEDULE_MODE: "enforce",
    FLYIO_TOKEN: "fly-token",
    ...overrides,
  } as unknown as Env;
}

function indexerMachine(state: string, id = "current-indexer") {
  return {
    id,
    state,
    config: {
      env: { YTSEARCH_MACHINE_ROLE: "queue-indexer", QUEUE_TARGET_BACKEND: "raw" },
    },
  };
}

function scheduledController(cron: string, instant: string): ScheduledController {
  return {
    cron,
    scheduledTime: new Date(instant).getTime(),
    noRetry: vi.fn(),
  };
}
