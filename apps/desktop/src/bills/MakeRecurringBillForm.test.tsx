import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, TransactionRowDto } from "@/bindings";
import { MakeRecurringBillForm } from "./BillsView";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  baseCurrency: vi.fn(),
  recurringBillList: vi.fn(),
  createRecurringBill: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    baseCurrency: mocks.baseCurrency,
    recurringBillList: mocks.recurringBillList,
    createRecurringBill: mocks.createRecurringBill,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });

const CHECKING = "0190a000-0000-7000-8000-000000000001";
const CARD = "0190a000-0000-7000-8000-0000000000c1";

function account(over: Partial<AccountViewDto>): AccountViewDto {
  return {
    id: CHECKING,
    name: "Checking",
    cashflow_role: "liquid_cash",
    subtype: null,
    active: true,
    balance: { minor_units: 100_000, currency: "USD" },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
    ...over,
  };
}

function txn(over: Partial<TransactionRowDto> = {}): TransactionRowDto {
  return {
    transaction_id: "0190b000-0000-7000-8000-000000000001",
    account_id: CHECKING,
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-15T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -1200, currency: "USD" },
    memo: null,
    counterparty: "Netflix",
    category_id: null,
    reviewed: true,
    note: null,
    tag_ids: [],
    split_count: 0,
    category_source: null,
    category_confidence_bps: null,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(
    ok([
      account({ id: CHECKING, name: "Checking", cashflow_role: "liquid_cash" }),
      account({ id: CARD, name: "Amex", cashflow_role: "credit_facility" }),
    ]),
  );
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.createRecurringBill.mockResolvedValue(mutationOk());
});

test("pre-fills from the transaction and creates a bill paid from that account", async () => {
  const onCreated = vi.fn();
  renderWithClient(
    <MakeRecurringBillForm transaction={txn()} onCancel={() => {}} onCreated={onCreated} />,
  );

  // Pre-filled from the merchant + amount.
  const name = await screen.findByLabelText<HTMLInputElement>("Bill name");
  expect(name.value).toBe("Netflix");
  expect(screen.getByLabelText<HTMLInputElement>("Amount").value).toBe("12");
  // The pay-from select visibly shows the txn's account (not "No autopay account"),
  // i.e. the form waited for accounts to load before mounting (review fix).
  expect(screen.getByLabelText<HTMLSelectElement>("Autopay account").value).toBe(CHECKING);

  fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));

  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  const input = mocks.createRecurringBill.mock.calls[0]?.[0];
  expect(input).toMatchObject({
    name: "Netflix",
    frequency: "monthly",
    anchor_date: "2026-06-15",
    autopay_account_id: CHECKING, // pay-from = the txn's (liquid) account
    amount: { minor_units: 1200, currency: "USD" },
  });
  expect(onCreated).toHaveBeenCalled();
});

test("a card transaction creates a bill paid from that card", async () => {
  renderWithClient(
    <MakeRecurringBillForm
      transaction={txn({ account_id: CARD, account_name: "Amex", counterparty: "Spotify" })}
      onCancel={() => {}}
      onCreated={() => {}}
    />,
  );

  await screen.findByLabelText("Bill name");
  fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));

  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  const input = mocks.createRecurringBill.mock.calls[0]?.[0];
  expect(input.autopay_account_id).toBe(CARD);
  expect(input.name).toBe("Spotify");
});
