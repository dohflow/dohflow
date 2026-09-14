import type { TransactionRowDto } from "@/bindings";
import {
  activeFilterCount,
  EMPTY_FILTERS,
  filterTransactions,
  sortTransactions,
  toTransactionPageInput,
} from "./filters";

function row(overrides: Partial<TransactionRowDto>): TransactionRowDto {
  return {
    transaction_id: Math.random().toString(36).slice(2),
    account_id: "acct-1",
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-07-01T12:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -1000, currency: "USD" },
    memo: null,
    counterparty: null,
    category_id: null,
    reviewed: true,
    note: null,
    tag_ids: [],
    split_count: 0,
    category_source: null,
    category_confidence_bps: null,
    ...overrides,
  };
}

describe("filterTransactions", () => {
  const rows = [
    row({ memo: "Trader Joe's", note: "weekly groceries" }),
    row({ counterparty: "Shell Oil" }),
    row({ note: "piano lessons", account_name: "Savings", account_id: "acct-2" }),
  ];

  it("search spans memo, counterparty, note, and account name", () => {
    expect(
      filterTransactions(rows, { ...EMPTY_FILTERS, query: "trader" }),
    ).toHaveLength(1);
    expect(
      filterTransactions(rows, { ...EMPTY_FILTERS, query: "shell" }),
    ).toHaveLength(1);
    // Notes are searchable (the feedback's explicit ask).
    expect(
      filterTransactions(rows, { ...EMPTY_FILTERS, query: "piano" }),
    ).toHaveLength(1);
    expect(
      filterTransactions(rows, { ...EMPTY_FILTERS, query: "savings" }),
    ).toHaveLength(1);
    expect(
      filterTransactions(rows, { ...EMPTY_FILTERS, query: "zzz" }),
    ).toHaveLength(0);
  });

  it("filters by inclusive date range on the calendar day", () => {
    const dated = [
      row({ occurred_at: "2026-06-01T09:00:00Z" }),
      row({ occurred_at: "2026-06-15T09:00:00Z" }),
      row({ occurred_at: "2026-07-01T09:00:00Z" }),
    ];
    const june = filterTransactions(dated, {
      ...EMPTY_FILTERS,
      from: "2026-06-01",
      to: "2026-06-30",
    });
    expect(june).toHaveLength(2);
  });

  it("filters by tag, account, and the uncategorized sentinel", () => {
    const mixed = [
      row({ tag_ids: ["vacation"], category_id: "cat-1" }),
      row({ account_id: "acct-2" }),
    ];
    expect(
      filterTransactions(mixed, { ...EMPTY_FILTERS, tagId: "vacation" }),
    ).toHaveLength(1);
    expect(
      filterTransactions(mixed, { ...EMPTY_FILTERS, accountIds: ["acct-2"] }),
    ).toHaveLength(1);
    expect(
      filterTransactions(mixed, { ...EMPTY_FILTERS, categoryId: "uncategorized" }),
    ).toHaveLength(1);
    expect(
      filterTransactions(mixed, { ...EMPTY_FILTERS, categoryId: "cat-1" }),
    ).toHaveLength(1);
  });

  it("unreviewedOnly keeps only unreviewed rows", () => {
    const mixed = [row({ reviewed: false }), row({ reviewed: true })];
    const unreviewed = filterTransactions(mixed, {
      ...EMPTY_FILTERS,
      unreviewedOnly: true,
    });
    expect(unreviewed).toHaveLength(1);
    expect(unreviewed[0]!.reviewed).toBe(false);
  });
});

describe("sortTransactions", () => {
  const rows = [
    row({ occurred_at: "2026-06-01T00:00:00Z", amount: { minor_units: -500, currency: "USD" } }),
    row({ occurred_at: "2026-07-01T00:00:00Z", amount: { minor_units: 2000, currency: "USD" } }),
  ];

  it("orders by date and amount without mutating the input", () => {
    const newest = sortTransactions(rows, "newest");
    expect(newest[0]!.occurred_at).toContain("2026-07-01");
    const oldest = sortTransactions(rows, "oldest");
    expect(oldest[0]!.occurred_at).toContain("2026-06-01");
    const largest = sortTransactions(rows, "amount_desc");
    expect(largest[0]!.amount.minor_units).toBe(2000);
    const smallest = sortTransactions(rows, "amount_asc");
    expect(smallest[0]!.amount.minor_units).toBe(-500);
    // The source order is untouched.
    expect(rows[0]!.occurred_at).toContain("2026-06-01");
  });
});

describe("activeFilterCount", () => {
  it("counts facets but not the search query", () => {
    expect(activeFilterCount(EMPTY_FILTERS)).toBe(0);
    expect(activeFilterCount({ ...EMPTY_FILTERS, query: "x" })).toBe(0);
    expect(
      activeFilterCount({
        ...EMPTY_FILTERS,
        accountIds: ["a"],
        from: "2026-01-01",
        unreviewedOnly: true,
      }),
    ).toBe(3);
  });
});

describe("the account facet is a set (personal-cfo-4d8.27.9.4, ADR 0057 §3)", () => {
  it("passes every selected account through to the page input", () => {
    // The Debt page scopes its embedded list to a multi-account selection. A mapping
    // that dropped all but the first id would look correct on every single-account
    // screen in the app and silently under-report on the one page that needs it.
    const input = toTransactionPageInput(
      { ...EMPTY_FILTERS, accountIds: ["a", "b", "c"] },
      "newest",
      0,
      10,
    );
    expect(input.account_ids).toEqual(["a", "b", "c"]);
  });

  it("sends an empty set for no constraint, not a null", () => {
    // Empty means "no constraint" on both sides. Inverting that would return nothing for
    // every unfiltered read in the app.
    const input = toTransactionPageInput(EMPTY_FILTERS, "newest", 0, 10);
    expect(input.account_ids).toEqual([]);
  });

  it("counts an account scope once however many accounts it holds", () => {
    // The badge says how many FACETS narrow the list, not how many accounts — three
    // selected accounts are still one constraint.
    expect(
      activeFilterCount({ ...EMPTY_FILTERS, accountIds: ["a", "b", "c"] }),
    ).toBe(1);
  });

  it("keeps rows on any selected account and drops the rest", () => {
    const rows = [
      row({ transaction_id: "1", account_id: "a" }),
      row({ transaction_id: "2", account_id: "b" }),
      row({ transaction_id: "3", account_id: "c" }),
    ];
    const kept = filterTransactions(rows, {
      ...EMPTY_FILTERS,
      accountIds: ["a", "c"],
    });
    expect(kept.map((r) => r.account_id)).toEqual(["a", "c"]);
  });
});
