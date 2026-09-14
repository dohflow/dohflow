import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  AccountViewDto,
  CategoryDto,
  TransactionPageInput,
  TransactionRowDto,
} from "@/bindings";
import { TransactionsView } from "./TransactionsView";
import {
  filterTransactions,
  sortTransactions,
  type TransactionFilters,
  type TransactionSort,
} from "./filters";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  transactionPage: vi.fn(),
  recordTransaction: vi.fn(),
  recordTransfer: vi.fn(),
  recurringTransferList: vi.fn(),
  createRecurringTransfer: vi.fn(),
  deleteRecurringTransfer: vi.fn(),
  recurringBillList: vi.fn(),
  createRecurringBill: vi.fn(),
  categoryList: vi.fn(),
  recategorizeTransaction: vi.fn(),
  applyMerchantMemory: vi.fn(),
  tagList: vi.fn(),
  transactionSplits: vi.fn(),
  setTags: vi.fn(),
  setNote: vi.fn(),
  createTag: vi.fn(),
  attachDocument: vi.fn(),
  spendByCategory: vi.fn(),
  baseCurrency: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    transactionPage: mocks.transactionPage,
    recordTransaction: mocks.recordTransaction,
    recordTransfer: mocks.recordTransfer,
    recurringTransferList: mocks.recurringTransferList,
    createRecurringTransfer: mocks.createRecurringTransfer,
    deleteRecurringTransfer: mocks.deleteRecurringTransfer,
    recurringBillList: mocks.recurringBillList,
    createRecurringBill: mocks.createRecurringBill,
    categoryList: mocks.categoryList,
    recategorizeTransaction: mocks.recategorizeTransaction,
    applyMerchantMemory: mocks.applyMerchantMemory,
    tagList: mocks.tagList,
    transactionSplits: mocks.transactionSplits,
    setTags: mocks.setTags,
    setNote: mocks.setNote,
    createTag: mocks.createTag,
    attachDocument: mocks.attachDocument,
    spendByCategory: mocks.spendByCategory,
    baseCurrency: mocks.baseCurrency,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const err = (message: string) =>
  ({ status: "error", error: { kind: "Internal", message } }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });
// The new transaction id record_transaction returns (personal-cfo-4d8.24.2.1) — the
// inline metadata follow-ups must be keyed on THIS id.
const NEW_TXN_ID = "0190b000-0000-7000-8000-0000000000ff";
const VACATION_TAG = "0190d000-0000-7000-8000-0000000000d1";
const recordOk = () =>
  ok({ transaction_id: NEW_TXN_ID, result: { op_seq: 1, replayed: false } });

/// A fake `transactionPage` server: filters / sorts / windows `list` with the
/// reference pure functions from filters.ts, exactly as db-worker's SQL does.
function pageResult(list: TransactionRowDto[], input: TransactionPageInput) {
  const filters: TransactionFilters = {
    query: input.query ?? "",
    accountIds: input.account_ids ?? [],
    categoryId: input.category_id ?? "",
    tagId: input.tag_id ?? "",
    recurringEventId: input.recurring_event_id ?? "",
    from: input.from_date ?? "",
    to: input.to_date ?? "",
    unreviewedOnly: input.unreviewed_only,
  };
  const visible = sortTransactions(
    filterTransactions(list, filters),
    input.sort as TransactionSort,
  );
  return ok({
    rows: visible.slice(input.offset, input.offset + input.limit),
    total: visible.length,
  });
}

/// Wrap spend rows in the breakdown envelope the aggregate now returns
/// (personal-cfo-90eg): the rows plus what the same query excluded.
function breakdown(
  rows: unknown[],
  over: { uncategorized_minor?: number; transfers_minor?: number } = {},
) {
  return {
    rows,
    uncategorized_minor: over.uncategorized_minor ?? 0,
    transfers_minor: over.transfers_minor ?? 0,
  };
}

/// Serve every subsequent `transactionPage` call from `list`.
function mockTransactionPage(list: TransactionRowDto[]) {
  mocks.transactionPage.mockImplementation((input: TransactionPageInput) =>
    Promise.resolve(pageResult(list, input)),
  );
}

