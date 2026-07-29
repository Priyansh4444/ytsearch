import { describe, expect, test } from "vitest";
import { evaluateSearchSchedule, nextPublicOpening } from "./service-schedule";

describe("Pacific search schedule", () => {
  test.each([
    ["2026-07-26T18:44:00Z", "closed", "2026-07-26T18:45:00.000Z"],
    ["2026-07-26T18:45:00Z", "prewarm", "2026-07-26T19:00:00.000Z"],
    ["2026-07-26T18:59:00Z", "prewarm", "2026-07-26T19:00:00.000Z"],
    ["2026-07-26T19:00:00Z", "open", "2026-07-27T07:00:00.000Z"],
    ["2026-07-27T06:59:00Z", "open", "2026-07-27T07:00:00.000Z"],
    ["2026-07-27T07:00:00Z", "closed", "2026-07-27T18:45:00.000Z"],
  ])("%s evaluates as %s", (instant, phase, nextTransitionAt) => {
    expect(evaluateSearchSchedule(new Date(instant))).toMatchObject({
      phase,
      nextTransitionAt,
      timeZone: "America/Los_Angeles",
    });
  });

  test("uses PST in winter and PDT in summer", () => {
    expect(evaluateSearchSchedule(new Date("2026-01-15T20:00:00Z")).phase).toBe("open");
    expect(evaluateSearchSchedule(new Date("2026-07-15T19:00:00Z")).phase).toBe("open");
    expect(nextPublicOpening(new Date("2026-01-15T19:00:00Z"))).toBe("2026-01-15T20:00:00.000Z");
    expect(nextPublicOpening(new Date("2026-07-15T18:00:00Z"))).toBe("2026-07-15T19:00:00.000Z");
  });

  test("keeps noon local across spring-forward and fall-back changes", () => {
    expect(nextPublicOpening(new Date("2026-03-08T08:30:00Z"))).toBe("2026-03-08T19:00:00.000Z");
    expect(nextPublicOpening(new Date("2026-11-01T08:30:00Z"))).toBe("2026-11-01T20:00:00.000Z");
  });
});
