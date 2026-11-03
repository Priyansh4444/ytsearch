import { describe, expect, test } from "vitest";
import { captionLines, markableTerms } from "./captions";

const WINDOW = { startMs: 10_000, endMs: 30_000 };
const hit = (t_start_ms: number, t_end_ms: number, text: string, extra = {}) => ({
  match_kind: "transcript",
  t_start_ms,
  t_end_ms,
  text,
  ...extra,
});

describe("captionLines", () => {
  test("keeps lines overlapping the window, in order, with each chunk's real times", () => {
    const lines = captionLines(
      [
        hit(20_000, 28_000, "later words"),
        hit(10_000, 18_000, "earlier words"),
        hit(40_000, 45_000, "outside"),
      ],
      WINDOW,
      new Set(),
    );
    expect(lines).toEqual([
      { startMs: 10_000, endMs: 18_000, text: "earlier words", matched: false },
      { startMs: 20_000, endMs: 28_000, text: "later words", matched: false },
    ]);
  });

  test("fills the gaps from a hit's neighbours, timed around it", () => {
    const lines = captionLines(
      [hit(14_000, 18_000, "abcd", { prev_text: "before", next_text: "after" })],
      { startMs: 0, endMs: 60_000 },
      new Set(),
    );
    expect(lines.map((line) => line.text)).toEqual(["before", "abcd", "after"]);
    expect(lines[0]).toMatchObject({ startMs: 8_000, endMs: 14_000 });
    expect(lines[2]).toMatchObject({ startMs: 18_000, endMs: 23_000 });
  });

  test("a chunk's exact times beat another hit's estimate of it, in either order", () => {
    const exact = hit(20_000, 24_000, "shared words");
    const neighbour = hit(10_000, 14_000, "first", { next_text: "shared words" });
    for (const hits of [
      [exact, neighbour],
      [neighbour, exact],
    ]) {
      const line = captionLines(hits, { startMs: 0, endMs: 60_000 }, new Set()).find(
        (candidate) => candidate.text === "shared words",
      );
      expect(line).toMatchObject({ startMs: 20_000, endMs: 24_000 });
    }
  });

  test("splits a long chunk into evenly timed lines without losing a word", () => {
    const text = Array.from({ length: 20 }, (_, index) => `w${index}`).join(" ");
    const lines = captionLines([hit(10_000, 30_000, text)], WINDOW, new Set());
    expect(lines.map((line) => line.text).join(" ")).toBe(text);
    expect(lines.length).toBe(3);
    expect(lines[0]).toMatchObject({ startMs: 10_000, endMs: 17_000 });
    expect(lines[2]?.endMs).toBe(30_000);
    expect(lines.every((line) => line.text.split(" ").length <= 8)).toBe(true);
  });

  test("marks only lines holding a searched word, and only whole words", () => {
    const lines = captionLines(
      [
        hit(10_000, 12_000, "ray shooting here"),
        hit(13_000, 15_000, "sharpshootingly no"),
        hit(16_000, 18_000, "other"),
      ],
      WINDOW,
      markableTerms("the shooting"),
    );
    expect(lines.map((line) => line.matched)).toEqual([true, false, false]);
  });

  test("ignores title hits and malformed ones", () => {
    const hits = [
      { match_kind: "title", t_start_ms: 10_000, text: "title" },
      { match_kind: "transcript", text: "no time" },
      { match_kind: "transcript", t_start_ms: 10_000, text: "   " },
    ];
    expect(captionLines(hits, WINDOW, new Set())).toEqual([]);
  });

  test("a hit with no end still gets a usable range", () => {
    const [line] = captionLines(
      [{ match_kind: "transcript", t_start_ms: 10_000, text: "words here" }],
      WINDOW,
      new Set(),
    );
    expect(line.endMs).toBeGreaterThan(10_000);
  });
});

describe("markableTerms", () => {
  test("drops common and short words, whatever the case", () => {
    expect([...markableTerms("Why did Theo drop a Prisma")]).toEqual(["theo", "drop", "prisma"]);
    expect([...markableTerms(undefined)]).toEqual([]);
  });
});
