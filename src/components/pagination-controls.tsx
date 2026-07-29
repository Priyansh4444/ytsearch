import { For } from "solid-js";
import type { Component, JSX } from "solid-js";
import type { SearchResponse } from "../types";

export const PaginationControls: Component<{
  response: SearchResponse;
  currentPage: number;
  onPageChange: (page: number) => void;
}> = (props) => {
  const hasPrevious = () => props.currentPage > 1;
  const hasNext = () => props.response.has_next_page;
  const pages = () => pageNumbers(props.currentPage, hasNext());

  return (
    <nav
      aria-label="Search result pages"
      class="flex flex-wrap items-center justify-center gap-2 py-3 text-sm"
    >
      <PageButton
        disabled={!hasPrevious()}
        onClick={() => props.onPageChange(props.currentPage - 1)}
      >
        Previous
      </PageButton>

      <For each={pages()}>
        {(pageNumber) => (
          <PageButton
            active={pageNumber === props.currentPage}
            onClick={() => props.onPageChange(pageNumber)}
          >
            {pageNumber}
          </PageButton>
        )}
      </For>

      <PageButton disabled={!hasNext()} onClick={() => props.onPageChange(props.currentPage + 1)}>
        Next
      </PageButton>
    </nav>
  );
};

const PageButton: Component<{
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: JSX.Element;
}> = (props) => (
  <button
    type="button"
    aria-current={props.active ? "page" : undefined}
    disabled={props.disabled}
    onClick={props.onClick}
    class={`pressable min-h-10 border px-3 py-1.5 ${
      props.active
        ? "border-vesper-yellow bg-vesper-yellow text-vesper-bg"
        : "border-vesper-control bg-vesper-bg text-vesper-muted hover:border-vesper-yellow hover:text-vesper-yellow"
    } disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-vesper-control disabled:hover:text-vesper-muted`}
  >
    {props.children}
  </button>
);

function pageNumbers(currentPage: number, hasNext: boolean): number[] {
  const start = Math.max(1, currentPage - 3);
  const end = hasNext ? currentPage + 1 : currentPage;
  return Array.from({ length: end - start + 1 }, (_, index) => start + index);
}
