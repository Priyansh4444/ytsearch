import { describe, expect, test } from "vitest";
import { presentSearchError } from "./search-error";

describe("presentSearchError", () => {
  test.each([
    ["ApiTransportError", "Connection problem", "Check your connection"],
    ["ApiDecodeError", "Search response error", "unexpected response"],
    ["ApiResponseError", "Search unavailable", "Try again in a moment"],
  ])("presents %s without exposing its technical detail", (_tag, title, message) => {
    const presentation = presentSearchError({
      _tag,
      message: "private backend detail",
      cause: new Error("secret upstream URL"),
    });

    expect(presentation.title).toBe(title);
    expect(presentation.message).toContain(message);
    expect(JSON.stringify(presentation)).not.toContain("private backend detail");
    expect(JSON.stringify(presentation)).not.toContain("secret upstream URL");
  });

  test("uses a safe fallback for unknown failures", () => {
    expect(presentSearchError(new Error("private backend detail"))).toEqual({
      title: "Search failed",
      message: "Unable to search. Try again.",
    });
  });
});
