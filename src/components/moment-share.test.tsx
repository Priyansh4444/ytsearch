import { fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { parseSharedMoment } from "../../shared/share-link";
import type { SearchHit } from "../types";
import { MomentSharePanel } from "./moment-share";

const { capture, copyToClipboard } = vi.hoisted(() => ({
  capture: vi.fn(),
  copyToClipboard: vi.fn(),
}));
vi.mock("../analytics", () => ({ capture }));
vi.mock("../utils", () => ({ copyToClipboard }));

const hit = { t_start_ms: 754_900, text: "the moment" } as SearchHit;

const open = () =>
  render(() => (
    <MomentSharePanel
      videoId="dQw4w9WgXcQ"
      hit={hit}
      query="server actions"
      hybrid={false}
      analytics={{ rank: 1 }}
    />
  ));

const sharedMoment = () => {
  const href = screen.getByRole("link", { name: "Preview" }).getAttribute("href")!;
  const url = new URL(href);
  return parseSharedMoment(url.pathname, url.searchParams);
};

describe("MomentSharePanel", () => {
  beforeEach(() => copyToClipboard.mockReset().mockResolvedValue(undefined));

  test("starts just before the moment with a clip the Worker can parse", () => {
    open();
    expect(sharedMoment()).toEqual({
      videoId: "dQw4w9WgXcQ",
      startSeconds: 752,
      endSeconds: 782,
      query: "server actions",
      hybrid: false,
    });
  });

  test("the window controls move the link's bounds", async () => {
    open();
    fireEvent.click(screen.getByRole("button", { name: "Move start 5 seconds earlier" }));
    fireEvent.click(screen.getByRole("button", { name: "Move end 5 seconds later" }));
    await waitFor(() =>
      expect(sharedMoment()).toMatchObject({ startSeconds: 747, endSeconds: 787 }),
    );
    expect(screen.getByText("40 seconds")).toBeInTheDocument();
  });

  test("copies the current link and reports it without the search words", async () => {
    open();
    fireEvent.click(screen.getByRole("button", { name: "Copy clip link" }));
    expect(await screen.findByRole("button", { name: "Clip link copied" })).toBeInTheDocument();
    expect(copyToClipboard.mock.calls[0]?.[0]).toContain("/s/dQw4w9WgXcQ?t=752&end=782");
    expect(capture).toHaveBeenCalledWith("moment_share_copied", {
      rank: 1,
      window_seconds: 30,
    });
  });

  test("composes an X post that quotes the moment and links the clip", () => {
    open();
    const post = new URL(screen.getByRole("link", { name: "Post on X" }).getAttribute("href")!);
    expect(post.searchParams.get("url")).toContain("/s/dQw4w9WgXcQ?t=752");
    expect(post.searchParams.get("text")).toBe("“the moment”");
  });

  test("shows the link to copy by hand when the clipboard is denied", async () => {
    copyToClipboard.mockRejectedValueOnce(new Error("denied"));
    open();
    fireEvent.click(screen.getByRole("button", { name: "Copy clip link" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("/s/dQw4w9WgXcQ"));
  });
});
