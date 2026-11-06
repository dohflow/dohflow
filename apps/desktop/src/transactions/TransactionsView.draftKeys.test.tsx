/**
 * personal-cfo-56w.8: the per-draft idempotency key of the Add transaction form and of a recurring bill
 * created from it (personal-cfo-g3m.35, 6wk.23), and of Add transfer, one-off and recurring
 * (personal-cfo-g3m.39), pinned for EVERY field of the draft's identity with the shared helper. Lives in its own file so it stays clear of the large TransactionsView suite.
 */

import { fireEvent, screen } from "@testing-library/react";

import { expectDraftKeyIdentity } from "@/test/idempotencyIdentity";
import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, CategoryDto } from "@/bindings";
import { TransactionsView } from "./TransactionsView";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  transactionPage: vi.fn(),
  recordTransaction: vi.fn(),
  recurringTransferList: vi.fn(),
  recurringBillList: vi.fn(),
  createRecurringBill: vi.fn(),
  checkLoanOverlap: vi.fn(),
  categoryList: vi.fn(),
  tagList: vi.fn(),
  transactionSplits: vi.fn(),
  spendByCategory: vi.fn(),
  baseCurrency: vi.fn(),
  createTag: vi.fn(),
  recordTransfer: vi.fn(),
  createRecurringTransfer: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    transactionPage: mocks.transactionPage,
    recordTransaction: mocks.recordTransaction,
    recurringTransferList: mocks.recurringTransferList,
    recurringBillList: mocks.recurringBillList,
    createRecurringBill: mocks.createRecurringBill,
    checkLoanOverlap: mocks.checkLoanOverlap,
    categoryList: mocks.categoryList,
    tagList: mocks.tagList,
    transactionSplits: mocks.transactionSplits,
    spendByCategory: mocks.spendByCategory,
    baseCurrency: mocks.baseCurrency,
    createTag: mocks.createTag,
    recordTransfer: mocks.recordTransfer,
    createRecurringTransfer: mocks.createRecurringTransfer,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const recordOk = () =>
  ok({ transaction_id: "0190b000-0000-7000-8000-0000000000ff", result: { op_seq: 1, replayed: false } });
const VACATION_TAG = "0190d000-0000-7000-8000-0000000000d1";

const account = (over: Partial<AccountViewDto> = {}): AccountViewDto => ({
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
});
const SAVINGS = account({ id: "0190a000-0000-7000-8000-000000000002", name: "Savings" });
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

beforeEach(() => {
  for (const fn of Object.values(mocks)) fn.mockReset();
  mocks.accountList.mockResolvedValue(ok([account(), SAVINGS]));
  mocks.transactionPage.mockResolvedValue(ok({ rows: [], total: 0 }));
  mocks.recurringTransferList.mockResolvedValue(ok([]));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.checkLoanOverlap.mockResolvedValue(ok(null));
  mocks.categoryList.mockResolvedValue(ok([CATEGORY]));
  mocks.tagList.mockResolvedValue(ok([]));
  mocks.transactionSplits.mockResolvedValue(ok([]));
  mocks.spendByCategory.mockResolvedValue(
    ok({ rows: [], uncategorized_minor: 0, transfers_minor: 0 }),
  );
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.createTag.mockResolvedValue(ok({ tag_id: VACATION_TAG }));
  // A lost connection: the call throws, so the form stays open and the key stays with the draft.
  mocks.recordTransaction.mockRejectedValue(new Error("connection lost"));
  mocks.createRecurringBill.mockRejectedValue(new Error("connection lost"));
  mocks.recordTransfer.mockRejectedValue(new Error("connection lost"));
  mocks.createRecurringTransfer.mockRejectedValue(new Error("connection lost"));
});

const change = (label: string | RegExp, value: string) =>
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
const byId = (id: string) => document.getElementById(id) as HTMLElement;
const openDetails = () => {
  if (!screen.queryByLabelText("Note")) {
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
  }
};

