import { afterEach, describe, expect, test, vi } from "vitest";
import {
  hybridToggleEnabled,
  loadSearchModePreference,
  resolveSearchMode,
  saveSearchModePreference,
} from "./search-mode";
import { searchStateFromLocation, writeSearchUrl } from "./search-route";

const offer = () => vi.stubEnv("VITE_HYBRID_TOGGLE", "true");

afterEach(() => {
  vi.unstubAllEnvs();
  window.localStorage.clear();
  window.history.replaceState({}, "", "/");
});

describe("search mode", () => {
  test("a build without the toggle never leaves lexical, even for a hybrid link", () => {
    expect(hybridToggleEnabled()).toBe(false);
    expect(resolveSearchMode("hybrid")).toBeUndefined();
    saveSearchModePreference("hybrid");
    expect(loadSearchModePreference()).toBe("lexical");
    window.history.replaceState({}, "", "/?q=rust&mode=hybrid");
    expect(searchStateFromLocation()?.mode).toBe("lexical");
  });

  test("a build with the toggle honours the URL over the saved preference", () => {
    offer();
    saveSearchModePreference("hybrid");
    window.history.replaceState({}, "", "/?q=rust&mode=lexical");
    expect(searchStateFromLocation()?.mode).toBe("lexical");
    window.history.replaceState({}, "", "/?q=rust");
    expect(searchStateFromLocation()?.mode).toBe("hybrid");
  });

  test("an unknown mode falls back to the preference", () => {
    offer();
    expect(resolveSearchMode("fused")).toBeUndefined();
    expect(resolveSearchMode(null)).toBeUndefined();
    window.history.replaceState({}, "", "/?q=rust&mode=fused");
    expect(searchStateFromLocation()?.mode).toBe("lexical");
  });

  test("the mode survives a round trip through the URL, and lexical adds nothing", () => {
    offer();
    writeSearchUrl({ query: "rust", youtuber: "", page: 1, sort: "relevance", mode: "hybrid" });
    expect(window.location.search).toBe("?q=rust&mode=hybrid");
    expect(searchStateFromLocation()?.mode).toBe("hybrid");
    writeSearchUrl({ query: "rust", youtuber: "", page: 1, sort: "relevance", mode: "lexical" });
    expect(window.location.search).toBe("?q=rust");
  });

  test("storage failures leave search working", () => {
    const broken = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    } as unknown as Storage;
    offer();
    expect(loadSearchModePreference(broken)).toBe("lexical");
    expect(() => saveSearchModePreference("hybrid", broken)).not.toThrow();
  });
});
