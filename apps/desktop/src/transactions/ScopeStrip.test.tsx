import { fireEvent, render, screen } from "@testing-library/react";

import type { AccountViewDto, TagViewDto } from "@/bindings";

import { EMPTY_FILTERS, type TransactionFilters } from "./filters";
import { ScopeStrip } from "./ScopeStrip";
import { scopeChips } from "./scopeChips";

const ctx = {
  accounts: [
    { id: "a1", name: "Checking" } as AccountViewDto,
    { id: "a2", name: "Savings" } as AccountViewDto,
  ],
  tags: [{ id: "t1", name: "Reimbursable" } as TagViewDto],
  categoryLabel: (id: string) => (id === "c1" ? "Food & Dining / Groceries" : id),
  drilled: false,
};

const chipsFor = (over: Partial<TransactionFilters>, drilled = false) =>
  scopeChips({ ...EMPTY_FILTERS, ...over }, vi.fn(), { ...ctx, drilled });

describe("scopeChips (personal-cfo-3tbn)", () => {
  it("says nothing when nothing narrows the list", () => {
    expect(chipsFor({})).toEqual([]);
  });

  it("describes each facet, and the DRILL as one of them", () => {
    // The chart's drill and the bar's category facet are the same filters.categoryId.
    // Showing them in two places asks the reader to reconcile one piece of state with
    // itself — which is the disagreement ADR 0052 §2 exists to prevent.
    const chips = chipsFor({ categoryId: "c1" }, true);
    expect(chips).toHaveLength(1);
    expect(chips[0]?.label).toBe("Food & Dining / Groceries");
    expect(chips[0]?.fromChart).toBe(true);
  });

  it("marks a category picked from the FACET as not from the chart", () => {
    expect(chipsFor({ categoryId: "c1" }, false)[0]?.fromChart).toBe(false);
  });

  it("names one account but counts several", () => {
    expect(chipsFor({ accountIds: ["a1"] })[0]?.label).toBe("Checking");
    expect(chipsFor({ accountIds: ["a1", "a2"] })[0]?.label).toBe("2 accounts");
  });

  it("uses the full category path, so two leaves sharing a name are distinguishable", () => {
    // The strip has to say WHICH "Maintenance" is filtering the list.
    expect(chipsFor({ categoryId: "c1" })[0]?.label).toContain("Food & Dining");
  });

  it("clearing a chip clears only its own facet", () => {
    const set = vi.fn();
    const chips = scopeChips(
      { ...EMPTY_FILTERS, query: "coffee", tagId: "t1" },
      set,
      ctx,
    );
    chips.find((c) => c.key === "query")?.clear();
    expect(set).toHaveBeenCalledWith(
      expect.objectContaining({ query: "", tagId: "t1" }),
    );
  });

  it("describes a one-sided date range without inventing the other end", () => {
    expect(chipsFor({ from: "2026-01-01" })[0]?.label).toMatch(/– now$/);
    expect(chipsFor({ to: "2026-06-30" })[0]?.label).toMatch(/^any –/);
  });
});

describe("ScopeStrip", () => {
  const render1 = (over = {}) =>
    render(
      <ScopeStrip
        chips={[]}
        shown={12}
        total={12}
        outflowMinor={-40_000}
        inflowMinor={150_000}
        currency="USD"
        onClearAll={vi.fn()}
        {...over}
      />,
    );

  it("states the totals for the rows on screen", () => {
    render1();
    expect(screen.getByText("12 transactions")).toBeInTheDocument();
    expect(screen.getByText("$400.00 out")).toBeInTheDocument();
    expect(screen.getByText("$1,500.00 in")).toBeInTheDocument();
  });

  it("says so plainly when nothing narrows the list", () => {
    render1();
    expect(screen.getByText("Showing everything, newest first.")).toBeInTheDocument();
  });

  it("distinguishes shown from total when filtered", () => {
    render1({ shown: 3, total: 12 });
    expect(screen.getByText("3 of 12 transactions")).toBeInTheDocument();
  });

  it("clears a chip through its own handler", () => {
    const clear = vi.fn();
    render1({ chips: [{ key: "q", label: "“coffee”", clear }] });
    fireEvent.click(screen.getByRole("button", { name: /Remove “coffee”/ }));
    expect(clear).toHaveBeenCalled();
  });

  it("offers clear-all only when more than one thing narrows the list", () => {
    render1({ chips: [{ key: "q", label: "a", clear: vi.fn() }] });
    expect(screen.queryByText("Clear all")).not.toBeInTheDocument();
  });
});
