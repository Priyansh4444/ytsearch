import { fireEvent, render, screen } from "@solidjs/testing-library";
import { describe, expect, test, vi } from "vitest";
import type { SearchResponse } from "../types";
import { PaginationControls } from "./pagination-controls";

const response = (hasNextPage: boolean): SearchResponse => ({
  query: "solid",
  total_moments: 1,
  total_videos: 1,
  totals_approximate: false,
  total_found: 1,
  out_of: 1,
  time_to_first_result_ms: 1,
  page: 1,
  per_page: 12,
  sort: "relevance",
  mode: "lexical",
  has_next_page: hasNextPage,
  videos: [],
});

describe("PaginationControls", () => {
  test("keeps previous and next visible on a single result page", () => {
    render(() => (
      <PaginationControls
        response={response(false)}
        currentPage={1}
        onPageChange={() => undefined}
      />
    ));

    expect(screen.getByRole("button", { name: "Previous" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Next" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "1" })).toHaveAttribute("aria-current", "page");
  });

  test("moves to the next result page", async () => {
    const onPageChange = vi.fn();
    render(() => (
      <PaginationControls response={response(true)} currentPage={1} onPageChange={onPageChange} />
    ));

    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(onPageChange).toHaveBeenCalledWith(2);
  });
});