/// The recurring bill the key test sends: every field the form builds the bill from.
function fillBill() {
  fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
  change(/bill name/i, "Mortgage");
  change(/amount/i, "1500");
  change(/first due date/i, "2026-07-05");
  change(/frequency/i, "custom");
  change("Interval count", "2");
  change("Interval unit", "months");
}

async function openAddForm() {
  renderWithClient(<TransactionsView />);
  fireEvent.click(await screen.findByRole("button", { name: /add transaction/i }));
}

describe("the Add transaction key's identity (personal-cfo-56w.8)", () => {
  it("is the account, the signed amount and the date", async () => {
    await expectDraftKeyIdentity({
      name: "Add transaction",
      open: async () => {
        await openAddForm();
        change(/amount/i, "40");
        change(/date/i, "2026-06-05");
      },
      send: () => {
        fireEvent.click(screen.getByRole("button", { name: /add transaction/i }));
      },
      keys: () => mocks.recordTransaction.mock.calls.map((c) => c[0].idempotency_key),
      success: {
        arrange: () => mocks.recordTransaction.mockResolvedValueOnce(recordOk()),
        // A saved transaction closes the form: the same draft is typed into a freshly opened one.
        reopen: async () => {
          fireEvent.click(await screen.findByRole("button", { name: /^add transaction$/i }));
          change(/amount/i, "40");
          change(/date/i, "2026-06-05");
        },
      },
      fields: [
        { name: "account", change: () => fireEvent.change(byId("txn-account"), { target: { value: SAVINGS.id } }) },
        { name: "amount", change: () => change(/amount/i, "41") },
        { name: "date", change: () => change(/date/i, "2026-06-06") },
        { name: "kind (income instead of expense)", change: () => { fireEvent.click(screen.getByRole("button", { name: "Income" })); } },
      ],
    });
  });
});

describe("a recurring bill created from the Add form: key identity (personal-cfo-56w.8)", () => {
  it("is every field the bill is built from", async () => {
    await expectDraftKeyIdentity({
      name: "recurring at add time",
      open: async () => {
        await openAddForm();
        fillBill();
      },
      send: () => {
        fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
      },
      keys: () => mocks.createRecurringBill.mock.calls.map((c) => c[0].idempotency_key),
      success: {
        arrange: () => mocks.createRecurringBill.mockResolvedValueOnce(ok({ op_seq: 1, replayed: false })),
        // A created bill closes the form: the same bill is typed into a freshly opened one.
        reopen: async () => {
          fireEvent.click(await screen.findByRole("button", { name: /^add transaction$/i }));
          fillBill();
        },
      },
      fields: [
        { name: "bill name", change: () => change(/bill name/i, "Home loan") },
        { name: "amount", change: () => change(/amount/i, "1600") },
        { name: "first due date", change: () => change(/first due date/i, "2026-08-05") },
        { name: "account (pay from)", change: () => fireEvent.change(byId("txn-account"), { target: { value: SAVINGS.id } }) },
        { name: "interval count", change: () => change("Interval count", "3") },
        { name: "interval unit", change: () => change("Interval unit", "weeks") },
        { name: "note", change: () => { openDetails(); change("Note", "Home loan"); } },
        { name: "category", change: () => { openDetails(); change("Category", CATEGORY.id); } },
        {
          name: "a tag",
          change: async () => {
            openDetails();
            const input = screen.getByLabelText("Add a tag");
            fireEvent.change(input, { target: { value: "Vacation" } });
            fireEvent.keyDown(input, { key: "Enter" });
            await vi.waitFor(() => expect(mocks.createTag).toHaveBeenCalled());
          },
        },
        // Last: choosing a fixed frequency hides the custom interval fields.
        { name: "frequency", change: () => change(/frequency/i, "weekly") },
      ],
    });
  });
});

