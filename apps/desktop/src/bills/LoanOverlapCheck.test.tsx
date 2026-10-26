import { fireEvent, screen, waitFor, within } from "@testing-library/react";

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
  linkBillToLoan: vi.fn(),
  setBillNotLoanPayment: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    baseCurrency: mocks.baseCurrency,
    recurringBillList: mocks.recurringBillList,
    createRecurringBill: mocks.createRecurringBill,
    checkLoanOverlap: mocks.checkLoanOverlap,
    linkBillToLoan: mocks.linkBillToLoan,
    setBillNotLoanPayment: mocks.setBillNotLoanPayment,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const mutationOk = () => ok({ op_seq: 1, replayed: false });
const createdOk = () => ok({ op_seq: 1, replayed: false, event_id: "bill-new" });
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
  mocks.createRecurringBill.mockResolvedValue(createdOk());
  mocks.checkLoanOverlap.mockResolvedValue(ok(null));
  mocks.linkBillToLoan.mockResolvedValue(mutationOk());
  mocks.setBillNotLoanPayment.mockResolvedValue(mutationOk());
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

// ----- "Add and link to <loan>" and the remembered No (personal-cfo-6wk.22) -------------------------------

test("a STRONG match offers 'Add and link to <loan>', which creates the bill and then links it", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  const { onCreated } = await submit();

  fireEvent.click(await screen.findByRole("button", { name: "Add and link to Test Mortgage" }));
  await waitFor(() => expect(mocks.linkBillToLoan).toHaveBeenCalledTimes(1));
  expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);
  expect(mocks.linkBillToLoan.mock.calls[0]?.slice(0, 2)).toEqual(["bill-new", "loan-1"]);
  expect(mocks.setBillNotLoanPayment).not.toHaveBeenCalled();
  expect(onCreated).toHaveBeenCalled();
  // Created BEFORE linked: the link needs the new bill's id.
  expect(mocks.createRecurringBill.mock.invocationCallOrder[0]).toBeLessThan(
    mocks.linkBillToLoan.mock.invocationCallOrder[0] ?? 0,
  );
});

test("the link is not offered when the loan already has a linked bill", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  mocks.recurringBillList.mockResolvedValue(
    ok([
      {
        id: "bill-existing",
        name: "Existing",
        loan_link: { loan_account_id: "loan-1", loan_name: "Test Mortgage", in_force: true },
      },
    ]),
  );
  await submit();

  expect(await screen.findByRole("button", { name: /add anyway/i })).toBeInTheDocument();
  await waitFor(() => expect(mocks.recurringBillList).toHaveBeenCalled());
  expect(screen.queryByRole("button", { name: /add and link/i })).toBeNull();
});

test("'Add anyway' and 'Don't add' neither link nor record an answer", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  await submit();
  fireEvent.click(await screen.findByRole("button", { name: /add anyway/i }));
  await waitFor(() => expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1));
  expect(mocks.linkBillToLoan).not.toHaveBeenCalled();
  expect(mocks.setBillNotLoanPayment).not.toHaveBeenCalled();
});

test("answering No remembers that the new bill is not the loan's payment", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match(ASK)));
  await submit();

  fireEvent.click(await screen.findByRole("button", { name: "No, it's a different bill" }));
  await waitFor(() => expect(mocks.setBillNotLoanPayment).toHaveBeenCalledTimes(1));
  expect(mocks.setBillNotLoanPayment.mock.calls[0]?.slice(0, 2)).toEqual(["bill-new", "loan-1"]);
  expect(mocks.linkBillToLoan).not.toHaveBeenCalled();
});

test("answering Yes (don't add) records nothing", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match(ASK)));
  await submit();
  fireEvent.click(await screen.findByRole("button", { name: "Yes — don't add a second one" }));
  expect(mocks.createRecurringBill).not.toHaveBeenCalled();
  expect(mocks.setBillNotLoanPayment).not.toHaveBeenCalled();
});

test("if linking fails after the bill was created, the bill stays, creation completes, and an error notice says what to do", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  mocks.linkBillToLoan.mockResolvedValue({
    status: "error",
    error: { LoanAlreadyLinked: { bill_id: "other", bill_name: "Other" } },
  });
  const { onCreated } = await submit();
  fireEvent.click(await screen.findByRole("button", { name: "Add and link to Test Mortgage" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);

  // Never silent (FRONTEND.md section 6): the approved sentence, as an alert in the shell region.
  const region = await screen.findByRole("region", { name: "Notifications" });
  const alert = await within(region).findByRole("alert");
  await waitFor(() => expect(alert).toHaveTextContent('The bill was added, but linking it to Test Mortgage didn\'t work. Use "Link it" on the warning to try again.'));
});

test("if saving the answer fails after the bill was created, an error notice says it may be asked again", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match(ASK)));
  mocks.setBillNotLoanPayment.mockResolvedValue({
    status: "error",
    error: { Validation: "recurring bill does not exist" },
  });
  const { onCreated } = await submit();
  fireEvent.click(await screen.findByRole("button", { name: "No, it's a different bill" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  expect(mocks.createRecurringBill).toHaveBeenCalledTimes(1);

  const region = await screen.findByRole("region", { name: "Notifications" });
  const alert = await within(region).findByRole("alert");
  await waitFor(() => expect(alert).toHaveTextContent("The bill was added, but your answer wasn't saved, so DohFlow may ask again."));
});

test("when the follow-up works, there is no notice", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  const { onCreated } = await submit();
  fireEvent.click(await screen.findByRole("button", { name: "Add and link to Test Mortgage" }));
  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  expect(screen.queryByRole("region", { name: "Notifications" })).toBeNull();
});

test("a bill added with no follow-up shows no notice even if it is an ordinary add", async () => {
  mocks.checkLoanOverlap.mockResolvedValue(ok(match()));
  const { onCreated } = await submit();
  fireEvent.click(await screen.findByRole("button", { name: /add anyway/i }));
  await waitFor(() => expect(onCreated).toHaveBeenCalled());
  expect(screen.queryByRole("region", { name: "Notifications" })).toBeNull();
});

