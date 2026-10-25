import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { BillLoanLinkDto, RecurringBillDto } from "@/bindings";
import { formatMoney } from "@/lib/format";
import { BillLoanLinkSection } from "./BillLoanLinkSection";

const mocks = vi.hoisted(() => ({
  unlinkBillFromLoan: vi.fn(),
  setBillNotLoanPayment: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    unlinkBillFromLoan: mocks.unlinkBillFromLoan,
    setBillNotLoanPayment: mocks.setBillNotLoanPayment,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (minor_units: number) => ({ minor_units, currency: "USD" });

function link(over: Partial<BillLoanLinkDto> = {}): BillLoanLinkDto {
  return {
    loan_account_id: "loan-1",
    loan_name: "Test Mortgage",
    in_force: true,
    cadence_changed: false,
    below_scheduled: false,
    scheduled_payment: usd(150_000),
    other: null,
    ...over,
  };
}

function bill(over: Partial<RecurringBillDto> = {}): RecurringBillDto {
  return {
    id: "bill-1",
    name: "ACME HOME LOANS",
    bill_type: "subscription",
    amount: usd(150_000),
    frequency: "monthly",
    anchor_date: "2026-07-15",
    autopay_account_id: null,
    autopay_account_name: null,
    autopay_enabled: false,
    next_due_date: "2026-07-15",
    description: null,
    active: true,
    created_at: "2026-06-01T00:00:00Z",
    archived_at: null,
    category_id: null,
    tag_ids: [],
    loan_link: null,
    not_loan_payment: null,
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.unlinkBillFromLoan.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  mocks.setBillNotLoanPayment.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
});

test("a bill with neither a link nor an answer shows nothing", () => {
  const { container } = renderWithClient(<BillLoanLinkSection bill={bill()} />);
  expect(container).toBeEmptyDOMElement();
});

test("a linked bill says so, and Unlink removes the link", async () => {
  renderWithClient(<BillLoanLinkSection bill={bill({ loan_link: link() })} />);

  expect(screen.getByText("Linked to Test Mortgage's payment.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Unlink" }));
  await waitFor(() => expect(mocks.unlinkBillFromLoan).toHaveBeenCalledTimes(1));
  expect(mocks.unlinkBillFromLoan.mock.calls[0]?.[0]).toBe("bill-1");
});

const SPLIT = `Includes ${formatMoney(usd(150_000))} for Test Mortgage (principal and interest) and ${formatMoney(usd(120_000))} other, such as escrow.`;

test("the split is shown while the link is in force and the bill is larger", () => {
  renderWithClient(<BillLoanLinkSection bill={bill({ loan_link: link({ other: usd(120_000) }) })} />);
  expect(screen.getByText(SPLIT)).toBeInTheDocument();
});

test("there is no split when the bill is not larger than the scheduled payment", () => {
  renderWithClient(<BillLoanLinkSection bill={bill({ loan_link: link({ other: null }) })} />);
  expect(screen.queryByText(/Includes/)).toBeNull();
});

test("there is no split while the link is not in force", () => {
  renderWithClient(
    <BillLoanLinkSection
      bill={bill({ loan_link: link({ other: usd(120_000), in_force: false }) })}
    />,
  );
  expect(screen.queryByText(/Includes/)).toBeNull();
});

test("a link that no longer matches the loan's cadence says both now count", () => {
  renderWithClient(
    <BillLoanLinkSection
      bill={bill({ loan_link: link({ in_force: false, cadence_changed: true }) })}
    />,
  );
  expect(
    screen.getByText("This bill no longer repeats like Test Mortgage's payment, so both now count."),
  ).toBeInTheDocument();
  expect(screen.queryByText(/less than/)).toBeNull();
});

test("a linked bill below the scheduled payment is warned about", () => {
  renderWithClient(
    <BillLoanLinkSection bill={bill({ loan_link: link({ below_scheduled: true }) })} />,
  );
  expect(
    screen.getByText("This linked bill is less than Test Mortgage's scheduled payment."),
  ).toBeInTheDocument();
});

test("a remembered answer is shown and can be forgotten", async () => {
  renderWithClient(
    <BillLoanLinkSection
      bill={bill({ not_loan_payment: { loan_account_id: "loan-1", loan_name: "Test Mortgage" } })}
    />,
  );
  expect(screen.getByText("You said this isn't Test Mortgage's payment.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Forget this answer" }));
  await waitFor(() => expect(mocks.setBillNotLoanPayment).toHaveBeenCalledTimes(1));
  expect(mocks.setBillNotLoanPayment.mock.calls[0]?.slice(0, 2)).toEqual(["bill-1", null]);
});

test("a failed action says so in place", async () => {
  mocks.unlinkBillFromLoan.mockResolvedValue({
    status: "error",
    error: { Validation: "recurring bill does not exist" },
  });
  renderWithClient(<BillLoanLinkSection bill={bill({ loan_link: link() })} />);
  fireEvent.click(screen.getByRole("button", { name: "Unlink" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("recurring bill does not exist");
  expect(screen.getByRole("button", { name: "Unlink" })).toBeEnabled();
});
