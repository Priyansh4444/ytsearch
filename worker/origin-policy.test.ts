import { describe, expect, it } from "vitest";

import { requestOriginAllowed } from "./origin-policy";

describe("request origin policy", () => {
  it("allows the configured production origin", () => {
    expect(requestOriginAllowed(new URL("https://search.pronsh.dev/api/search?q=test"))).toBe(true);
    expect(
      requestOriginAllowed(new URL("https://find.example.com/"), "https://find.example.com"),
    ).toBe(true);
  });

  it("allows localhost development on arbitrary ports", () => {
    expect(requestOriginAllowed(new URL("http://localhost:5173/anything"))).toBe(true);
    expect(requestOriginAllowed(new URL("http://127.0.0.1:8787/api/health"))).toBe(true);
    expect(requestOriginAllowed(new URL("http://[::1]:8787/"))).toBe(true);
  });

  it("rejects preview and unrelated domains", () => {
    expect(requestOriginAllowed(new URL("https://ytsearch-755.pages.dev/"))).toBe(false);
    expect(requestOriginAllowed(new URL("https://evil.example/api/search"))).toBe(false);
    expect(requestOriginAllowed(new URL("https://search.pronsh.dev.evil.example/"))).toBe(false);
  });

  it("fails closed for an invalid configured origin", () => {
    expect(requestOriginAllowed(new URL("https://search.pronsh.dev/"), "not a url")).toBe(false);
  });
});