function account(over: Partial<AccountViewDto> = {}): AccountViewDto {
  return {
    id: "0190a000-0000-7000-8000-000000000001",
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: { minor_units: 125_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

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
    amount: { minor_units: -4000, currency: "USD" },
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

const CATEGORY: CategoryDto = {
  id: "0190c000-0000-7000-8000-000000000001",
  parent_id: null,
  name: "Groceries",
  category_type: "expense",
  icon: null,
  color: null,
  is_system: true,
  forecast_behavior: "variable_regular",
  archived: false,
};

// A second liquid account, so the "Add transfer" affordance appears.
const SAVINGS = account({
  id: "0190a000-0000-7000-8000-000000000002",
  name: "Savings",
});

beforeEach(() => {
  mocks.accountList.mockReset();
  mocks.transactionPage.mockReset();
  mocks.recordTransaction.mockReset();
  mocks.recordTransfer.mockReset();
  mocks.recurringTransferList.mockReset();
  mocks.createRecurringTransfer.mockReset();
  mocks.deleteRecurringTransfer.mockReset();
  mocks.categoryList.mockReset();
  mocks.recategorizeTransaction.mockReset();
  mocks.accountList.mockResolvedValue(ok([account()]));
  mockTransactionPage([]);
  mocks.tagList.mockReset();
  mocks.tagList.mockResolvedValue(ok([]));
  mocks.spendByCategory.mockReset();
  mocks.spendByCategory.mockResolvedValue(ok(breakdown([])));
  mocks.baseCurrency.mockReset();
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.transactionSplits.mockReset();
  mocks.transactionSplits.mockResolvedValue(ok([]));
  mocks.categoryList.mockResolvedValue(ok([CATEGORY]));
  mocks.recurringTransferList.mockResolvedValue(ok([]));
  mocks.createRecurringTransfer.mockResolvedValue(
    ok({ recurring_transfer_id: "rt-1", mutation: { op_seq: 1, replayed: false } }),
  );
  mocks.recurringBillList.mockReset();
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.createRecurringBill.mockReset();
  mocks.createRecurringBill.mockResolvedValue(mutationOk());
  mocks.recordTransfer.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  // record_transaction returns the new id + the mutation outcome (4d8.24.2.1).
  mocks.recordTransaction.mockResolvedValue(recordOk());
  mocks.recategorizeTransaction.mockResolvedValue(mutationOk());
  mocks.setTags.mockReset();
  mocks.setTags.mockResolvedValue(mutationOk());
  mocks.setNote.mockReset();
  mocks.setNote.mockResolvedValue(mutationOk());
  mocks.createTag.mockReset();
  mocks.createTag.mockResolvedValue(ok({ tag_id: VACATION_TAG }));
  mocks.applyMerchantMemory.mockReset();
  mocks.applyMerchantMemory.mockResolvedValue(ok(0));
  mocks.attachDocument.mockReset();
  mocks.attachDocument.mockResolvedValue(
    ok({
      id: "0190c000-0000-7000-8000-000000000001",
      mime_type: "application/pdf",
      original_filename: "receipt.pdf",
      plaintext_size: 3,
      created_at: "2026-06-05T00:00:00Z",
    }),
  );
});

/// A jsdom `File` with a working `arrayBuffer()` (jsdom omits it; the attach path awaits it).
function pickable(name: string, type: string, bytes: number[]): File {
  const file = new File([new Uint8Array(bytes)], name, { type });
  file.arrayBuffer = () => Promise.resolve(new Uint8Array(bytes).buffer);
  return file;
}

describe("TransactionsView", () => {
  it("prompts to add an account when there are none", async () => {
    mocks.accountList.mockResolvedValue(ok([]));
    renderWithClient(<TransactionsView />);
    expect(await screen.findByText(/add an account first/i)).toBeInTheDocument();
  });

  it("shows the empty state when there are accounts but no transactions", async () => {
    renderWithClient(<TransactionsView />);
    expect(await screen.findByText(/no transactions yet/i)).toBeInTheDocument();
  });

  it("lists transactions with a signed amount", async () => {
    mockTransactionPage([txn({ amount: { minor_units: 15_000, currency: "USD" } })]);
    renderWithClient(<TransactionsView />);
    // The account has its own column now; scope to the row so the account filter's
    // option of the same name is not ambiguous.
    const amountRow = await screen.findByText(/\+\$?150/);
    expect(
      within(amountRow.closest("tr")!).getByText("Checking"),
    ).toBeInTheDocument();
    // Income renders with a leading "+" and the major-unit amount.
    const amount = screen.getByText(/\+\$?150/);
    expect(amount).toBeInTheDocument();
    // The amount lives in a right-aligned, tabular table cell (personal-cfo-4d8.24.11).
    expect(amount.closest("td")).toHaveClass("text-right", "tabular-nums");
  });

  it("paginates the list past 10 transactions with a sticky page size", async () => {
    window.localStorage.clear();
    const txns = Array.from({ length: 12 }, (_, i) =>
      txn({
        transaction_id: `0190b000-0000-7000-8000-0000000000${String(i + 10)}`,
        memo: `Txn ${i + 1}`,
      }),
    );
    mockTransactionPage(txns);
    renderWithClient(<TransactionsView />);

    // Page 1: the first 10 (the server window), with the server-side total.
    expect(await screen.findByText("Txn 1")).toBeInTheDocument();
    expect(screen.getByText("Txn 10")).toBeInTheDocument();
    expect(screen.queryByText("Txn 11")).toBeNull();
    // The scope strip states the same count (personal-cfo-3tbn), so this asserts on the
    // PAGER's copy specifically — a bare match would now find two and prove neither.
    expect(
      screen.getAllByText(/of 12 transactions/).length,
    ).toBeGreaterThan(0);

    // Next page: the remainder arrives from the next server window.
    fireEvent.click(screen.getByRole("button", { name: /next page/i }));
    expect(await screen.findByText("Txn 11")).toBeInTheDocument();
    expect(screen.getByText("Txn 12")).toBeInTheDocument();
    expect(screen.queryByText("Txn 1")).toBeNull();
  });

  it("categorizes a transaction inline from the list (4d8.13)", async () => {
    mockTransactionPage([txn({ memo: "Coffee" })]);
    renderWithClient(<TransactionsView />);
    await screen.findByText("Coffee");

    // The row's category control defaults to "Uncategorized" (empty value).
    const select = screen.getByRole("combobox", { name: /category/i });
    expect(select).toHaveValue("");

    // Picking a category recategorizes that transaction via the existing IPC.
    fireEvent.change(select, { target: { value: CATEGORY.id } });
    await waitFor(() =>
      expect(mocks.recategorizeTransaction).toHaveBeenCalledWith(
        "0190b000-0000-7000-8000-000000000001",
        CATEGORY.id,
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("auto-categorizes from merchant memory and reports the count (5n4.1)", async () => {
    mockTransactionPage([txn({ memo: "Coffee" })]);
    mocks.applyMerchantMemory.mockResolvedValue(ok(3));
    renderWithClient(<TransactionsView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: /auto-categorize/i }));

    await waitFor(() => expect(mocks.applyMerchantMemory).toHaveBeenCalled());
    expect(
      await screen.findByText(/categorized 3 transactions/i),
    ).toBeInTheDocument();
  });

  it("reports when auto-categorize finds nothing to do (5n4.1)", async () => {
    mockTransactionPage([txn({ memo: "Coffee" })]);
    mocks.applyMerchantMemory.mockResolvedValue(ok(0));
    renderWithClient(<TransactionsView />);
    await screen.findByText("Coffee");

    fireEvent.click(screen.getByRole("button", { name: /auto-categorize/i }));
    expect(
      await screen.findByText(/no new transactions to categorize/i),
    ).toBeInTheDocument();
  });

  it("badges an auto-categorized row with its source and confidence (5n4.1)", async () => {
    mockTransactionPage([
      txn({
        memo: "Coffee",
        category_id: CATEGORY.id,
        category_source: "rule",
        category_confidence_bps: 8200,
      }),
    ]);
    renderWithClient(<TransactionsView />);
    await screen.findByText("Coffee");
    // The "Auto" provenance badge shows the rounded confidence percent.
    expect(await screen.findByText(/Auto · 82%/)).toBeInTheDocument();
  });

  it("shows no provenance badge for a user-categorized row (5n4.1)", async () => {
    mockTransactionPage([
      txn({
        memo: "Coffee",
        category_id: CATEGORY.id,
        category_source: "user",
        category_confidence_bps: 10_000,
      }),
    ]);
    renderWithClient(<TransactionsView />);
    await screen.findByText("Coffee");
    expect(screen.queryByText(/Auto ·/)).toBeNull();
  });

  it("shows an imported transaction's merchant/memo as the title (byxe)", async () => {
    mockTransactionPage([txn({ memo: "Coffee", counterparty: "coffee" })]);
    renderWithClient(<TransactionsView />);
    // The memo is the row title; the account now has its OWN column rather than being
    // concatenated into a subtitle (personal-cfo-4d8.27.8.3).
    expect(await screen.findByText("Coffee")).toBeInTheDocument();
    const row = screen.getAllByRole("row").find((r) => within(r).queryByText("Coffee"));
    expect(within(row!).getByText("Checking")).toBeInTheDocument();
  });

  it("records an expense with the correct signed amount and refreshes", async () => {
    // First fetch: an empty vault; after the mutation invalidates, the new row.
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([txn()]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    // Expense is the default type; fill amount + date and submit.
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "40" },
    });
    fireEvent.change(screen.getByLabelText(/date/i), {
      target: { value: "2026-06-05" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    await screen.findByText("Checking");
    expect(mocks.recordTransaction).toHaveBeenCalledTimes(1);
    const input = mocks.recordTransaction.mock.calls[0]?.[0];
    expect(input.amount.minor_units).toBe(-4000); // expense → negative
    expect(input.account_id).toBe(account().id);
    expect(input.occurred_at).toBe("2026-06-05T00:00:00Z");
  });

  it("promotes to a recurring bill from the add form (4d8.24.2.2)", async () => {
    mockTransactionPage([]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    // Toggle recurring mode → the bill name + frequency fields appear.
    fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
    fireEvent.change(screen.getByLabelText(/bill name/i), {
      target: { value: "Spotify" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "10.99" },
    });
    fireEvent.change(screen.getByLabelText(/first due date/i), {
      target: { value: "2026-07-05" },
    });
    fireEvent.change(screen.getByLabelText(/frequency/i), {
      target: { value: "monthly" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /create recurring bill/i }),
    );

    await waitFor(() =>
      expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1),
    );
    const bill = mocks.createRecurringBill.mock.calls[0]?.[0];
    expect(bill).toMatchObject({
      name: "Spotify",
      bill_type: "subscription",
      frequency: "monthly",
      anchor_date: "2026-07-05",
      autopay_account_id: account().id,
    });
    expect(bill.amount.minor_units).toBe(1099); // positive per-occurrence magnitude
    // Recurring is an alternative, not an addition — no one-off record.
    expect(mocks.recordTransaction).not.toHaveBeenCalled();
  });

  it("requires a name before creating a recurring bill (4d8.24.2.2)", async () => {
    mockTransactionPage([]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "10" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /create recurring bill/i }),
    );

    expect(
      await screen.findByText(/enter a name for the recurring bill/i),
    ).toBeInTheDocument();
    expect(mocks.createRecurringBill).not.toHaveBeenCalled();
  });

  it("attaches a staged document to the new transaction after recording (4d8.24.2.3)", async () => {
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "40" },
    });
    fireEvent.change(screen.getByLabelText(/date/i), {
      target: { value: "2026-06-05" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    fireEvent.change(screen.getByLabelText(/attach documents/i), {
      target: { files: [pickable("receipt.pdf", "application/pdf", [1, 2, 3])] },
    });
    expect(await screen.findByText("receipt.pdf")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    await waitFor(() =>
      expect(mocks.recordTransaction).toHaveBeenCalledTimes(1),
    );
    await waitFor(() =>
      expect(mocks.attachDocument).toHaveBeenCalledTimes(1),
    );
    const [id, name, mime, data] = mocks.attachDocument.mock.calls[0] ?? [];
    expect(id).toBe(NEW_TXN_ID); // the RETURNED id, not a placeholder
    expect(name).toBe("receipt.pdf");
    expect(mime).toBe("application/pdf");
    expect(data).toEqual([1, 2, 3]); // raw bytes
    // Ordering: the record happens before the attach.
    expect(mocks.recordTransaction.mock.invocationCallOrder[0] ?? 0).toBeLessThan(
      mocks.attachDocument.mock.invocationCallOrder[0] ?? 0,
    );
  });

  it("drops a staged document when removed before submit (4d8.24.2.3)", async () => {
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "40" },
    });
    fireEvent.change(screen.getByLabelText(/date/i), {
      target: { value: "2026-06-05" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    fireEvent.change(screen.getByLabelText(/attach documents/i), {
      target: { files: [pickable("receipt.pdf", "application/pdf", [1, 2, 3])] },
    });
    expect(await screen.findByText("receipt.pdf")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /remove receipt\.pdf/i }));
    expect(screen.queryByText("receipt.pdf")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));
    await waitFor(() =>
      expect(mocks.recordTransaction).toHaveBeenCalledTimes(1),
    );
    expect(mocks.attachDocument).not.toHaveBeenCalled();
  });

  it("keeps the transaction and warns when an attachment fails (4d8.24.2.3)", async () => {
    mocks.attachDocument.mockResolvedValue(err("attach exploded"));
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "40" },
    });
    fireEvent.change(screen.getByLabelText(/date/i), {
      target: { value: "2026-06-05" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    fireEvent.change(screen.getByLabelText(/attach documents/i), {
      target: { files: [pickable("receipt.pdf", "application/pdf", [1, 2, 3])] },
    });
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    // The row is saved (record called), the attach failed → a non-blocking warning.
    expect(
      await screen.findByText(/could not be saved/i),
    ).toBeInTheDocument();
    expect(mocks.recordTransaction).toHaveBeenCalledTimes(1);
    expect(mocks.attachDocument).toHaveBeenCalledTimes(1);
    // The form closed (add-details button gone) — the row persisted despite the attach failure.
    expect(
      screen.queryByRole("button", { name: /add details/i }),
    ).not.toBeInTheDocument();
  });

  it("hides the attachment picker when creating a recurring bill (4d8.24.2.3)", async () => {
    renderWithClient(<TransactionsView />);
    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    // The picker is present for a one-off transaction…
    expect(screen.getByLabelText(/attach documents/i)).toBeInTheDocument();
    // …and gone once "recurring" is on (a bill has no transaction id to attach to).
    fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
    expect(screen.queryByLabelText(/attach documents/i)).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText(/bill name/i), {
      target: { value: "Netflix" },
    });
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "15" },
    });
    fireEvent.change(screen.getByLabelText(/first due date/i), {
      target: { value: "2026-07-05" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /create recurring bill/i }),
    );
    await waitFor(() =>
      expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1),
    );
    expect(mocks.attachDocument).not.toHaveBeenCalled();
  });

  it("adds a transaction with category + tags + note in one flow (4d8.24.2)", async () => {
    // A pre-existing tag so the flow selects rather than mints one.
    mocks.tagList.mockResolvedValue(
      ok([{ id: VACATION_TAG, name: "Vacation", color: null, archived: false }]),
    );
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([
      txn({
        transaction_id: NEW_TXN_ID,
        category_id: CATEGORY.id,
        tag_ids: [VACATION_TAG],
        note: "Groceries run",
      }),
    ]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "40" } });
    fireEvent.change(screen.getByLabelText(/date/i), {
      target: { value: "2026-06-05" },
    });

    // Reveal the inline metadata, then set category, a tag, and a note.
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    fireEvent.change(screen.getByLabelText("Category"), {
      target: { value: CATEGORY.id },
    });
    fireEvent.change(screen.getByLabelText(/add a tag/i), {
      target: { value: "Vacation" },
    });
    fireEvent.keyDown(screen.getByLabelText(/add a tag/i), { key: "Enter" });
    fireEvent.change(screen.getByLabelText("Note"), {
      target: { value: "Groceries run" },
    });

    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    // The record runs first; each follow-up is keyed on the RETURNED id.
    await waitFor(() =>
      expect(mocks.recordTransaction).toHaveBeenCalledTimes(1),
    );
    await waitFor(() =>
      expect(mocks.recategorizeTransaction).toHaveBeenCalledWith(
        NEW_TXN_ID,
        CATEGORY.id,
        expect.stringMatching(/.+/),
      ),
    );
    await waitFor(() =>
      expect(mocks.setTags).toHaveBeenCalledWith(
        NEW_TXN_ID,
        [VACATION_TAG],
        expect.stringMatching(/.+/),
      ),
    );
    await waitFor(() =>
      expect(mocks.setNote).toHaveBeenCalledWith(
        NEW_TXN_ID,
        "Groceries run",
        expect.stringMatching(/.+/),
      ),
    );
  });

  it("keeps the transaction and warns when a metadata follow-up fails (4d8.24.2)", async () => {
    mocks.tagList.mockResolvedValue(
      ok([{ id: VACATION_TAG, name: "Vacation", color: null, archived: false }]),
    );
    // The tag write fails, but the record already succeeded.
    mocks.setTags.mockResolvedValue(err("tags exploded"));
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "40" } });
    fireEvent.change(screen.getByLabelText(/date/i), {
      target: { value: "2026-06-05" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    fireEvent.change(screen.getByLabelText(/add a tag/i), {
      target: { value: "Vacation" },
    });
    fireEvent.keyDown(screen.getByLabelText(/add a tag/i), { key: "Enter" });
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    // The transaction is saved (never rolled back), the form closes, and a
    // non-blocking notice appears.
    expect(
      await screen.findByText(/some details .* could not be saved/i),
    ).toBeInTheDocument();
    expect(mocks.recordTransaction).toHaveBeenCalledTimes(1);
    expect(mocks.setTags).toHaveBeenCalledTimes(1);
    // The add form is gone (record succeeded → closed).
    expect(
      screen.queryByRole("button", { name: /add details/i }),
    ).not.toBeInTheDocument();
  });

  it("fires no metadata follow-ups on the fast path (4d8.24.2)", async () => {
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "40" } });
    fireEvent.change(screen.getByLabelText(/date/i), {
      target: { value: "2026-06-05" },
    });
    // Submit WITHOUT opening "Add details".
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    await waitFor(() =>
      expect(mocks.recordTransaction).toHaveBeenCalledTimes(1),
    );
    // Wait for the refreshed row. The account column and the filter option share the name,
    // and the scope strip now totals the same amount, so match on the ROW cell rather than
    // the first $40 on screen.
    await vi.waitFor(() =>
      expect(
        screen.getAllByText(/\$40/).some((el) => el.closest("tr") !== null),
      ).toBe(true),
    );
    expect(mocks.recategorizeTransaction).not.toHaveBeenCalled();
    expect(mocks.setTags).not.toHaveBeenCalled();
    expect(mocks.setNote).not.toHaveBeenCalled();
  });

  it("shows the assigned category as a chip on the row (bac)", async () => {
    mockTransactionPage([txn({ category_id: CATEGORY.id })]);
    renderWithClient(<TransactionsView />);
    expect(await screen.findByText("Groceries")).toBeInTheDocument();
  });

  it("shows a child category leaf-only on the row chip, not the Parent / Leaf path (4d8.24.9)", async () => {
    const parent: CategoryDto = { ...CATEGORY, id: "0190c000-0000-7000-8000-0000000000f0", name: "Food and Drink" };
    const child: CategoryDto = { ...CATEGORY, id: "0190c000-0000-7000-8000-0000000000f1", name: "Groceries", parent_id: parent.id };
    mocks.categoryList.mockResolvedValue(ok([parent, child]));
    mockTransactionPage([txn({ category_id: child.id })]);
    renderWithClient(<TransactionsView />);

    const select = await screen.findByLabelText<HTMLSelectElement>("Category");
    // The chip (the collapsed select) shows the leaf's own name.
    expect(select.value).toBe(child.id);
    const selected = select.querySelector(`option[value="${child.id}"]`);
    expect(selected?.textContent).toBe("Groceries");
    // No option carries the full "Food and Drink / Groceries" path…
    expect(
      screen.queryByRole("option", { name: "Food and Drink / Groceries" }),
    ).not.toBeInTheDocument();
    // …instead the leaf is disambiguated under its parent <optgroup>.
    expect(
      select.querySelector('optgroup[label="Food and Drink"]'),
    ).not.toBeNull();
  });

  it("renders the category's emoji on a colored swatch, and the toggle hides it (4d8.24.10)", async () => {
    window.localStorage.removeItem("pcfo.txnCategoryIcons");
    const iconCategory: CategoryDto = {
      ...CATEGORY,
      id: "0190c000-0000-7000-8000-0000000000f2",
      name: "Coffee",
      icon: "☕",
      color: "#006341",
    };
    mocks.categoryList.mockResolvedValue(ok([iconCategory]));
    mockTransactionPage([txn({ category_id: iconCategory.id })]);
    renderWithClient(<TransactionsView />);

    // Default (icons on): the emoji swatch renders on the row.
    expect(await screen.findByText("☕")).toBeInTheDocument();

    // Toggling "Show category icons" off hides the swatch (name select stays).
    fireEvent.click(screen.getByRole("checkbox", { name: /show category icons/i }));
    await waitFor(() =>
      expect(screen.queryByText("☕")).not.toBeInTheDocument(),
    );
    expect(screen.getByLabelText("Category")).toBeInTheDocument();
  });

  it("hides Add transfer with fewer than two liquid accounts (npoe)", async () => {
    renderWithClient(<TransactionsView />); // default: one account
    await screen.findByRole("button", { name: /add transaction/i });
    expect(
      screen.queryByRole("button", { name: /add transfer/i }),
    ).not.toBeInTheDocument();
  });

  it("records a transfer between two accounts (npoe)", async () => {
    mocks.accountList.mockResolvedValue(ok([account(), SAVINGS]));
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transfer/i }));
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "50" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
    const input = mocks.recordTransfer.mock.calls[0]?.[0];
    expect(input.source_account_id).toBe(account().id);
    expect(input.dest_account_id).toBe(SAVINGS.id);
    expect(input.amount.minor_units).toBe(5000);
  });

  it("schedules a recurring transfer when Recurring is chosen (npoe)", async () => {
    mocks.accountList.mockResolvedValue(ok([account(), SAVINGS]));
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transfer/i }));
    fireEvent.click(screen.getByRole("button", { name: "Recurring" }));
    fireEvent.change(screen.getByLabelText(/amount/i), {
      target: { value: "200" },
    });
    fireEvent.change(screen.getByLabelText(/frequency/i), {
      target: { value: "weekly" },
    });
    fireEvent.click(screen.getByRole("button", { name: /schedule transfer/i }));

    await waitFor(() =>
      expect(mocks.createRecurringTransfer).toHaveBeenCalledTimes(1),
    );
    const input = mocks.createRecurringTransfer.mock.calls[0]?.[0];
    expect(input.source_account_id).toBe(account().id);
    expect(input.dest_account_id).toBe(SAVINGS.id);
    expect(input.amount.minor_units).toBe(20000);
    expect(input.frequency).toBe("weekly");
    expect(mocks.recordTransfer).not.toHaveBeenCalled();
  });

  const BROKERAGE = account({
    id: "0190a000-0000-7000-8000-000000000003",
    name: "Brokerage",
    cashflow_role: "investment_asset",
    balance: { minor_units: 900_000, currency: "USD" },
  });
  const CARD = account({
    id: "0190a000-0000-7000-8000-000000000004",
    name: "Visa",
    cashflow_role: "credit_facility",
    balance: { minor_units: -120_000, currency: "USD" },
  });
  const toSelect = () => screen.getByLabelText(/^to$/i);
  const fromSelect = () => screen.getByLabelText(/^from$/i);

  it("records a contribution to an investment account (9h0.1)", async () => {
    mocks.accountList.mockResolvedValue(ok([account(), BROKERAGE]));
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transfer/i }));
    fireEvent.change(toSelect(), { target: { value: BROKERAGE.id } });
    fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "100" } });
    expect(screen.getByText(/contributing to your investment/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
    const input = mocks.recordTransfer.mock.calls[0]?.[0];
    expect(input.source_account_id).toBe(account().id);
    expect(input.dest_account_id).toBe(BROKERAGE.id);
  });

  it("offers a card as a one-off payment destination but not for a recurring transfer", async () => {
    mocks.accountList.mockResolvedValue(ok([account(), BROKERAGE, CARD]));
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transfer/i }));
    // One-off: the card is a valid destination (a debt payment, r7sb).
    expect(within(toSelect()).getByRole("option", { name: /Visa/ })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Recurring" }));
    // Recurring: the card drops out (recurring debt is the debt overlay), the investment stays.
    await waitFor(() =>
      expect(within(toSelect()).queryByRole("option", { name: /Visa/ })).toBeNull(),
    );
    expect(within(toSelect()).getByRole("option", { name: /Brokerage/ })).toBeInTheDocument();
  });

  it("an investment source can only withdraw to cash (j0cg.2)", async () => {
    mocks.accountList.mockResolvedValue(ok([account(), BROKERAGE, CARD]));
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transfer/i }));
    fireEvent.change(fromSelect(), { target: { value: BROKERAGE.id } });

    // From a brokerage, the only destination is the liquid account — no card, no other investment.
    await waitFor(() =>
      expect(within(toSelect()).getByRole("option", { name: /Checking/ })).toBeInTheDocument(),
    );
    expect(within(toSelect()).queryByRole("option", { name: /Visa/ })).toBeNull();
    expect(screen.getByText(/withdrawing from your investment/i)).toBeInTheDocument();
  });

  it("disables Recurring and dead-ends gracefully with no cash account", async () => {
    const IRA = account({
      id: "0190a000-0000-7000-8000-000000000005",
      name: "IRA",
      cashflow_role: "investment_asset",
      balance: { minor_units: 500_000, currency: "USD" },
    });
    mocks.accountList.mockResolvedValue(ok([BROKERAGE, IRA])); // two investments, no cash
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transfer/i }));
    // Recurring needs a cash source, so it's disabled…
    expect(screen.getByRole("button", { name: "Recurring" })).toBeDisabled();
    // …and an investment source has nowhere to go (no cash), so submit is blocked with a hint.
    expect(await screen.findByText(/no account this one can move money to/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add transfer" })).toBeDisabled();
  });

  it("flags an unreviewed transaction on the list (4d8.7)", async () => {
    mockTransactionPage([
      txn({ memo: "Imported", reviewed: false }),
      txn({
        transaction_id: "0190b000-0000-7000-8000-000000000002",
        memo: "Manual",
        reviewed: true,
      }),
    ]);
    renderWithClient(<TransactionsView />);
    await screen.findByText("Imported");
    // Only the unreviewed row carries the indicator.
    expect(screen.getAllByTitle("Unreviewed")).toHaveLength(1);
  });

  it("reveals the bulk-action bar when transactions are selected (j0cg.4)", async () => {
    mockTransactionPage([
      txn({ memo: "Imported" }),
      txn({ transaction_id: "0190b000-0000-7000-8000-000000000002", memo: "Coffee" }),
    ]);
    renderWithClient(<TransactionsView />);
    await screen.findByText("Imported");

    const checkboxes = screen.getAllByRole("checkbox", { name: /select/i });
    fireEvent.click(checkboxes[0]!);

    expect(screen.getByText(/1 selected/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /mark reviewed/i })).toBeInTheDocument();
    // Select-all offers the full count.
    expect(screen.getByRole("button", { name: /select all 2/i })).toBeInTheDocument();
  });

  it("shows a transaction's tag chips on the list (4d8.15)", async () => {
    mocks.tagList.mockResolvedValue(
      ok([
        {
          id: "0190d000-0000-7000-8000-0000000000d1",
          name: "Vacation",
          color: null,
          archived: false,
        },
      ]),
    );
    mockTransactionPage([txn({ tag_ids: ["0190d000-0000-7000-8000-0000000000d1"] })]);
    renderWithClient(<TransactionsView />);
    expect(await screen.findByText("Vacation")).toBeInTheDocument();
  });

  it("expands a split transaction inline to show its lines (4d8.19)", async () => {
    mockTransactionPage([
      txn({
        memo: "Target",
        amount: { minor_units: -20000, currency: "USD" },
        split_count: 2,
      }),
    ]);
    mocks.transactionSplits.mockResolvedValue(
      ok([
        {
          id: "0190f000-0000-7000-8000-000000000001",
          amount: { minor_units: -12000, currency: "USD" },
          category_id: null,
          note: null,
          tag_ids: [],
          sort_order: 0,
        },
        {
          id: "0190f000-0000-7000-8000-000000000002",
          amount: { minor_units: -8000, currency: "USD" },
          category_id: null,
          note: null,
          tag_ids: [],
          sort_order: 1,
        },
      ]),
    );
    renderWithClient(<TransactionsView />);
    await screen.findByText("Target");

    // Expand the split row → the lines load inline (lazily fetched on expand).
    fireEvent.click(screen.getByRole("button", { name: /show split lines/i }));
    expect(await screen.findByText("-$120.00")).toBeInTheDocument();
    expect(screen.getByText("-$80.00")).toBeInTheDocument();
    expect(mocks.transactionSplits).toHaveBeenCalledWith(
      "0190b000-0000-7000-8000-000000000001",
    );
  });

  it("renders a real columnar header (4d8.27.8.3)", async () => {
    mockTransactionPage([txn()]);
    renderWithClient(<TransactionsView />);
    await screen.findByRole("table");
    // Ordered Activity-then-Date to match the Projected Activity table. The two blank
    // headers are the select and expand columns.
    expect(
      screen.getAllByRole("columnheader").map((h) => h.textContent),
    ).toEqual([
      "",
      "Activity",
      "Date",
      "Accounts",
      "Category",
      "Tags",
      "Amount",
      "Balance after",
      "",
    ]);
  });

  it("shows a transfer as from -> to on BOTH legs (4d8.27.8.3)", async () => {
    // The same movement appears once per account with opposite signs; both rows must
    // read the same direction, or the list would contradict itself. No fixture in the
    // repo carried a counter account before this, so this branch was never executed.
    mockTransactionPage([
      txn({
        transaction_id: "t-out",
        account_name: "Checking",
        counter_account_id: "acc-savings",
        counter_account_name: "Savings",
        amount: { minor_units: -30_000, currency: "USD" },
      }),
      txn({
        transaction_id: "t-in",
        account_name: "Savings",
        counter_account_id: "acc-checking",
        counter_account_name: "Checking",
        amount: { minor_units: 30_000, currency: "USD" },
      }),
    ]);
    renderWithClient(<TransactionsView />);
    // The table renders immediately (in its loading state), so wait for a ROW.
    await screen.findByText(/-\$?300/);

    const rowFor = (amount: RegExp) =>
      screen.getByText(amount).closest("tr")!;
    for (const amount of [/-\$?300/, /\+\$?300/]) {
      const cells = within(rowFor(amount)).getAllByRole("cell");
      // The Accounts cell is the 4th (select, activity, date, accounts, …).
      expect(cells[3]).toHaveTextContent("Checking");
      expect(cells[3]).toHaveTextContent("Savings");
      // Money left Checking, so Checking reads first on both legs.
      expect(cells[3]!.textContent!.indexOf("Checking")).toBeLessThan(
        cells[3]!.textContent!.indexOf("Savings"),
      );
    }
  });

  it("leaves the accounts column single-sided for an ordinary expense", async () => {
    mockTransactionPage([txn({ account_name: "Checking" })]);
    renderWithClient(<TransactionsView />);
    // The scope strip totals the same figure, so take the occurrence that IS in a row.
    const row = (await screen.findByText(
      (_, el) =>
        el?.tagName === "TD" && /-?\$?40/.test(el.textContent ?? ""),
    )).closest("tr")!;
    const cells = within(row).getAllByRole("cell");
    expect(cells[3]).toHaveTextContent("Checking");
    // No arrow, because there is no second account of the user's.
    expect(cells[3]!.querySelector("svg")).toBeNull();
  });

  it("shows the shared table's skeletons while loading, not a separate spinner", async () => {
    // The point of the primitive is that these stop being re-invented per screen, so the
    // migration has to actually USE them — otherwise the four-state code has no
    // production consumer at all (ADR 0053 §1).
    mocks.transactionPage.mockImplementation(
      () => new Promise(() => undefined), // never resolves: stay loading
    );
    renderWithClient(<TransactionsView />);
    await screen.findByRole("table");
    expect(document.querySelectorAll(".animate-pulse").length).toBeGreaterThan(0);
    expect(screen.queryByText(/Loading transactions/)).not.toBeInTheDocument();
  });

  it("shows the shared empty state when filters match nothing", async () => {
    // `total` stays non-zero so this is the "no matches" case rather than the first-run
    // invitation — and it must go INSIDE the ok() envelope.
    mocks.transactionPage.mockImplementation(() =>
      Promise.resolve(ok({ rows: [], total: 3 })),
    );
    renderWithClient(<TransactionsView />);
    expect(await screen.findByText("No transactions match these filters")).toBeInTheDocument();
  });
});

