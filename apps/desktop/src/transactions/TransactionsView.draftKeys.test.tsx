/**
 * personal-cfo-56w.8: the per-draft idempotency key of the Add transaction form and of a recurring bill
 * created from it (personal-cfo-g3m.35, 6wk.23), pinned for EVERY field of the draft's identity with the
 * shared helper. Lives in its own file so it stays clear of the large TransactionsView suite.
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
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
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
});

const change = (label: string | RegExp, value: string) =>
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
const byId = (id: string) => document.getElementById(id) as HTMLElement;
const openDetails = () => {
  if (!screen.queryByLabelText("Note")) {
    fireEvent.click(screen.getByRole("button", { name: /add details/i }));
  }
};

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
        fireEvent.click(screen.getByLabelText(/set up as a recurring bill/i));
        change(/bill name/i, "Mortgage");
        change(/amount/i, "1500");
        change(/first due date/i, "2026-07-05");
        change(/frequency/i, "custom");
        change("Interval count", "2");
        change("Interval unit", "months");
      },
      send: () => {
        fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
      },
      keys: () => mocks.createRecurringBill.mock.calls.map((c) => c[0].idempotency_key),
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
