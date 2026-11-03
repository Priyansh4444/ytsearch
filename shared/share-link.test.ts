import { describe, expect, test } from "vitest";
import {
  MAX_SHARED_WINDOW_SECONDS,
  formatClockTime,
  movedWindow,
  parseSharedMoment,
  sharedMomentPath,
  xPostUrl,
  youtubeEmbedUrl,
  youtubeMomentUrl,
  type SharedMoment,
} from "./share-link";

const parse = (path: string) => {
  const url = new URL(path, "https://example.test");
  return parseSharedMoment(url.pathname, url.searchParams);
};

describe("shared moment links", () => {
  test.each<SharedMoment>([
    { videoId: "dQw4w9WgXcQ", startSeconds: 0, endSeconds: 30, query: "never gonna", hybrid: true },
    {
      videoId: "a_b-c1",
      startSeconds: 754,
      endSeconds: undefined,
      query: undefined,
      hybrid: false,
    },
    {
      videoId: "dQw4w9WgXcQ",
      startSeconds: 12,
      endSeconds: 612,
      query: "c++ & rust = ?",
      hybrid: false,
    },
  ])("round-trips $videoId at $startSeconds", (moment) => {
    expect(parse(sharedMomentPath(moment))).toEqual(moment);
  });

  test.each([
    "/s/",
    "/s/short",
    "/s/has/slash",
    "/s/<script>alert",
    "/s/dQw4w9WgXcQ?t=-1",
    "/s/dQw4w9WgXcQ?t=1.5",
    "/s/dQw4w9WgXcQ?t=soon",
    "/other/dQw4w9WgXcQ",
  ])("rejects %s", (path) => {
    expect(parse(path)).toBeUndefined();
  });

  test("drops an end that is not a usable window instead of the whole link", () => {
    const end = (query: string) => parse(`/s/dQw4w9WgXcQ?t=100${query}`)?.endSeconds;
    expect(end("&end=100")).toBeUndefined();
    expect(end("&end=99")).toBeUndefined();
    expect(end(`&end=${101 + MAX_SHARED_WINDOW_SECONDS}`)).toBeUndefined();
    expect(end(`&end=${100 + MAX_SHARED_WINDOW_SECONDS}`)).toBe(700);
  });

  test("bounds the query it carries", () => {
    expect(parse(`/s/dQw4w9WgXcQ?q=${"x".repeat(500)}`)?.query).toHaveLength(200);
    expect(parse("/s/dQw4w9WgXcQ?q=%20%20")?.query).toBeUndefined();
  });

  test("moving a bound keeps the window ordered and within the limit", () => {
    const window = { start: 100, end: 130 };
    expect(movedWindow(window, "start", -5, 5)).toEqual({ start: 95, end: 130 });
    expect(movedWindow(window, "end", 5, 5)).toEqual({ start: 100, end: 135 });
    expect(movedWindow(window, "start", 100, 5)).toEqual({ start: 125, end: 130 });
    expect(movedWindow(window, "end", -100, 5)).toEqual({ start: 100, end: 105 });
    expect(movedWindow({ start: 3, end: 30 }, "start", -5, 5)).toEqual({ start: 0, end: 30 });
    expect(movedWindow({ start: 0, end: 600 }, "end", 5, 5)).toEqual({ start: 0, end: 600 });
    expect(movedWindow({ start: 700, end: 1300 }, "start", -5, 5)).toEqual({
      start: 700,
      end: 1300,
    });
  });

  test("links to YouTube at the window, and its player stops at the end", () => {
    const moment = {
      videoId: "dQw4w9WgXcQ",
      startSeconds: 86,
      endSeconds: 116,
      query: "x",
      hybrid: false,
    };
    expect(youtubeMomentUrl(moment)).toBe("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=86s");
    expect(youtubeEmbedUrl(moment, "www.youtube.com")).toBe(
      "https://www.youtube.com/embed/dQw4w9WgXcQ?start=86&rel=0&end=116",
    );
    expect(youtubeEmbedUrl({ ...moment, endSeconds: undefined })).toBe(
      "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?start=86&rel=0",
    );
  });

  test("an X post quotes the moment and ends on the YouTube link so the video plays", () => {
    const moment = {
      videoId: "dQw4w9WgXcQ",
      startSeconds: 86,
      endSeconds: 116,
      query: "x",
      hybrid: false,
    };
    const url = new URL(xPostUrl(moment, "  it  uses ray\nshooting ", "https://site.test/s/abc"));
    expect(url.origin + url.pathname).toBe("https://x.com/intent/post");
    expect(url.searchParams.get("text")).toBe("“it uses ray shooting”\n\nhttps://site.test/s/abc");
    expect(url.searchParams.get("url")).toBe("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=86s");

    const long = new URL(xPostUrl(moment, "word ".repeat(100), "https://site.test/s/abc"));
    const text = long.searchParams.get("text")!;
    expect(text.split("\n\n")[0]!.length).toBeLessThan(190);
    expect(text).toContain("…”");
  });

  test("formats clock times the way YouTube shows them", () => {
    expect(formatClockTime(0)).toBe("0:00");
    expect(formatClockTime(65)).toBe("1:05");
    expect(formatClockTime(3_725)).toBe("1:02:05");
  });
});