describe("spend-by-category breakdown (personal-cfo-4d8.27.8.4, ADR 0052)", () => {
  const spend = (
    over: Partial<{
      category_id: string;
      name: string;
      total_minor: number;
      has_children: boolean;
    }> = {},
  ) => ({
    category_id: "cat-food",
    name: "Food",
    parent_id: null,
    total_minor: 84_200,
    own_minor: 0,
    has_children: true,
    ...over,
  });

  beforeEach(() => {
    // A vault with no transactions replaces the whole surface — chart included — with
    // "No transactions yet", so these tests need rows for the chart to exist at all.
    mockTransactionPage([txn()]);
  });

  it("states the range it is charting, since the list has no date bound by default", async () => {
    // ADR 0052 §4. Unbounded, this would silently total ALL history on first paint and
    // the user would have no way to know what period they were reading.
    mocks.spendByCategory.mockResolvedValue(ok(breakdown([spend()])));
    renderWithClient(<TransactionsView />);
    expect(await screen.findByText(/the last 90 days, through/i)).toBeInTheDocument();
    // …and which currency, because the aggregate is scoped to one (§4).
    expect(screen.getByText(/amounts in USD/i)).toBeInTheDocument();
  });

  it("sends the list's facets to the aggregate, but never its category", async () => {
    // §2's one-shared-query promise. The category is the chart's DRILL LEVEL
    // (parent_id), so sending it as a filter too would constrain the same thing twice
    // and collapse the breakdown to one bar.
    mocks.spendByCategory.mockResolvedValue(ok(breakdown([spend()])));
    renderWithClient(<TransactionsView />);
    await screen.findByText("Food");
    expect(mocks.spendByCategory).toHaveBeenCalledWith(
      expect.objectContaining({
        parent_id: null,
        account_ids: [],
        tag_id: null,
        query: null,
        unreviewed_only: false,
      }),
    );
    // The input has no category field at all — the drill level is parent_id.
    expect(mocks.spendByCategory.mock.calls[0]![0]).not.toHaveProperty("category_id");
  });

  it("drills the chart and narrows the list with one click, and backs out of both", async () => {
    mocks.spendByCategory.mockResolvedValue(ok(breakdown([spend()])));
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /Food, \$842\.00/ }));

    // The chart drills to Food's children…
    await waitFor(() =>
      expect(mocks.spendByCategory).toHaveBeenCalledWith(
        expect.objectContaining({ parent_id: "cat-food" }),
      ),
    );
    // …and the LIST is filtered to the same category, so the rows below are the ones the
    // bar counted rather than a disconnected panel.
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenCalledWith(
        expect.objectContaining({ category_id: "cat-food" }),
      ),
    );
    expect(screen.getByText("Spending in Food")).toBeInTheDocument();

    // Backing out returns both views together.
    fireEvent.click(screen.getByRole("button", { name: /all categories/i }));
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenCalledWith(
        expect.objectContaining({ category_id: null }),
      ),
    );
    expect(screen.getByText("Where the money went")).toBeInTheDocument();
  });

  it("drops zero-spend categories rather than drawing empty bars", async () => {
    mocks.spendByCategory.mockResolvedValue(
      ok(breakdown([spend(), spend({ category_id: "cat-none", name: "Unused", total_minor: 0 })])),
    );
    renderWithClient(<TransactionsView />);
    await screen.findByText("Food");
    expect(screen.queryByText("Unused")).not.toBeInTheDocument();
  });
});

