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
  // Matched-by explanation reflects the flags.
  expect(screen.getByText("Matched by name and payment amount.")).toBeInTheDocument();
});

test("renders nothing when there is no overlap", () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(ok([]));
  const { container } = renderWithClient(<LoanDoubleCountWarning />);
  expect(container).toBeEmptyDOMElement();
});
