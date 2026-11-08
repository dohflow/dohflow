import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  AccountViewDto,
  CategoryDto,
  CreateRecurringBillInput,
  TransactionPageInput,
  TransactionRowDto,
} from "@/bindings";
import { COMPACT_QUERY } from "@/lib/compact";
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
  checkLoanOverlap: vi.fn(),
  linkBillToLoan: vi.fn(),
  setBillNotLoanPayment: vi.fn(),
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
    checkLoanOverlap: mocks.checkLoanOverlap,
    linkBillToLoan: mocks.linkBillToLoan,
    setBillNotLoanPayment: mocks.setBillNotLoanPayment,
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
  mocks.checkLoanOverlap.mockReset();
  mocks.checkLoanOverlap.mockResolvedValue(ok(null));
  mocks.linkBillToLoan.mockReset();
  mocks.linkBillToLoan.mockResolvedValue(mutationOk());
  mocks.setBillNotLoanPayment.mockReset();
  mocks.setBillNotLoanPayment.mockResolvedValue(mutationOk());
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
  // personal-cfo-i93n3.28: Activity on a phone is a list of cards of labeled values, not a horizontal table.
  describe("on a phone", () => {
    afterEach(() => vi.unstubAllGlobals());
    function compactScreen() {
      vi.stubGlobal("matchMedia", ((query: string) => ({
        matches: query === COMPACT_QUERY,
        media: query,
        addEventListener: () => undefined,
        removeEventListener: () => undefined,
      })) as unknown as typeof window.matchMedia);
    }

    it("shows each transaction as a card: the select control and the title in the heading row, labeled values below", async () => {
      compactScreen();
      mockTransactionPage([txn({ amount: { minor_units: 15_000, currency: "USD" } })]);
      renderWithClient(<TransactionsView />);
      const amount = await screen.findByText(/\+\$?150/);
      expect(screen.queryByRole("table")).toBeNull();
      const card = amount.closest("li")!;
      // The checkbox (a control column) heads the card with the title: no "select" label, no lowercase key as a label.
      expect(within(card).getByRole("checkbox", { name: /^Select/ })).toBeInTheDocument();
      const labels = [...card.querySelectorAll("dt")].map((d) => d.textContent);
      expect(labels).toEqual(expect.arrayContaining(["Date", "Accounts", "Category", "Amount", "Balance after"]));
      expect(labels).not.toContain("select");
      expect(labels).not.toContain("expand");
      expect(labels.indexOf("Date")).toBeLessThan(labels.indexOf("Amount"));
      expect(within(card).getByText("Checking")).toBeInTheDocument();
    });

    it("hides the Tags pair for a transaction with no tags, so no bare label is read (and shows it when there are tags)", async () => {
      compactScreen();
      mockTransactionPage([txn({ amount: { minor_units: 15_000, currency: "USD" }, tag_ids: [] })]);
      renderWithClient(<TransactionsView />);
      const card = (await screen.findByText(/\+\$?150/)).closest("li")!;
      expect([...card.querySelectorAll("dt")].map((d) => d.textContent)).not.toContain("Tags");
    });

    it("wraps the action buttons and the search, sort and filter controls on a phone (compact-only classes)", async () => {
      compactScreen();
      mockTransactionPage([txn({ amount: { minor_units: 15_000, currency: "USD" } })]);
      renderWithClient(<TransactionsView />);
      await screen.findByText(/\+\$?150/);
      expect(screen.getByRole("button", { name: /Add transaction/ }).parentElement).toHaveClass("compact:flex-wrap");
      // At the largest text size a button's own label wraps instead of running off the screen.
      expect(screen.getByRole("button", { name: /Add transaction/ })).toHaveClass("compact:whitespace-normal", "compact:max-w-full");
      expect(screen.getByText("Checking")).toHaveClass("compact:whitespace-normal");
      // The category picker may not be wider than the card.
      expect(screen.getByRole("combobox", { name: "Category" })).toHaveClass("compact:w-full", "compact:max-w-full");
      const search = screen.getByRole("searchbox", { name: "Search transactions" });
      expect(search.parentElement).toHaveClass("compact:basis-full");
      expect(search.parentElement!.parentElement).toHaveClass("compact:flex-wrap");
      expect(screen.getByRole("combobox", { name: "Sort transactions" }).parentElement).toHaveClass("compact:flex-1");
    });

    it("the Add transaction form fits as many columns as the text size allows and wraps its action row (compact-only)", async () => {
      compactScreen();
      mockTransactionPage([txn({ amount: { minor_units: 15_000, currency: "USD" } })]);
      renderWithClient(<TransactionsView />);
      await screen.findByText(/\+\$?150/);
      fireEvent.click(screen.getByRole("button", { name: /Add transaction/ }));
      const amount = await screen.findByLabelText("Amount");
      const grid = amount.closest("div.grid")!;
      expect(grid.className).toContain("compact:grid-cols-[repeat(auto-fit,minmax(min(100%,9rem),1fr))]");
      expect(grid.className).toContain("grid-cols-2");
      // Its cells may shrink below their content (a date field is wide at the largest text size), and the date field's text is capped.
      expect(grid.className).toContain("compact:[&>*]:min-w-0");
      // (iOS WebKit gives a native date field an intrinsic minimum width that `min-width` cannot lower, which measured at 328 pt
      // in a 230 pt column at the largest text size; without its native appearance it takes the column's width and still opens
      // the system date picker.)
      expect(screen.getByLabelText("Date")).toHaveClass("compact:max-w-full", "compact:appearance-none");
      const submit = screen.getByRole("button", { name: "Add transaction", description: "" , hidden: false });
      expect(submit.closest("form")!.querySelector("div.justify-end")).toHaveClass("compact:flex-wrap");
    });

    it("keeps the horizontal table on a desktop window", async () => {
      mockTransactionPage([txn({ amount: { minor_units: 15_000, currency: "USD" } })]);
      renderWithClient(<TransactionsView />);
      await screen.findByText(/\+\$?150/);
      expect(screen.getByRole("table")).toBeInTheDocument();
    });
  });

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

  it("shows the schema's amount message when an empty transaction form is submitted, and records nothing (zod resolver)", async () => {
    mockTransactionPage([]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(
      await screen.findByRole("button", { name: /add transaction/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    expect(await screen.findByText("Enter an amount above zero.")).toBeInTheDocument();
    expect(screen.getByLabelText(/amount/i)).toHaveAttribute("aria-invalid", "true");
    expect(mocks.recordTransaction).not.toHaveBeenCalled();
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

  describe("recurring at add time (personal-cfo-6wk.23)", () => {
    const LOAN = {
      loan_account_id: "loan-1",
      loan_name: "Test Mortgage",
      strength: "strong",
      matched_by: "loan_typed",
    };

    async function fillRecurring(amount = "1500") {
      mockTransactionPage([]);
      renderWithClient(<TransactionsView />);
      fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
      fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
      fireEvent.change(screen.getByLabelText(/bill name/i), { target: { value: "Mortgage" } });
      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: amount } });
      fireEvent.change(screen.getByLabelText(/first due date/i), { target: { value: "2026-07-05" } });
    }
    const create = () => fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
    const keyOf = (call: number) => mocks.createRecurringBill.mock.calls[call]?.[0].idempotency_key;

    it("runs the loan-overlap check on exactly what would be created, and creates when nothing matches", async () => {
      await fillRecurring();
      create();
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
      expect(mocks.checkLoanOverlap).toHaveBeenCalledTimes(1);
      expect(mocks.checkLoanOverlap.mock.calls[0]?.[0]).toMatchObject({
        name: "Mortgage",
        frequency: "monthly",
        anchor_date: "2026-07-05",
        autopay_account_id: account().id,
        source_merchant_key: null,
      });
      expect(mocks.checkLoanOverlap.mock.calls[0]?.[0].amount.minor_units).toBe(150_000);
    });

    it("a payment into a loan shows the same warning and creates nothing until the person decides", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
      await fillRecurring();
      create();
      expect(
        await screen.findByText("This is Test Mortgage's payment — don't add a second one."),
      ).toBeInTheDocument();
      expect(mocks.createRecurringBill).not.toHaveBeenCalled();
      fireEvent.click(screen.getByRole("button", { name: "Add anyway" }));
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
      expect(mocks.linkBillToLoan).not.toHaveBeenCalled();
    });

    it("Add and link creates the bill and links it to the loan", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
      mocks.createRecurringBill.mockResolvedValue(ok({ op_seq: 1, replayed: false, event_id: "bill-new" }));
      await fillRecurring();
      create();
      fireEvent.click(await screen.findByRole("button", { name: "Add and link to Test Mortgage" }));
      await waitFor(() =>
        expect(mocks.linkBillToLoan).toHaveBeenCalledWith("bill-new", "loan-1", expect.any(String)),
      );
    });

    it("Don’t add closes the form and creates nothing", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
      await fillRecurring();
      create();
      fireEvent.click(await screen.findByRole("button", { name: "Don't add" }));
      expect(screen.queryByLabelText(/bill name/i)).not.toBeInTheDocument();
      expect(mocks.createRecurringBill).not.toHaveBeenCalled();
    });

    it("a declared-terms match asks, naming the paying account", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(
        ok({ ...LOAN, strength: "possible", matched_by: "declared_terms" }),
      );
      mocks.createRecurringBill.mockResolvedValue(ok({ op_seq: 1, replayed: false, event_id: "bill-new" }));
      await fillRecurring();
      create();
      expect(await screen.findByText("Is this Test Mortgage's payment?")).toBeInTheDocument();
      expect(screen.getByText(/It's paid from Checking around Test Mortgage's due date/)).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "No, it's a different bill" }));
      await waitFor(() =>
        expect(mocks.setBillNotLoanPayment).toHaveBeenCalledWith(expect.any(String), "loan-1", expect.any(String)),
      );
    });

    it.each([
      ["an error result", () => mocks.checkLoanOverlap.mockResolvedValue(err("check broke"))],
      ["a throw", () => mocks.checkLoanOverlap.mockRejectedValue(new Error("check broke"))],
    ])("fails open: %s from the check still saves the bill", async (_name, fail) => {
      fail();
      await fillRecurring();
      create();
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
      expect(screen.queryByText(/don't add a second one/)).not.toBeInTheDocument();
    });

    it("a first submit that threw says it could not confirm, and submitting again sends the same key and creates one bill", async () => {
      mocks.createRecurringBill.mockRejectedValueOnce(new Error("connection lost"));
      await fillRecurring();
      create();
      expect(
        await screen.findByText(
          "Couldn’t confirm whether the recurring bill was created. Submit again and it won’t be added twice.",
        ),
      ).toBeInTheDocument();
      // The form stays open, with the person's input.
      expect(screen.getByLabelText(/bill name/i)).toHaveValue("Mortgage");
      create();
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(2));
      expect(keyOf(1)).toBe(keyOf(0));
      await waitFor(() => expect(screen.queryByLabelText(/bill name/i)).not.toBeInTheDocument());
    });

    it("a typed refusal resubmits with the same key, and a changed amount is a new draft with a new key", async () => {
      mocks.createRecurringBill.mockResolvedValue({
        status: "error",
        error: { Validation: "Pick an account." },
      });
      await fillRecurring();
      create();
      expect(await screen.findByText("Pick an account.")).toBeInTheDocument();
      create();
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(2));
      expect(keyOf(1)).toBe(keyOf(0));
      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "1600" } });
      create();
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(3));
      expect(keyOf(2)).not.toBe(keyOf(0));
    });

    it("the key is the same when the bill is created from the warning after a throw", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
      mocks.createRecurringBill.mockRejectedValueOnce(new Error("connection lost"));
      await fillRecurring();
      create();
      fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
      await screen.findByText(/couldn’t confirm whether the recurring bill was created/i);
      create();
      fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(2));
      expect(keyOf(1)).toBe(keyOf(0));
    });

    it("a link that threw after the bill was created closes the form, says so, and never creates the bill again", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
      mocks.createRecurringBill.mockResolvedValue(ok({ op_seq: 1, replayed: false, event_id: "bill-new" }));
      mocks.linkBillToLoan.mockRejectedValue(new Error("connection lost"));
      await fillRecurring();
      create();
      fireEvent.click(await screen.findByRole("button", { name: "Add and link to Test Mortgage" }));
      expect(
        await screen.findByText(
          "The bill was added, but we couldn’t confirm that it was linked to Test Mortgage. Check the warning on the bill before trying again.",
        ),
      ).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByLabelText(/bill name/i)).not.toBeInTheDocument());
      expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);
    });

    it("an answer that threw after the bill was created closes the form, says so, and never creates the bill again", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(
        ok({ ...LOAN, strength: "possible", matched_by: "declared_terms" }),
      );
      mocks.createRecurringBill.mockResolvedValue(ok({ op_seq: 1, replayed: false, event_id: "bill-new" }));
      mocks.setBillNotLoanPayment.mockRejectedValue(new Error("connection lost"));
      await fillRecurring();
      create();
      fireEvent.click(await screen.findByRole("button", { name: "No, it's a different bill" }));
      expect(
        await screen.findByText(
          "The bill was added, but we couldn’t confirm that your answer was saved, so DohFlow may ask again.",
        ),
      ).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByLabelText(/bill name/i)).not.toBeInTheDocument());
      expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);
    });

    describe("an edit while the warning is open ends that confirmation", () => {
      const openDetails = () => {
        if (!screen.queryByLabelText("Note")) {
          fireEvent.click(screen.getByRole("button", { name: /add details/i }));
        }
      };
      const edits: [string, () => unknown, (bill: CreateRecurringBillInput) => unknown, unknown][] = [
        [
          "the amount",
          () => fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "1600" } }),
          (bill) => bill.amount.minor_units,
          160_000,
        ],
        [
          "the bill name",
          () => fireEvent.change(screen.getByLabelText(/bill name/i), { target: { value: "Home loan" } }),
          (bill) => bill.name,
          "Home loan",
        ],
        [
          "the first due date",
          () =>
            fireEvent.change(screen.getByLabelText(/first due date/i), { target: { value: "2026-08-05" } }),
          (bill) => bill.anchor_date,
          "2026-08-05",
        ],
        [
          "the frequency",
          () => fireEvent.change(screen.getByLabelText(/frequency/i), { target: { value: "weekly" } }),
          (bill) => bill.frequency,
          "weekly",
        ],
        [
          "the note",
          () => {
            openDetails();
            fireEvent.change(screen.getByLabelText("Note"), { target: { value: "Home loan" } });
          },
          (bill) => bill.description,
          "Home loan",
        ],
        [
          "the category",
          () => {
            openDetails();
            fireEvent.change(screen.getByLabelText("Category"), { target: { value: CATEGORY.id } });
          },
          (bill) => bill.category_id,
          CATEGORY.id,
        ],
        [
          "a tag",
          async () => {
            openDetails();
            const input = screen.getByLabelText("Add a tag");
            fireEvent.change(input, { target: { value: "Vacation" } });
            fireEvent.keyDown(input, { key: "Enter" });
            await waitFor(() => expect(mocks.createTag).toHaveBeenCalled());
          },
          (bill) => bill.tag_ids,
          [VACATION_TAG],
        ],
      ];

      it.each(edits)(
        "%s: the warning goes, nothing is created, and the next submit checks and creates the current draft with a new key",
        async (_field, edit, read, expected) => {
          mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
          mocks.createRecurringBill.mockRejectedValueOnce(new Error("connection lost"));
          await fillRecurring();
          // A first attempt that threw leaves the 1500 draft's key behind.
          create();
          fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
          await screen.findByText(/couldn’t confirm whether the recurring bill was created/i);
          expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);

          create();
          await screen.findByRole("button", { name: "Add anyway" });
          await edit();
          // The stale confirmation is gone: the form is back to its submit state.
          await waitFor(() => expect(screen.queryByRole("button", { name: "Add anyway" })).toBeNull());
          expect(screen.getByRole("button", { name: /create recurring bill/i })).toBeEnabled();
          expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);

          create();
          const checks = mocks.checkLoanOverlap.mock.calls;
          await waitFor(() => expect(checks.length).toBe(3));
          expect(read(checks[2]?.[0])).toEqual(expected);
          fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
          await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(2));
          expect(read(mocks.createRecurringBill.mock.calls[1]?.[0])).toEqual(expected);
          expect(keyOf(1)).not.toBe(keyOf(0));
        },
      );

      it("editing the account ends the confirmation, and the next create is for that account", async () => {
        mocks.accountList.mockResolvedValue(ok([account(), SAVINGS]));
        mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
        await fillRecurring();
        create();
        await screen.findByRole("button", { name: "Add anyway" });
        fireEvent.change(document.getElementById("txn-account") as HTMLElement, {
          target: { value: SAVINGS.id },
        });
        await waitFor(() => expect(screen.queryByRole("button", { name: "Add anyway" })).toBeNull());
        expect(mocks.createRecurringBill).not.toHaveBeenCalled();
        create();
        fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
        await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
        expect(mocks.createRecurringBill.mock.calls[0]?.[0].autopay_account_id).toBe(SAVINGS.id);
      });

      it.each([
        [
          "the custom interval count",
          () => fireEvent.change(screen.getByLabelText("Interval count"), { target: { value: "3" } }),
          "every_3_months",
        ],
        [
          "the custom interval unit",
          () => fireEvent.change(screen.getByLabelText("Interval unit"), { target: { value: "weeks" } }),
          "every_2_weeks",
        ],
      ])("editing %s ends the confirmation, and the next create carries it", async (_name, edit, token) => {
        mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
        await fillRecurring();
        fireEvent.change(screen.getByLabelText(/frequency/i), { target: { value: "custom" } });
        fireEvent.change(screen.getByLabelText("Interval count"), { target: { value: "2" } });
        fireEvent.change(screen.getByLabelText("Interval unit"), { target: { value: "months" } });
        create();
        await screen.findByRole("button", { name: "Add anyway" });
        edit();
        await waitFor(() => expect(screen.queryByRole("button", { name: "Add anyway" })).toBeNull());
        expect(mocks.createRecurringBill).not.toHaveBeenCalled();
        create();
        fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
        await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
        expect(mocks.createRecurringBill.mock.calls[0]?.[0].frequency).toBe(token);
      });

      it("an edit that is put back still ends the confirmation, and the unchanged draft keeps its key", async () => {
        mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
        mocks.createRecurringBill.mockRejectedValueOnce(new Error("connection lost"));
        await fillRecurring();
        create();
        fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
        await screen.findByText(/couldn’t confirm whether the recurring bill was created/i);

        create();
        await screen.findByRole("button", { name: "Add anyway" });
        fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "1600" } });
        await waitFor(() => expect(screen.queryByRole("button", { name: "Add anyway" })).toBeNull());
        fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "1500" } });
        expect(screen.queryByRole("button", { name: "Add anyway" })).toBeNull();

        create();
        fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
        await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(2));
        expect(keyOf(1)).toBe(keyOf(0));
      });

      it("turning recurring off ends the confirmation", async () => {
        mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
        await fillRecurring();
        create();
        await screen.findByRole("button", { name: "Add anyway" });
        fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
        await waitFor(() => expect(screen.queryByRole("button", { name: "Add anyway" })).toBeNull());
        expect(screen.getByRole("button", { name: /add transaction/i })).toBeInTheDocument();
        expect(mocks.createRecurringBill).not.toHaveBeenCalled();
      });
    });

    it("a link the app refused keeps its existing message", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(ok(LOAN));
      mocks.createRecurringBill.mockResolvedValue(ok({ op_seq: 1, replayed: false, event_id: "bill-new" }));
      mocks.linkBillToLoan.mockResolvedValue({ status: "error", error: { Validation: "nope" } });
      await fillRecurring();
      create();
      fireEvent.click(await screen.findByRole("button", { name: "Add and link to Test Mortgage" }));
      expect(
        await screen.findByText(/The bill was added, but linking it to Test Mortgage didn't work/),
      ).toBeInTheDocument();
    });
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

    // The row is saved (record called), the attach failed → a non-blocking warning that
    // says how many documents did not attach.
    expect(
      await screen.findByText(/1 attachment didn’t attach/i),
    ).toBeInTheDocument();
    expect(mocks.recordTransaction).toHaveBeenCalledTimes(1);
    expect(mocks.attachDocument).toHaveBeenCalledTimes(1);
    // The form closed (add-details button gone) — the row persisted despite the attach failure.
    expect(
      screen.queryByRole("button", { name: /add details/i }),
    ).not.toBeInTheDocument();
  });

  it("a file that cannot be read after the record committed is reported, and never records twice (g3m.35)", async () => {
    const unreadable = pickable("receipt.pdf", "application/pdf", [1]);
    unreadable.arrayBuffer = () => Promise.reject(new Error("file moved"));
    const other = pickable("scan.pdf", "application/pdf", [2]);
    mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
    fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "40" } });
    fireEvent.change(screen.getByLabelText(/date/i), { target: { value: "2026-06-05" } });
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    fireEvent.change(screen.getByLabelText(/attach documents/i), {
      target: { files: [unreadable, other] },
    });
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    // The unreadable one is known not to be attached; the other is still tried and attaches.
    expect(await screen.findByText(/1 attachment didn’t attach/i)).toBeInTheDocument();
    expect(mocks.recordTransaction).toHaveBeenCalledTimes(1);
    expect(mocks.attachDocument).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: /add details/i })).not.toBeInTheDocument();
  });

  describe("one idempotency key per draft (g3m.35)", () => {
    async function fillDraft(amount: string) {
      fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: amount } });
      fireEvent.change(screen.getByLabelText(/date/i), { target: { value: "2026-06-05" } });
    }
    const submit = () => fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));
    const keyOf = (call: number) => mocks.recordTransaction.mock.calls[call]?.[0].idempotency_key;

    it("submitting again after a call that threw sends the same key; a changed amount sends a new one", async () => {
      mocks.recordTransaction.mockRejectedValueOnce(new Error("connection lost"));
      mocks.recordTransaction.mockRejectedValueOnce(new Error("connection lost"));
      mockTransactionPage([txn()]);
      renderWithClient(<TransactionsView />);
      await fillDraft("40");
      submit();
      expect(await screen.findByText(/couldn’t confirm whether the transaction was recorded/i)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: /add details/i })).toBeInTheDocument();

      submit();
      await waitFor(() => expect(mocks.recordTransaction).toHaveBeenCalledTimes(2));
      expect(keyOf(1)).toBe(keyOf(0));

      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "55" } });
      submit();
      await waitFor(() => expect(mocks.recordTransaction).toHaveBeenCalledTimes(3));
      expect(keyOf(2)).not.toBe(keyOf(0));
    });

    it("a typed refusal resubmits with the same key too, since nothing was recorded under it", async () => {
      mocks.recordTransaction.mockResolvedValueOnce({
        status: "error",
        error: { Validation: "Pick an account." },
      });
      mockTransactionPage([txn()]);
      renderWithClient(<TransactionsView />);
      await fillDraft("40");
      submit();
      expect(await screen.findByText("Pick an account.")).toBeInTheDocument();
      submit();
      await waitFor(() => expect(mocks.recordTransaction).toHaveBeenCalledTimes(2));
      expect(keyOf(1)).toBe(keyOf(0));
    });
  });

  describe("a submit in flight owns the form (g3m.38)", () => {
    async function startPendingSubmit() {
      let settle: { resolve: (v: unknown) => void; reject: (e: unknown) => void } = {
        resolve: () => {},
        reject: () => {},
      };
      mocks.recordTransaction.mockReturnValue(
        new Promise((resolve, reject) => (settle = { resolve, reject })),
      );
      mockTransactionPage([txn()]);
      renderWithClient(<TransactionsView />);
      fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "40" } });
      fireEvent.change(screen.getByLabelText(/date/i), { target: { value: "2026-06-05" } });
      fireEvent.click(screen.getByRole("button", { name: /add details/i }));
      fireEvent.change(screen.getByLabelText("Note"), { target: { value: "Groceries run" } });
      fireEvent.change(screen.getByLabelText(/attach documents/i), {
        target: { files: [pickable("receipt.pdf", "application/pdf", [1, 2, 3])] },
      });
      await screen.findByText("receipt.pdf");
      const amount = screen.getByLabelText(/amount/i);
      fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));
      await waitFor(() => expect(mocks.recordTransaction).toHaveBeenCalledTimes(1));
      return { settle, amount };
    }

    it("locks the fields, the staged-file controls and both buttons until the outcome", async () => {
      const { settle } = await startPendingSubmit();
      await waitFor(() => expect(screen.getByLabelText(/amount/i)).toBeDisabled());
      for (const id of ["txn-account", "txn-date", "txn-note", "txn-category", "txn-tags"]) {
        expect(document.getElementById(id)).toBeDisabled();
      }
      expect(screen.getByLabelText(/attach documents/i)).toBeDisabled();
      expect(screen.getByRole("button", { name: /attach document/i })).toBeDisabled();
      expect(screen.getByRole("button", { name: /remove receipt\.pdf/i })).toBeDisabled();
      expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
      expect(screen.getByRole("button", { name: /adding…/i })).toBeDisabled();
      // The header's Add buttons stay hidden while the form is open.
      expect(screen.queryByRole("button", { name: /^add transfer$/i })).toBeNull();
      await act(async () => settle.resolve({ status: "error", error: { Validation: "No." } }));
    });

    it("Cancel and Escape during the submit leave the form mounted, and the outcome shows in it", async () => {
      const { settle, amount } = await startPendingSubmit();
      await waitFor(() => expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled());
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
      expect(screen.getByLabelText(/amount/i)).toBe(amount);

      await act(async () => settle.reject(new Error("connection lost")));
      expect(
        await screen.findByText(/couldn’t confirm whether the transaction was recorded/i),
      ).toBeInTheDocument();
      // The same form, with what the person entered, unlocked.
      expect(screen.getByLabelText(/amount/i)).toBe(amount);
      expect(amount).toHaveValue("40");
      expect(amount).toBeEnabled();
      expect(screen.getByLabelText("Note")).toHaveValue("Groceries run");
      expect(screen.getByText("receipt.pdf")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    });

    it("creating the bill from the loan warning locks the form the same way until the outcome", async () => {
      mocks.checkLoanOverlap.mockResolvedValue(
        ok({ loan_account_id: "loan-1", loan_name: "Test Mortgage", strength: "strong", matched_by: "loan_typed" }),
      );
      let finish: (value: unknown) => void = () => {};
      mocks.createRecurringBill.mockReturnValue(new Promise((resolve) => (finish = resolve)));
      mockTransactionPage([]);
      renderWithClient(<TransactionsView />);
      fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
      fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
      fireEvent.change(screen.getByLabelText(/bill name/i), { target: { value: "Mortgage" } });
      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "1500" } });
      fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
      fireEvent.click(await screen.findByRole("button", { name: "Add anyway" }));
      await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
      await waitFor(() => expect(screen.getByLabelText(/amount/i)).toBeDisabled());
      expect(screen.getByLabelText(/bill name/i)).toBeDisabled();
      expect(screen.getByRole("button", { name: /^don't add$/i })).toBeDisabled();
      await act(async () => finish({ status: "error", error: { Validation: "No." } }));
      await waitFor(() => expect(screen.getByLabelText(/amount/i)).toBeEnabled());
    });

    it("an unchanged resubmit after the unknown outcome reuses the key", async () => {
      const { settle } = await startPendingSubmit();
      await act(async () => settle.reject(new Error("connection lost")));
      await screen.findByText(/couldn’t confirm whether the transaction was recorded/i);
      mocks.recordTransaction.mockResolvedValue(ok({ op_seq: 1, replayed: true, transaction_id: NEW_TXN_ID }));
      fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));
      await waitFor(() => expect(mocks.recordTransaction).toHaveBeenCalledTimes(2));
      expect(mocks.recordTransaction.mock.calls[1]?.[0].idempotency_key).toBe(
        mocks.recordTransaction.mock.calls[0]?.[0].idempotency_key,
      );
    });
  });

  it.each([
    ["note", () => mocks.setNote.mockRejectedValue(new Error("connection lost"))],
    ["tags", () => mocks.setTags.mockRejectedValue(new Error("connection lost"))],
    ["category", () => mocks.recategorizeTransaction.mockRejectedValue(new Error("connection lost"))],
  ])("a %s follow-up that throws after the record committed is a partial result, not a failed save (g3m.35)", async (_name, reject) => {
    mocks.tagList.mockResolvedValue(
      ok([{ id: VACATION_TAG, name: "Vacation", color: null, archived: false }]),
    );
    reject();
    mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
      Promise.resolve(pageResult([], input)),
    );
    mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
    renderWithClient(<TransactionsView />);

    fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
    fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "40" } });
    fireEvent.change(screen.getByLabelText(/date/i), { target: { value: "2026-06-05" } });
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
    fireEvent.change(screen.getByLabelText("Category"), { target: { value: CATEGORY.id } });
    fireEvent.change(screen.getByLabelText(/add a tag/i), { target: { value: "Vacation" } });
    fireEvent.keyDown(screen.getByLabelText(/add a tag/i), { key: "Enter" });
    fireEvent.change(screen.getByLabelText("Note"), { target: { value: "Groceries run" } });
    fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));

    // The warning for a saved transaction, never the "couldn't confirm the record" message.
    expect(
      await screen.findByText(/some details .* may not have saved\. Open it to check before trying again/i),
    ).toBeInTheDocument();
    expect(screen.queryByText(/couldn’t confirm whether the transaction was recorded/i)).toBeNull();
    expect(mocks.recordTransaction).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: /add details/i })).not.toBeInTheDocument();
  });

  describe("which warning shows after a saved transaction (g3m.35)", () => {
    const DETAILS_UNKNOWN =
      "Transaction added, but some details (category, tags, note, or attachments) may not have saved. Open it to check before trying again.";
    const ATTACH_UNKNOWN =
      "Transaction added, but some attachments may not have attached. Open it to check before trying again.";

    async function addWith(files: File[], withNote = false) {
      mocks.transactionPage.mockImplementationOnce((input: TransactionPageInput) =>
        Promise.resolve(pageResult([], input)),
      );
      mockTransactionPage([txn({ transaction_id: NEW_TXN_ID })]);
      renderWithClient(<TransactionsView />);
      fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "40" } });
      fireEvent.change(screen.getByLabelText(/date/i), { target: { value: "2026-06-05" } });
      fireEvent.click(screen.getByRole("button", { name: /add details/i }));
      if (withNote) {
        fireEvent.change(screen.getByLabelText("Note"), { target: { value: "Groceries run" } });
      }
      if (files.length > 0) {
        fireEvent.change(screen.getByLabelText(/attach documents/i), { target: { files } });
      }
      fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));
    }

    it("(a) a thrown note call is unknown: the details sentence, not 'could not be saved'", async () => {
      mocks.setNote.mockRejectedValue(new Error("connection lost"));
      await addWith([], true);
      expect(await screen.findByText(DETAILS_UNKNOWN)).toBeInTheDocument();
      expect(screen.queryByText(/could not be saved/i)).not.toBeInTheDocument();
    });

    it("(a) a refused note beside a thrown attach is still the details sentence: unknown outranks", async () => {
      mocks.setNote.mockResolvedValue(err("note refused"));
      mocks.attachDocument.mockRejectedValue(new Error("connection lost"));
      await addWith([pickable("receipt.pdf", "application/pdf", [1])], true);
      expect(await screen.findByText(DETAILS_UNKNOWN)).toBeInTheDocument();
    });

    it("(b) a thrown attach call is unknown: it may or may not have attached, never 'didn’t attach'", async () => {
      mocks.attachDocument.mockRejectedValue(new Error("connection lost"));
      await addWith([pickable("receipt.pdf", "application/pdf", [1])]);
      expect(await screen.findByText(ATTACH_UNKNOWN)).toBeInTheDocument();
      expect(screen.queryByText(/didn’t attach/i)).not.toBeInTheDocument();
      expect(mocks.recordTransaction).toHaveBeenCalledTimes(1);
    });

    it("(b) one file confirmed unattached and one that threw: the unknown outranks the count", async () => {
      const unreadable = pickable("a.pdf", "application/pdf", [1]);
      unreadable.arrayBuffer = () => Promise.reject(new Error("file moved"));
      mocks.attachDocument.mockRejectedValue(new Error("connection lost"));
      await addWith([unreadable, pickable("b.pdf", "application/pdf", [2])]);
      expect(await screen.findByText(ATTACH_UNKNOWN)).toBeInTheDocument();
      expect(screen.queryByText(/didn’t attach/i)).not.toBeInTheDocument();
    });

    it("(c) a refused note with nothing unknown keeps the 'could not be saved' sentence", async () => {
      mocks.setNote.mockResolvedValue(err("note refused"));
      await addWith([], true);
      expect(
        await screen.findByText(/some details .* could not be saved\. Open it to try again/i),
      ).toBeInTheDocument();
    });

    it("(d) a file that fails before anything is sent, or is refused, is counted as 'didn’t attach'", async () => {
      const unreadable = pickable("a.pdf", "application/pdf", [1]);
      unreadable.arrayBuffer = () => Promise.reject(new Error("file moved"));
      mocks.attachDocument.mockResolvedValue(err("attach refused"));
      await addWith([unreadable, pickable("b.pdf", "application/pdf", [2])]);
      expect(await screen.findByText(/2 attachments didn’t attach/i)).toBeInTheDocument();
    });
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
        null, // a plain replace: the add form has no earlier set to protect
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

  describe("a transfer submit in flight owns the form (g3m.39)", () => {
    const UNKNOWN =
      "Couldn’t confirm whether the transfer was saved. Submit again and it won’t be added twice.";
    const transferKey = (call: number) =>
      mocks.recordTransfer.mock.calls[call]?.[0].idempotency_key;
    const recurringKey = (call: number) =>
      mocks.createRecurringTransfer.mock.calls[call]?.[0].idempotency_key;

    async function fillTransfer(amount = "50") {
      mocks.accountList.mockResolvedValue(ok([account(), SAVINGS]));
      renderWithClient(<TransactionsView />);
      fireEvent.click(await screen.findByRole("button", { name: /add transfer/i }));
      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: amount } });
    }
    const submit = () => fireEvent.click(screen.getByRole("button", { name: "Add transfer" }));

    it("locks the fields and both buttons while pending; Cancel does nothing; the outcome then shows in the same form", async () => {
      let settle: { reject: (e: unknown) => void } = { reject: () => {} };
      mocks.recordTransfer.mockReturnValue(new Promise((_, reject) => (settle = { reject })));
      await fillTransfer();
      const amount = screen.getByLabelText(/amount/i);
      submit();
      await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
      await waitFor(() => expect(amount).toBeDisabled());
      for (const id of ["transfer-source", "transfer-dest", "transfer-date"]) {
        expect(document.getElementById(id)).toBeDisabled();
      }
      expect(screen.getByRole("button", { name: "One-off" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "Recurring" })).toBeDisabled();
      expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
      expect(screen.getByRole("button", { name: /adding…/i })).toBeDisabled();
      // The header's Add buttons stay hidden while the form is open.
      expect(screen.queryByRole("button", { name: /^add transfer$/i })).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
      fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
      expect(screen.getByLabelText(/amount/i)).toBe(amount);

      await act(async () => settle.reject(new Error("connection lost")));
      expect(await screen.findByText(UNKNOWN)).toBeInTheDocument();
      // The same form, with what the person entered, unlocked.
      expect(screen.getByLabelText(/amount/i)).toBe(amount);
      expect(amount).toHaveValue("50");
      expect(amount).toBeEnabled();
      expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    });

    it("an unchanged resubmit after an unknown outcome reuses the key; a changed amount or date gets a new one", async () => {
      mocks.recordTransfer.mockRejectedValue(new Error("connection lost"));
      await fillTransfer();
      submit();
      await screen.findByText(UNKNOWN);
      submit();
      await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
      expect(transferKey(1)).toBe(transferKey(0));

      fireEvent.change(screen.getByLabelText(/amount/i), { target: { value: "60" } });
      submit();
      await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(3));
      expect(transferKey(2)).not.toBe(transferKey(1));

      fireEvent.change(document.getElementById("transfer-date") as HTMLElement, {
        target: { value: "2026-01-02" },
      });
      submit();
      await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(4));
      expect(transferKey(3)).not.toBe(transferKey(2));
    });

    it("a refusal shows the app's reason and the next try is a new draft with a new key", async () => {
      mocks.recordTransfer.mockResolvedValue({
        status: "error",
        error: { Validation: "That account cannot cover this." },
      });
      await fillTransfer();
      submit();
      expect(await screen.findByText("That account cannot cover this.")).toBeInTheDocument();
      expect(screen.queryByText(UNKNOWN)).toBeNull();
      submit();
      await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(2));
      expect(transferKey(1)).not.toBe(transferKey(0));
    });

    it("a recurring transfer that threw is an unknown outcome with the same key on retry; a changed schedule is a new draft", async () => {
      mocks.createRecurringTransfer.mockRejectedValue(new Error("connection lost"));
      await fillTransfer("200");
      fireEvent.click(screen.getByRole("button", { name: "Recurring" }));
      const schedule = () => fireEvent.click(screen.getByRole("button", { name: /schedule transfer/i }));
      schedule();
      await screen.findByText(UNKNOWN);
      schedule();
      await waitFor(() => expect(mocks.createRecurringTransfer).toHaveBeenCalledTimes(2));
      expect(recurringKey(1)).toBe(recurringKey(0));
      fireEvent.change(screen.getByLabelText(/frequency/i), { target: { value: "weekly" } });
      schedule();
      await waitFor(() => expect(mocks.createRecurringTransfer).toHaveBeenCalledTimes(3));
      expect(recurringKey(2)).not.toBe(recurringKey(1));
    });
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
  it("stops pretending to load when the page read fails, with one message that claims nothing about the data", async () => {
    // The defect this fixes: `total` stays undefined on error, so the table skeletonised
    // forever — the screen showed an error message AND an endless loading list, which
    // reads as "still trying".
    mocks.transactionPage.mockRejectedValue(new Error("host down"));
    renderWithClient(<TransactionsView />);

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/^Couldn’t read this page of transactions\.$/);
    // One message: not a banner and a table message.
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    // A failed read cannot say the data is fine.
    expect(screen.queryByText(/your data is fine/i)).toBeNull();
    expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  });

  it("adds the read's own reason when it gave one", async () => {
    mocks.transactionPage.mockResolvedValue({
      status: "error",
      error: { Validation: "That date range is not valid." },
    });
    renderWithClient(<TransactionsView />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn’t read this page of transactions. That date range is not valid.",
    );
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("Retry reads the page again, reads Retrying… with the message kept while it runs, and recovers", async () => {
    mocks.transactionPage.mockRejectedValueOnce(new Error("host down"));
    let finish: (value: unknown) => void = () => {};
    mocks.transactionPage.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve)),
    );
    renderWithClient(<TransactionsView />);
    await screen.findByRole("alert");
    const calls = mocks.transactionPage.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    const pending = await screen.findByRole("button", { name: "Retrying…" });
    expect(pending).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn’t read this page of transactions.");
    expect(mocks.transactionPage.mock.calls.length).toBe(calls + 1);

    await act(async () =>
      finish(pageResult([txn({ transaction_id: "t1", memo: "Lunch" })], mocks.transactionPage.mock.calls.at(-1)?.[0])),
    );
    expect(await screen.findByText("Lunch")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("button", { name: /retry/i })).toBeNull();
  });

  it("a second failure keeps the message and Retry", async () => {
    mocks.transactionPage.mockRejectedValue(new Error("host down"));
    renderWithClient(<TransactionsView />);
    await screen.findByRole("alert");
    const calls = mocks.transactionPage.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(mocks.transactionPage.mock.calls.length).toBe(calls + 1));
    expect(await screen.findByRole("button", { name: "Retry" })).toBeEnabled();
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("alert")).toHaveTextContent("Couldn’t read this page of transactions.");
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
