import { act, renderHook } from "@testing-library/react";

import { usePagination } from "./usePagination";

function items(n: number): number[] {
  return Array.from({ length: n }, (_, i) => i);
}

describe("usePagination", () => {
  beforeEach(() => window.localStorage.clear());

  it("slices to the default page size (10) and reports the range", () => {
    const { result } = renderHook(() => usePagination(items(25), "test.key"));
    expect(result.current.pageSize).toBe(10);
    expect(result.current.pageItems).toHaveLength(10);
    expect(result.current.pageItems[0]).toBe(0);
    expect(result.current.pageCount).toBe(3);
    expect(result.current.total).toBe(25);
    expect(result.current.rangeStart).toBe(1);
    expect(result.current.rangeEnd).toBe(10);
  });

  it("pages forward", () => {
    const { result } = renderHook(() => usePagination(items(25), "test.key"));
    act(() => result.current.setPage(1));
    expect(result.current.page).toBe(1);
    expect(result.current.pageItems[0]).toBe(10);
    expect(result.current.rangeStart).toBe(11);
    expect(result.current.rangeEnd).toBe(20);
  });

  it("changing the page size resets to page 1 and is sticky", () => {
    const { result } = renderHook(() => usePagination(items(25), "test.key"));
    act(() => result.current.setPage(2));
    act(() => result.current.setPageSize(25));
    expect(result.current.pageSize).toBe(25);
    expect(result.current.page).toBe(0);
    expect(result.current.pageItems).toHaveLength(25);
    expect(window.localStorage.getItem("test.key")).toBe("25");
    // A fresh mount reads the sticky size.
    const { result: reopened } = renderHook(() =>
      usePagination(items(25), "test.key"),
    );
    expect(reopened.current.pageSize).toBe(25);
  });

  it("clamps the page when the list shrinks", () => {
    const { result, rerender } = renderHook(
      ({ data }) => usePagination(data, "test.key"),
      { initialProps: { data: items(25) } },
    );
    act(() => result.current.setPage(2)); // last page of 3
    rerender({ data: items(5) }); // now a single page
    expect(result.current.page).toBe(0);
    expect(result.current.pageItems).toHaveLength(5);
  });
});
