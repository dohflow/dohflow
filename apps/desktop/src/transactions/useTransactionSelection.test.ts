import { act, renderHook } from "@testing-library/react";

import type { TransactionRowDto } from "@/bindings";
import { useTransactionSelection } from "./useTransactionSelection";

function row(id: string, over: Partial<TransactionRowDto> = {}): TransactionRowDto {
  return {
    transaction_id: id,
    account_id: "acct-1",
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-22T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -3200, currency: "USD" },
    memo: null,
    counterparty: null,
    category_id: null,
    reviewed: false,
    note: null,
    tag_ids: [],
    split_count: 0,
    category_source: null,
    category_confidence_bps: null,
    ...over,
  };
}

describe("useTransactionSelection", () => {
  it("dedupes a transaction present in both lists into one entry", () => {
    const { result } = renderHook(() => useTransactionSelection());
    // The SAME id is visible in both lists…
    act(() => result.current.setVisibleRows("inbox", [row("a"), row("b")]));
    act(() => result.current.setVisibleRows("activity", [row("a"), row("c")]));
    // …and selected once.
    act(() => result.current.toggle(row("a")));
    expect(result.current.selectedCount).toBe(1);
    // Select-all unions both lists deduped: a, b, c = 3.
    act(() => result.current.selectAllVisible());
    expect(result.current.selectedCount).toBe(3);
    expect(result.current.visibleCount).toBe(3);
  });

  it("toggles off and clears via setSelectionIds([])", () => {
    const { result } = renderHook(() => useTransactionSelection());
    act(() => result.current.toggle(row("a")));
    act(() => result.current.toggle(row("a")));
    expect(result.current.selectedCount).toBe(0);
    act(() => result.current.toggle(row("a")));
    act(() => result.current.setSelectionIds([]));
    expect(result.current.selectedCount).toBe(0);
  });

  it("refreshes a selected row's DTO when it re-registers, but never prunes it", () => {
    const { result } = renderHook(() => useTransactionSelection());
    act(() => result.current.setVisibleRows("activity", [row("a", { tag_ids: ["old"] })]));
    act(() => result.current.toggle(row("a", { tag_ids: ["old"] })));
    // A refetch changes a's tags (same id) — the selected DTO refreshes.
    act(() => result.current.setVisibleRows("activity", [row("a", { tag_ids: ["new"] })]));
    expect(result.current.selectedRows[0]?.tag_ids).toEqual(["new"]);
    // The row scrolls off the page (no longer visible) — selection persists by id (AC-6).
    act(() => result.current.setVisibleRows("activity", [row("b")]));
    expect(result.current.selectedRows.map((r) => r.transaction_id)).toEqual(["a"]);
    expect(result.current.selectedRows[0]?.tag_ids).toEqual(["new"]);
  });
});

it("selectRows adds explicit rows never registered as visible (4d8.25.16)", () => {
  const { result } = renderHook(() => useTransactionSelection());
  act(() => {
    result.current.selectRows([row("beyond-1"), row("beyond-2")]);
  });
  expect(result.current.selectedCount).toBe(2);
  expect(result.current.isSelected("beyond-1")).toBe(true);
  // Idempotent: re-adding the same row does not duplicate.
  act(() => {
    result.current.selectRows([row("beyond-1")]);
  });
  expect(result.current.selectedCount).toBe(2);
});
