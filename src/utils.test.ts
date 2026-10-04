import { describe, expect, test } from "vitest";
import { highlightedParts } from "./utils";

describe("highlightedParts", () => {
  test.each([
    ["Wildfire Safety", "fire"],
    ["Preact signals", "react"],
  ])("does not invent substring highlights for %s", (text, query) => {
    expect(highlightedParts(text, undefined, query)).toEqual([{ text, highlighted: false }]);
  });

  test("preserves exactly the backend-provided tagged snippet", () => {
    expect(highlightedParts("Wildfire and fire", "Wildfire and <mark>fire</mark>", "fire")).toEqual(
      [
        { text: "Wildfire and ", highlighted: false },
        { text: "fire", highlighted: true },
      ],
    );
  });

  test("decodes the reader's escaped characters and nothing else", () => {
    const snippet = "it&#39;s &quot;R&amp;D&quot; <mark>&lt;b&gt;</mark> &amp;nbsp; plain";
    expect(highlightedParts("", snippet, "b")).toEqual([
      { text: 'it\'s "R&D" ', highlighted: false },
      { text: "<b>", highlighted: true },
      { text: " &nbsp; plain", highlighted: false },
    ]);
  });
});