describe("spend chart and list never describe different sets (ADR 0052 §2)", () => {
  beforeEach(() => {
    mockTransactionPage([txn()]);
    mocks.spendByCategory.mockResolvedValue(
      ok(breakdown([
        {
          category_id: CATEGORY.id,
          name: CATEGORY.name,
          parent_id: null,
          total_minor: 12_300,
          own_minor: 12_300,
          has_children: false,
        },
      ])),
    );
  });

  it("drills the chart when the FILTER BAR picks a category, not just when a bar is clicked", async () => {
    // Clearing the trail here (the first version of this) left the chart showing every
    // root while the list showed one category — the two describing different sets.
    renderWithClient(<TransactionsView />);
    await screen.findByText(CATEGORY.name);

    // The facets live behind the Filters disclosure.
    fireEvent.click(screen.getByRole("button", { name: /^filters$/i }));
    // Rows carry their own inline "Category" picker, so scope to the FILTER panel's —
    // it renders above the table.
    const [categoryFacet] = await screen.findAllByRole("combobox", { name: "Category" });
    fireEvent.change(categoryFacet!, { target: { value: CATEGORY.id } });

    await waitFor(() =>
      expect(mocks.spendByCategory).toHaveBeenCalledWith(
        expect.objectContaining({ parent_id: CATEGORY.id }),
      ),
    );
    expect(screen.getByText(`Spending in ${CATEGORY.name}`)).toBeInTheDocument();
  });

  it("says it has nothing to show for the uncategorized sentinel", async () => {
    // "Uncategorized" is a list sentinel, not a category, and the aggregate counts only
    // categorized expenses — so a full breakdown here would describe a different set
    // than the rows below it.
    renderWithClient(<TransactionsView />);
    await screen.findByText(CATEGORY.name);

    fireEvent.click(screen.getByRole("button", { name: /^filters$/i }));
    const [categoryFacet] = await screen.findAllByRole("combobox", { name: "Category" });
    fireEvent.change(categoryFacet!, { target: { value: "uncategorized" } });

    expect(
      await screen.findByText(/uncategorized transactions have no category breakdown/i),
    ).toBeInTheDocument();
  });
});

