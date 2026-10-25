import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, LoanOverlapMatchDto, TransactionRowDto } from "@/bindings";
import { MakeRecurringBillForm } from "./BillsView";

// The creation-time loan-payment check (personal-cfo-6wk.20): before a bill is created, it is
// compared with the loans' payments. Strong cancels creation by default, possible is a soft note,
// and the advisory check never blocks creation when it cannot run.

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  baseCurrency: vi.fn(),
  recurringBillList: vi.fn(),
  createRecurringBill: vi.fn(),
  checkLoanOverlap: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    baseCurrency: mocks.baseCurrency,
    recurringBillList: mocks.recurringBillList,
    createRecurringBill: mocks.createRecurringBill,
    checkLoanOverlap: mocks.checkLoanOverlap,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });
const CHECKING = "0190a000-0000-7000-8000-000000000001";

function account(): AccountViewDto {
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
  };
}

function txn(): TransactionRowDto {
  return {
    transaction_id: "0190b000-0000-7000-8000-000000000001",
    account_id: CHECKING,
    account_name: "Checking",
    counter_account_id: null,
    counter_account_name: null,
    occurred_at: "2026-06-15T00:00:00Z",
    transaction_date: null,
    balance_after_minor: null,
    amount: { minor_units: -150_000, currency: "USD" },
    memo: null,
    counterparty: "ACME HOME LOANS",
    category_id: null,
    reviewed: true,
    note: null,
    tag_ids: [],
    split_count: 0,
    category_source: null,
    category_confidence_bps: null,
  };
}

function match(over: Partial<LoanOverlapMatchDto> = {}): LoanOverlapMatchDto {
  return {
    loan_account_id: "loan-1",
    loan_name: "Test Mortgage",
    strength: "strong",
    matched_by: "loan_typed",
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(ok([account()]));
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.createRecurringBill.mockResolvedValue(mutationOk());
  mocks.checkLoanOverlap.mockResolvedValue(ok(null));
});

async function submit(onCancel = vi.fn(), onCreated = vi.fn()) {
  renderWithClient(
    <MakeRecurringBillForm transaction={txn()} onCancel={onCancel} onCreated={onCreated} />,
  );
  await screen.findByLabelText("Bill name");
  fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
  return { onCancel, onCreated };
}

test("with no match the bill is created straight away, after the check saw exactly what would be created", async () => {
  const { onCreated } = await submit();
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  expect(mocks.checkLoanOverlap).toHaveBeenCalledTimes(1);
  expect(mocks.checkLoanOverlap.mock.calls[0]?.[0]).toMatchObject({
    name: "ACME HOME LOANS",
    amount: { minor_units: 150_000, currency: "USD" },
    autopay_account_id: CHECKING,
    anchor_date: "2026-06-15",
  });
  expect(onCreated).toHaveBeenCalled();
});

test("a STRONG match stops creation and says this is the loan's payment", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  await submit();

  expect(
    await screen.findByText("This is Test Mortgage's payment — don't add a second one."),
  ).toBeInTheDocument();
  expect(mocks.createRecurringBill).not.toHaveBeenCalled();
});

test("\"Don't add\" cancels without creating anything", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  const { onCancel, onCreated } = await submit();

  fireEvent.click(await screen.findByRole("button", { name: /don't add/i }));
  expect(onCancel).toHaveBeenCalledTimes(1);
  expect(mocks.createRecurringBill).not.toHaveBeenCalled();
  expect(onCreated).not.toHaveBeenCalled();
});

test("\"Add anyway\" creates the bill: the user stays in control (ADR 0018)", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  const { onCreated } = await submit();

  fireEvent.click(await screen.findByRole("button", { name: /add anyway/i }));
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  expect(onCreated).toHaveBeenCalled();
});

test("a POSSIBLE match is a soft note, never the strong sentence, and creating is one click", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match({ strength: "possible", matched_by: null })));
  const { onCreated } = await submit();

  expect(
    await screen.findByText("This bill might be the same as Test Mortgage's payment."),
  ).toBeInTheDocument();
  expect(screen.queryByText(/don't add a second one/i)).toBeNull();
  expect(screen.queryByRole("button", { name: /don't add/i })).toBeNull();
  expect(mocks.createRecurringBill).not.toHaveBeenCalled();

  fireEvent.click(screen.getByRole("button", { name: /create recurring bill/i }));
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  expect(onCreated).toHaveBeenCalled();
});

test("if the advisory check returns an error the bill is still created", async () => {
  mocks.checkLoanOverlap.mockResolvedValue({
    status: "error",
    error: { kind: "internal", message: "boom" },
  });
  await submit();
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
});

test("if the advisory check throws the bill is still created", async () => {
  mocks.checkLoanOverlap.mockRejectedValue(new Error("no IPC"));
  await submit();
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
});

// ----- the declared-terms ASK: the user's own loan terms are enough to ask, never to assert ----------------

const ASK = { strength: "possible", matched_by: "declared_terms" } as const;

test("a declared-terms match ASKS whether it is the loan's payment, with the reason from its terms", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match(ASK)));
  await submit();

  expect(await screen.findByText("Is this Test Mortgage's payment?")).toBeInTheDocument();
  expect(
    screen.getByText(
      "It's paid from Checking around Test Mortgage's due date, as set in its terms.",
    ),
  ).toBeInTheDocument();
  // An ask never claims anything is counted twice.
  expect(screen.queryByText(/counted twice/i)).toBeNull();
  expect(mocks.createRecurringBill).not.toHaveBeenCalled();
});

test("answering Yes cancels without creating a second bill", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match(ASK)));
  const { onCancel, onCreated } = await submit();

  fireEvent.click(await screen.findByRole("button", { name: "Yes — don't add a second one" }));
  expect(onCancel).toHaveBeenCalledTimes(1);
  expect(mocks.createRecurringBill).not.toHaveBeenCalled();
  expect(onCreated).not.toHaveBeenCalled();
});

test("answering No adds the bill (an unrelated similar bill is not blocked)", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match(ASK)));
  const { onCreated } = await submit();

  fireEvent.click(await screen.findByRole("button", { name: "No, it's a different bill" }));
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  expect(onCreated).toHaveBeenCalled();
  // One check, one question: the answer does not trigger another ask.
  expect(mocks.checkLoanOverlap).toHaveBeenCalledTimes(1);
});

