import * as Effect from "effect/Effect";
import { afterEach, describe, expect, it, vi } from "vitest";
import { flyMachineStateEffect, flyMachinesEffect, startFlyMachineEffect } from "./fly-machine";

const env = {
  FLY_API_BASE_URL: "https://api.machines.dev",
  FLYIO_TOKEN: "secret-token",
};

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Effect Fly API client", () => {
  it("decodes machine state through the Effect boundary", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json({ state: "started" })));

    await expect(
      Effect.runPromise(flyMachineStateEffect(env, "search", "machine-1")),
    ).resolves.toBe("started");
  });

  it("rejects malformed machine lists before lifecycle code can use them", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(Response.json([{ state: "started" }])));

    await expect(Effect.runPromise(flyMachinesEffect(env, "indexer"))).rejects.toThrow(
      "Fly machine list response validation failed",
    );
  });

  it("passes Effect interruption to Fly fetches", async () => {
    const fetchMock = vi.fn().mockResolvedValue(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);

    await Effect.runPromise(startFlyMachineEffect(env, "search", "machine-1"));

    expect(fetchMock.mock.calls[0]?.[1]).toEqual({
      method: "POST",
      headers: { Authorization: "Bearer secret-token" },
      signal: expect.any(AbortSignal),
    });
  });
});