describe("Add transfer: key identity (personal-cfo-56w.8)", () => {
  const JOINT = account({ id: "0190a000-0000-7000-8000-000000000003", name: "Joint" });
  const WALLET = account({ id: "0190a000-0000-7000-8000-000000000004", name: "Wallet" });
  const REFUSED = { status: "error", error: { Validation: "That account cannot cover this." } } as const;

  /// Both transfer commands in the order they were called, so one spec can follow the draft across
  /// the One-off / Recurring switch.
  const sent: string[] = [];
  /// What the next call gets, whichever command it is; without one it throws, a lost connection.
  let next: { answer: unknown } | null = null;
  const respond = async (input: { idempotency_key: string }) => {
    sent.push(input.idempotency_key);
    if (next === null) throw new Error("connection lost");
    const { answer } = next;
    next = null;
    return answer;
  };
  beforeEach(() => {
    sent.length = 0;
    next = null;
    mocks.accountList.mockResolvedValue(ok([account(), SAVINGS, JOINT, WALLET]));
    mocks.recordTransfer.mockImplementation(respond);
    mocks.createRecurringTransfer.mockImplementation(respond);
  });
  const answerNext = (answer: unknown) => {
    next = { answer };
  };

  const field = (id: string) => byId(id);
  const pick = (id: string, value: string) => fireEvent.change(field(id), { target: { value } });
  const submitButton = () => screen.getByRole("button", { name: /^add transfer$|schedule transfer/i });
  const send = async () => {
    // The form is locked until the previous call settles.
    await vi.waitFor(() => expect(submitButton()).toBeEnabled());
    fireEvent.click(submitButton());
  };
  const openTransfer = async (recurring: boolean) => {
    renderWithClient(<TransactionsView />);
    fireEvent.click(await screen.findByRole("button", { name: /^add transfer$/i }));
    if (recurring) fireEvent.click(screen.getByRole("button", { name: "Recurring" }));
    change(/amount/i, "50");
    pick("transfer-date", "2026-06-05");
  };
  const reopen = (recurring: boolean) => async () => {
    fireEvent.click(await screen.findByRole("button", { name: /^add transfer$/i }));
    if (recurring) fireEvent.click(screen.getByRole("button", { name: "Recurring" }));
    change(/amount/i, "50");
    pick("transfer-date", "2026-06-05");
  };

  it("one-off: the accounts, the amount, the date and the mode", async () => {
    await expectDraftKeyIdentity({
      name: "Add transfer (one-off)",
      open: () => openTransfer(false),
      send,
      keys: () => sent,
      refusal: { arrange: () => answerNext(REFUSED) },
      success: { arrange: () => answerNext(ok({ op_seq: 1, replayed: false })), reopen: reopen(false) },
      fields: [
        { name: "To", change: () => pick("transfer-dest", JOINT.id) },
        { name: "From", change: () => pick("transfer-source", SAVINGS.id) },
        { name: "amount", change: () => change(/amount/i, "51") },
        { name: "date", change: () => pick("transfer-date", "2026-06-06") },
        // Last: from here on the form is a recurring transfer.
        { name: "mode (recurring instead of one-off)", change: () => { fireEvent.click(screen.getByRole("button", { name: "Recurring" })); } },
      ],
    });
  });

  it("recurring: the accounts, the amount, the start date and the schedule", async () => {
    await expectDraftKeyIdentity({
      name: "Add transfer (recurring)",
      open: () => openTransfer(true),
      send,
      keys: () => sent,
      refusal: { arrange: () => answerNext(REFUSED) },
      success: { arrange: () => answerNext(ok({ recurring_transfer_id: "rt-1", mutation: { op_seq: 1, replayed: false } })), reopen: reopen(true) },
      fields: [
        { name: "To", change: () => pick("transfer-dest", JOINT.id) },
        { name: "From", change: () => pick("transfer-source", SAVINGS.id) },
        { name: "amount", change: () => change(/amount/i, "51") },
        { name: "start date", change: () => pick("transfer-date", "2026-06-06") },
        { name: "frequency", change: () => pick("transfer-frequency", "weekly") },
        {
          name: "a custom interval's count",
          change: () => {
            pick("transfer-frequency", "custom");
            change("Interval count", "2");
            change("Interval unit", "months");
            // Sent once as a first draft of the custom schedule; the next change is the count.
          },
        },
        { name: "interval count", change: () => change("Interval count", "3") },
        { name: "interval unit", change: () => change("Interval unit", "weeks") },
      ],
    });
  });
});
