import { render, screen } from "@testing-library/react";

import type { Pagination } from "@/lib/usePagination";
import { PaginationControls } from "./pagination";

const pagination = {
  page: 0,
  pageCount: 1,
  pageSize: 10,
  setPage: () => undefined,
  setPageSize: () => undefined,
  total: 3,
  rangeStart: 1,
  rangeEnd: 3,
} as unknown as Pagination<unknown>;

// personal-cfo-i93n3.28 slice 8: the page-size and page controls wrap at the largest text size on a phone (compact-only; desktop unchanged).
describe("PaginationControls on a phone", () => {
  it("lets the page-size row and the page buttons wrap, and nothing wraps unconditionally", () => {
    render(<PaginationControls pagination={pagination} noun="transactions" />);
    const pageLabel = screen.getByText(/Page 1 of 1/);
    const pageButtons = pageLabel.parentElement!;
    expect(pageButtons).toHaveClass("compact:flex-wrap");
    expect(pageButtons.parentElement).toHaveClass("compact:flex-wrap");
    expect(pageButtons).not.toHaveClass("flex-wrap");
    expect(pageButtons.parentElement).not.toHaveClass("flex-wrap");
    // The outer row already wrapped at every size.
    expect(pageButtons.parentElement!.parentElement).toHaveClass("flex-wrap");
  });
});
