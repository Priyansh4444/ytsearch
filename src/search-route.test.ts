import { afterEach, describe, expect, test, vi } from "vitest";
import {
  resultAnchorFromHash,
  resultAnchorId,
  scrollToSearchResult,
  searchResultPermalink,
  searchStateFromLocation,
  writeSearchUrl,
} from "./search-route";

describe("search result permalinks", () => {
  afterEach(() => {
    window.history.replaceState({}, "", "/");
    document.body.replaceChildren();
  });

  test("encodes a stable video anchor and ignores unknown hashes", () => {
    expect(resultAnchorId("dQw4w9WgXcQ")).toBe("video-dQw4w9WgXcQ");
    expect(resultAnchorFromHash("#video-dQw4w9WgXcQ")).toBe("video-dQw4w9WgXcQ");
    expect(resultAnchorFromHash("video-abc")).toBe("video-abc");
    expect(resultAnchorFromHash("#other")).toBeUndefined();
    expect(resultAnchorFromHash("#video-")).toBeUndefined();
    expect(resultAnchorFromHash("#video-%E0%A4%A")).toBeUndefined();
  });

  test("builds a shareable result URL from the current search", () => {
    window.history.replaceState({}, "", "/?q=rust+testing&page=2");
    expect(searchResultPermalink("video-1")).toBe(
      `${window.location.origin}/?q=rust+testing&page=2#video-video-1`,
    );
  });

  test("scrolls the linked result into view and focuses it", () => {
    const article = document.createElement("article");
    article.id = "video-video-1";
    article.tabIndex = -1;
    const scrollIntoView = vi.fn();
    const focus = vi.fn();
    article.scrollIntoView = scrollIntoView;
    article.focus = focus;
    document.body.append(article);
    window.history.replaceState({}, "", "/?q=rust#video-video-1");

    expect(scrollToSearchResult()).toBe(true);
    expect(scrollIntoView).toHaveBeenCalledWith({ behavior: "smooth", block: "start" });
    expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  });

  test("uses instant scrolling when the user prefers reduced motion", () => {
    const article = document.createElement("article");
    article.id = "video-video-1";
    const scrollIntoView = vi.fn();
    article.scrollIntoView = scrollIntoView;
    article.focus = vi.fn();
    document.body.append(article);
    window.history.replaceState({}, "", "/?q=rust#video-video-1");
    vi.spyOn(window, "matchMedia").mockReturnValue({
      matches: true,
      media: "(prefers-reduced-motion: reduce)",
      onchange: null,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      addListener: () => undefined,
      removeListener: () => undefined,
      dispatchEvent: () => true,
    });

    expect(scrollToSearchResult()).toBe(true);
    expect(scrollIntoView).toHaveBeenCalledWith({ behavior: "auto", block: "start" });
  });

  test("does not scroll when the hash is missing or the card is not rendered", () => {
    expect(scrollToSearchResult({ hash: "" })).toBe(false);
    window.history.replaceState({}, "", "/?q=rust#video-missing");
    expect(scrollToSearchResult()).toBe(false);
  });

  test("keeps search state in the query string and drops result hashes on a new search", () => {
    window.history.replaceState({}, "", "/?q=old#video-video-1");
    writeSearchUrl({ query: "rust", youtuber: "@example", page: 1, sort: "relevance" });
    expect(searchStateFromLocation()).toEqual({
      query: "rust",
      youtuber: "@example",
      page: 1,
      sort: "relevance",
    });
    expect(window.location.hash).toBe("");
  });
});