describe("the scope strip is the one place scope is stated (personal-cfo-3tbn)", () => {
  it("drilling a chart bar writes a chip, and clearing the chip undoes the drill", async () => {
    // The AC's assertion in BOTH directions. The chart drill and the category facet are one
    // piece of state; if the chip and the trail could disagree, the reader would be
    // reconciling one thing with itself — the disagreement ADR 0052 §2 exists to prevent.
    mocks.categoryList.mockResolvedValue(
      ok([
        { id: "cat-food", name: "Food", parent_id: null, is_system: false, archived: false, color: null, icon: null },
      ]),
    );
    mocks.spendByCategory.mockResolvedValue(
      ok(breakdown([
        {
          category_id: "cat-food",
          name: "Food",
          total_minor: 40_000,
          currency: "USD",
          // Only a category WITH children is drillable — a leaf has nothing to open into.
          has_children: true,
        },
      ])),
    );
    mockTransactionPage([txn({ transaction_id: "t1", memo: "Lunch" })]);
    renderWithClient(<TransactionsView />);

    // Drill the bar.
    fireEvent.click(await screen.findByRole("button", { name: /Food/ }));

    // The scope strip now carries it, marked as coming from the chart.
    const chip = await screen.findByRole("button", { name: /Remove Food/ });
    expect(chip).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenLastCalledWith(
        expect.objectContaining({ category_id: "cat-food" }),
      ),
    );

    // Clearing the chip pops the drill: the list goes back to every category.
    fireEvent.click(chip);
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenLastCalledWith(
        expect.objectContaining({ category_id: null }),
      ),
    );
  });
});

