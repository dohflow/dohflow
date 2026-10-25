import { screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { LoanDoubleCountWarningDto } from "@/bindings";
import { LoanDoubleCountWarning } from "./LoanDoubleCountWarning";

const mocks = vi.hoisted(() => ({
  loanDoubleCountWarnings: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    loanDoubleCountWarnings: mocks.loanDoubleCountWarnings,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function warning(over: Partial<LoanDoubleCountWarningDto> = {}): LoanDoubleCountWarningDto {
  return {
    loan_account_id: "loan-1",
    loan_name: "Car Loan",
    bill_event_id: "bill-1",
    bill_name: "Car-Loan Payment",
    name_match: true,
    amount_match: false,
    strength: "strong",
    matched_by: "loan_typed",
    bill_type: "loan_payment",
    paid_from_account_name: "Checking",
    ...over,
  };
}

test("names the suspected duplicated loan/bill pair and why it matched", async () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(ok([warning({ amount_match: true })]));
  renderWithClient(<LoanDoubleCountWarning />);

  expect(await screen.findByText(/possible duplicated loan/i)).toBeInTheDocument();
  // The pair is a single labelled region naming both sides.
  const region = screen.getByRole("region", { name: /possible duplicated loans/i });
  expect(region).toHaveTextContent("Car Loan");
  expect(region).toHaveTextContent("Car-Loan Payment");
  // The approved card sentence, and the loan-typed route's reason with the type label and account.
  expect(
    screen.getByText(
      "This loan's payment is also set up as a recurring bill, so it's counted twice in Cash Flow and your forecast.",
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByText("A Loan payment bill paid from Checking around Car Loan's due date."),
  ).toBeInTheDocument();
  expect(region).not.toHaveTextContent(/loan-payment bill/i);
  expect(region).not.toHaveTextContent(/matched by/i);
});

test("renders nothing when there is no overlap", () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(ok([]));
  const { container } = renderWithClient(<LoanDoubleCountWarning />);
  expect(container).toBeEmptyDOMElement();
});

test("a POSSIBLE match is a soft note that never says the payment is counted twice", async () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(
    ok([warning({ strength: "possible", amount_match: true, bill_name: "ACME HOME LOANS" })]),
  );
  const { container } = renderWithClient(<LoanDoubleCountWarning />);

  expect(
    await screen.findByText("This bill might be the same as Car Loan's payment."),
  ).toBeInTheDocument();
  // No claim, no strong-match card, no link-style wording anywhere on the surface.
  expect(container).not.toHaveTextContent(/counted twice/i);
  expect(container).not.toHaveTextContent(/twice/i);
  expect(container).not.toHaveTextContent(/duplicated/i);
  expect(screen.queryByRole("region", { name: /possible duplicated loans/i })).toBeNull();
});

test("a STRONG match says it, and a possible one beside it stays soft", async () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(
    ok([
      warning({ strength: "strong", bill_event_id: "bill-1", bill_name: "Car-Loan Payment" }),
      warning({ strength: "possible", bill_event_id: "bill-2", bill_name: "Maybe Bill" }),
    ]),
  );
  renderWithClient(<LoanDoubleCountWarning />);

  const strong = await screen.findByRole("region", { name: /possible duplicated loans/i });
  expect(strong).toHaveTextContent(/counted twice/i);
  expect(strong).toHaveTextContent("Car-Loan Payment");
  expect(strong).not.toHaveTextContent("Maybe Bill");
  const soft = screen.getByRole("region", { name: /possible matches/i });
  expect(soft).toHaveTextContent("Maybe Bill");
  expect(soft).not.toHaveTextContent(/twice/i);
});

test("the payee route and the rent-or-mortgage label use the approved sentences", async () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(
    ok([
      warning({ matched_by: "payee", bill_event_id: "bill-1" }),
      warning({
        matched_by: "loan_typed",
        bill_type: "rent_mortgage",
        paid_from_account_name: "Joint Checking",
        bill_event_id: "bill-2",
      }),
    ]),
  );
  renderWithClient(<LoanDoubleCountWarning />);

  expect(
    await screen.findByText(
      "Same payee as the payments already made to Car Loan, around its due date.",
    ),
  ).toBeInTheDocument();
  expect(
    screen.getByText("A Rent or mortgage bill paid from Joint Checking around Car Loan's due date."),
  ).toBeInTheDocument();
});

test("a strong match whose evidence cannot be named shows no reason line rather than a guess", async () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(
    ok([warning({ matched_by: null, bill_type: null, paid_from_account_name: null })]),
  );
  renderWithClient(<LoanDoubleCountWarning />);
  const region = await screen.findByRole("region", { name: /possible duplicated loans/i });
  expect(region).not.toHaveTextContent(/payee|paid from/i);
});
