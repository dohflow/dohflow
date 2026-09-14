import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { AccountViewDto, CardStatementForecastDto } from "@/bindings";
import { PayCardAction } from "./PayCardAction";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  transactionList: vi.fn(),
  recordTransfer: vi.fn(),
}));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (m: number) => ({ minor_units: m, currency: "USD" });

const card: CardStatementForecastDto = {
  account_id: "card1",
  account_name: "Visa",
  currency: "USD",
  credit_limit_minor: 1_000_000,
  repayment_philosophy: "pay_statement_balance",
    estimate_mape_bps: 0,
  cycles: [
    {
      close_date: "2026-07-25",
      due_date: "2026-08-15",
      carried_opening_balance_minor: 120_000,
      known_charges_minor: 0,
      projected_variable_minor: 0,
      accrued_interest_minor: 0,
      statement_balance_minor: 80_000,
      minimum_due_minor: 2_500,
      full_pay_minor: 120_000,
      forecast_payment_minor: 80_000,
      statement_is_actual: false,
      is_closed: true,
    },
  ],
  stored_statements: [],
  estimate_basis: "none",
  estimate_sample_cycles: 0,
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.transactionList.mockResolvedValue(ok([]));
  mocks.recordTransfer.mockResolvedValue(ok({ op_seq: 1, replayed: false }));
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "chk",
        name: "Checking",
        cashflow_role: "liquid_cash",
        subtype: "checking",
        active: true,
        balance: usd(500_000),
      },
    ] as AccountViewDto[]),
  );
});

test("pays the statement balance by default to the card", async () => {
  renderWithClient(<PayCardAction card={card} />);

  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  fireEvent.click(await screen.findByRole("button", { name: /confirm payment/i }));

  await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
  const input = mocks.recordTransfer.mock.calls[0]?.[0];
  expect(input.dest_account_id).toBe("card1");
  expect(input.source_account_id).toBe("chk");
  expect(input.amount.minor_units).toBe(80_000); // the statement balance
});

test("a quick-pick sets the amount (current balance)", async () => {
  renderWithClient(<PayCardAction card={card} />);

  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  fireEvent.click(await screen.findByRole("button", { name: /current balance/i }));
  fireEvent.click(screen.getByRole("button", { name: /confirm payment/i }));

  await waitFor(() => expect(mocks.recordTransfer).toHaveBeenCalledTimes(1));
  expect(mocks.recordTransfer.mock.calls[0]?.[0].amount.minor_units).toBe(120_000);
});

test("shows no source when no liquid account holds the card's currency", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "eur",
        name: "Euro",
        cashflow_role: "liquid_cash",
        subtype: "checking",
        active: true,
        balance: { minor_units: 500_000, currency: "EUR" },
      },
    ] as AccountViewDto[]),
  );
  renderWithClient(<PayCardAction card={card} />);
  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  expect(await screen.findByText(/no liquid account holds funds/i)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull();
});

test("does not offer an investment account as a source (liquidate to cash first)", async () => {
  mocks.accountList.mockResolvedValue(
    ok([
      {
        id: "brk",
        name: "Brokerage",
        cashflow_role: "investment_asset",
        subtype: "brokerage",
        active: true,
        balance: usd(900_000),
      },
    ] as AccountViewDto[]),
  );
  renderWithClient(<PayCardAction card={card} />);

  fireEvent.click(screen.getByRole("button", { name: /pay card/i }));
  // A brokerage isn't a valid direct card payer, so the tool offers no source.
  expect(await screen.findByText(/no liquid account holds funds/i)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /confirm payment/i })).toBeNull();
});