describe("the four states say which thing happened (personal-cfo-xu32)", () => {
  it("stops pretending to load when the page read fails", async () => {
    // The defect this fixes: `total` stays undefined on error, so the table skeletonised
    // forever — the screen showed an error message AND an endless loading list, which
    // reads as "still trying".
    mocks.transactionPage.mockResolvedValue({
      status: "error",
      error: { kind: "Internal", message: "disk gone" },
    });
    renderWithClient(<TransactionsView />);

    const alerts = await screen.findAllByRole("alert");
    expect(
      alerts.some((a) => /couldn’t read this page of transactions/i.test(a.textContent ?? "")),
    ).toBe(true);
    // …and it says the data itself is fine.
    expect(
      alerts.some((a) => /your data is fine/i.test(a.textContent ?? "")),
    ).toBe(true);
  });

  it("distinguishes an aggregate failure from a vault failure", async () => {
    // The chart is a SEPARATE read from the list, so it can fail while the vault is
    // perfectly readable. On a local-first finance app, "your data might be damaged" and
    // "one computation failed" call for completely different reactions.
    mockTransactionPage([txn({ transaction_id: "t1", memo: "Lunch" })]);
    mocks.spendByCategory.mockResolvedValue({
      status: "error",
      error: { kind: "Internal", message: "aggregate boom" },
    });
    renderWithClient(<TransactionsView />);

    const alert = await screen.findByText(/the vault is readable/i);
    expect(alert).toBeInTheDocument();
    // The list is unaffected and still renders.
    expect(await screen.findByText("Lunch")).toBeInTheDocument();
  });

  it("offers the path that actually fills an empty vault", async () => {
    // Importing is how a vault gets populated; adding by hand is the fallback, so it reads
    // as the secondary action rather than the only one.
    mockTransactionPage([]);
    renderWithClient(<TransactionsView />);

    expect(await screen.findByText("No transactions yet")).toBeInTheDocument();
    expect(screen.getByText(/import a csv or ofx file/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add one manually" })).toBeInTheDocument();
  });

  it("keeps empty and no-match from being interchangeable", async () => {
    // "You have no transactions" and "your filters excluded them all" are different facts,
    // and only one of them means something is wrong with the filters.
    mockTransactionPage([txn({ transaction_id: "t1", memo: "Lunch" })]);
    renderWithClient(<TransactionsView />);
    await screen.findByText("Lunch");

    fireEvent.change(screen.getByPlaceholderText(/search/i), {
      target: { value: "nothing matches this" },
    });
    expect(
      await screen.findByText("No transactions match these filters"),
    ).toBeInTheDocument();
    expect(screen.queryByText("No transactions yet")).not.toBeInTheDocument();
  });
});

describe("the chart says what it excluded (personal-cfo-90eg)", () => {
  const spendRow = () => ({
    category_id: "cat-food",
    name: "Food",
    parent_id: null,
    total_minor: 84_200,
    own_minor: 0,
    has_children: false,
  });

  it("names the uncategorized and self-transfer amounts it could not place", async () => {
    // The chart total and the list total legitimately differ — the chart is expenses-only
    // and cannot place an uncategorized row or money moved between the household's own
    // accounts. Without saying so, the reader has no way to tell a rule from a bug.
    mockTransactionPage([txn({ transaction_id: "t1", memo: "Lunch" })]);
    mocks.spendByCategory.mockResolvedValue(
      ok(
        breakdown([spendRow()], {
          uncategorized_minor: 34_000,
          transfers_minor: 120_000,
        }),
      ),
    );
    renderWithClient(<TransactionsView />);

    const note = await screen.findByText(/expenses only\. excludes/i);
    expect(note).toHaveTextContent("$340.00 uncategorized");
    expect(note).toHaveTextContent("$1,200.00 moved between your own accounts");
  });

  it("says only 'Expenses only.' when nothing was excluded", async () => {
    // Naming a $0 exclusion would be noise dressed as precision.
    mockTransactionPage([txn({ transaction_id: "t1", memo: "Lunch" })]);
    mocks.spendByCategory.mockResolvedValue(ok(breakdown([spendRow()])));
    renderWithClient(<TransactionsView />);

    expect(await screen.findByText("Expenses only.")).toBeInTheDocument();
  });
});

describe("Balance after is the account's own running balance (personal-cfo-ttuy)", () => {
  it("asks for balances in date order and shows them", async () => {
    mockTransactionPage([
      txn({ transaction_id: "t1", memo: "Lunch", balance_after_minor: 250_000 }),
    ]);
    renderWithClient(<TransactionsView />);

    await screen.findByText("Lunch");
    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenLastCalledWith(
        expect.objectContaining({ with_balances: true }),
      ),
    );
    expect(screen.getByText("$2,500.00")).toBeInTheDocument();
  });

  it("goes blank when sorted by amount, and stops asking for the figure", async () => {
    // A running balance is only true in DATE order. Sorted by amount each row's figure
    // would still be individually correct while the column read as a sequence of balances
    // that never happened in that sequence — a confidently wrong number on every row.
    mockTransactionPage([
      txn({ transaction_id: "t1", memo: "Lunch", balance_after_minor: 250_000 }),
    ]);
    renderWithClient(<TransactionsView />);
    await screen.findByText("Lunch");

    fireEvent.change(screen.getByLabelText(/sort/i), {
      target: { value: "amount_desc" },
    });

    await waitFor(() =>
      expect(mocks.transactionPage).toHaveBeenLastCalledWith(
        expect.objectContaining({ sort: "amount_desc", with_balances: false }),
      ),
    );
  });
});
