import { describe, expect, it } from "vitest";
import { redirectToProduction } from "./experimental-redirect";

describe("experimental site redirect", () => {
  it.each([
    ["/", "https://search.pronsh.dev/"],
    ["/s/dQw4w9WgXcQ?t=42&e=60", "https://search.pronsh.dev/s/dQw4w9WgXcQ?t=42&e=60"],
    ["/embed/dQw4w9WgXcQ?t=1", "https://search.pronsh.dev/embed/dQw4w9WgXcQ?t=1"],
    [
      "/api/search?q=rust%20ownership&mode=hybrid",
      "https://search.pronsh.dev/api/search?q=rust%20ownership&mode=hybrid",
    ],
  ])("sends %s to the same place on production", (path, expected) => {
    const response = redirectToProduction(
      new Request(`https://search.experimental.pronsh.dev${path}`, { method: "GET" }),
    );
    expect(response.status).toBe(301);
    expect(response.headers.get("location")).toBe(expected);
  });

  it("never redirects to a host taken from the request", () => {
    const response = redirectToProduction(
      new Request("https://search.experimental.pronsh.dev//evil.example/x", {
        headers: { host: "evil.example" },
      }),
    );
    expect(new URL(response.headers.get("location") ?? "").origin).toBe(
      "https://search.pronsh.dev",
    );
  });
});
