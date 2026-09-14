import { fireEvent, screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { TransactionPageInput, TransactionRowDto } from "@/bindings";
import { matchesQuery } from "@/transactions/filters";
import { GlobalSearch } from "./GlobalSearch";

const mocks = vi.hoisted(() => ({
  transactionPage: vi.fn(),
  tagList: vi.fn(),
  categoryList: vi.fn(),
  // The detail drawer loads these on open.
  transactionAttachments: vi.fn(),
  transactionSplits: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    transactionPage: mocks.transactionPage,
    tagList: mocks.tagList,
    categoryList: mocks.categoryList,
    transactionAttachments: mocks.transactionAttachments,
    transactionSplits: mocks.transactionSplits,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function txn(over: Partial<TransactionRowDto> = {}): TransactionRowDto {
  return {
    transaction_id: "0190b000-0000-7000-8000-000000000001",
    account_id: "0190a000-0000-7000-8000-000000000001",
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-05T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -1250, currency: "USD" },
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

/// One hit via memo, one via note (the title falls back to the counterparty),
/// and one decoy that matches neither.
const ROWS: TransactionRowDto[] = [
  txn({
    transaction_id: "0190b000-0000-7000-8000-000000000001",
    memo: "Blue Bottle Coffee",
    occurred_at: "2026-06-05T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
  }),
  txn({
    transaction_id: "0190b000-0000-7000-8000-000000000002",
    counterparty: "Landlord LLC",
    note: "coffee fund reimbursement",
    occurred_at: "2026-06-04T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
  }),
  txn({
    transaction_id: "0190b000-0000-7000-8000-000000000003",
    memo: "Rent",
    occurred_at: "2026-06-03T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
  }),
];

const onClose = vi.fn();

function renderSearch() {
  return renderWithClient(<GlobalSearch onClose={onClose} />);
}

beforeEach(() => {
  onClose.mockReset();
  mocks.transactionPage.mockReset();
  // A fake server over ROWS: same matching semantics as db-worker's SQL
  // (`matchesQuery` is the reference implementation), newest first, capped.
  mocks.transactionPage.mockImplementation((input: TransactionPageInput) => {
    const hits = ROWS.filter((t) => matchesQuery(t, input.query ?? ""))
      .sort((a, b) => b.occurred_at.localeCompare(a.occurred_at));
    return Promise.resolve(
      ok({ rows: hits.slice(0, input.limit), total: hits.length }),
    );
  });
  mocks.tagList.mockReset();
  mocks.tagList.mockResolvedValue(ok([]));
  mocks.categoryList.mockReset();
  mocks.categoryList.mockResolvedValue(ok([]));
  mocks.transactionAttachments.mockReset();
  mocks.transactionAttachments.mockResolvedValue(ok([]));
  mocks.transactionSplits.mockReset();
  mocks.transactionSplits.mockResolvedValue(ok([]));
});

describe("GlobalSearch", () => {
  it("shows a hint (not results) while the query is empty", () => {
    renderSearch();
    expect(
      screen.getByText("Search all your transactions"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Blue Bottle Coffee")).not.toBeInTheDocument();
  });

  it("narrows results across memo AND note fields", async () => {
    renderSearch();
    fireEvent.change(
      screen.getByRole("searchbox", { name: "Search transactions" }),
      { target: { value: "coffee" } },
    );
    // Memo hit + note hit (titled by its counterparty), decoy filtered out.
    expect(await screen.findByText("Blue Bottle Coffee")).toBeInTheDocument();
    expect(screen.getByText("Landlord LLC")).toBeInTheDocument();
    expect(screen.queryByText("Rent")).not.toBeInTheDocument();
  });

  it("opens the full detail drawer on click", async () => {
    renderSearch();
    fireEvent.change(
      screen.getByRole("searchbox", { name: "Search transactions" }),
      { target: { value: "blue bottle" } },
    );
    fireEvent.click(await screen.findByText("Blue Bottle Coffee"));
    expect(
      await screen.findByRole("dialog", { name: "Transaction detail" }),
    ).toBeInTheDocument();
  });

  it("Esc closes the drawer first, then the palette", async () => {
    renderSearch();
    fireEvent.change(
      screen.getByRole("searchbox", { name: "Search transactions" }),
      { target: { value: "blue bottle" } },
    );
    fireEvent.click(await screen.findByText("Blue Bottle Coffee"));
    await screen.findByRole("dialog", { name: "Transaction detail" });

    fireEvent.keyDown(window, { key: "Escape" });
    expect(
      screen.queryByRole("dialog", { name: "Transaction detail" }),
    ).not.toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("closes on backdrop click but not on clicks inside the palette", () => {
    renderSearch();
    fireEvent.click(
      screen.getByRole("searchbox", { name: "Search transactions" }),
    );
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getByRole("dialog", { name: "Search transactions" }),
    );
    expect(onClose).toHaveBeenCalled();
  });

  it("shows a compact empty state when nothing matches", async () => {
    renderSearch();
    fireEvent.change(
      screen.getByRole("searchbox", { name: "Search transactions" }),
      { target: { value: "zzz-no-such-merchant" } },
    );
    expect(
      await screen.findByText("No matching transactions"),
    ).toBeInTheDocument();
  });
});
