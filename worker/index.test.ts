import { describe, expect, test, vi } from "vitest";
import worker from "./index";
import { shouldWakeIndexer } from "./wake-policy";

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
      { INDEX_WAKE_QUEUE: { send } } as unknown as Env,
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
