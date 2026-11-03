import { render, screen, within } from "@testing-library/react";

import type {
  AccountViewDto,
  CardCycleDto,
  CardStatementForecastDto,
  DebtTermsDto,
} from "@/bindings";

import { DebtTermsTable } from "./DebtTermsTable";
import type { ScopedDebt } from "./debtStats";

const terms = (accountId: string, over: Partial<DebtTermsDto> = {}): DebtTermsDto => ({
  account_id: accountId,
  apr_bps: 1_999,
  statement_close_day: 5,
  payment_due_day: 25,
  grace_period_days: null,
  credit_limit_minor: null,
  repayment_philosophy: "pay_fixed_amount",
  fixed_amount_minor: 12_300,
  min_payment_percent_bps: null,
  min_payment_floor_minor: null,
  paying_source_account_id: null,
  original_principal_minor: null,
  ...over,
});

const debt = (id: string, name: string, currency: string, owedMinor: number): ScopedDebt => ({
  account: {
    id,
    name,
    cashflow_role: "credit_facility",
    subtype: "credit_card",
    active: true,
    balance: { minor_units: -owedMinor, currency },
    notes: null,
    linked_account_id: null,
    linked_account_name: null,
  } as AccountViewDto,
  terms: terms(id),
  owedMinor,
});

const card = (id: string, currency: string, statementMinor: number): CardStatementForecastDto =>
  ({
    account_id: id,
    account_name: id,
    currency,
    credit_limit_minor: 0,
    repayment_philosophy: "pay_fixed_amount",
    cycles: [{ statement_balance_minor: statementMinor } as CardCycleDto],
    statements: [],
  }) as unknown as CardStatementForecastDto;

const USD_DEBT = debt("usd", "Visa", "USD", 250_000);
const EUR_DEBT = debt("eur", "Carte", "EUR", 480_000);
const CARDS = [card("usd", "USD", 31_000), card("eur", "EUR", 47_000)];

function rowOf(name: string) {
  return screen.getByRole("row", { name: new RegExp(name) });
}

describe("DebtTermsTable (personal-cfo-6wk.25)", () => {
  it.each([
    ["USD first", [USD_DEBT, EUR_DEBT]],
    ["EUR first", [EUR_DEBT, USD_DEBT]],
  ])("formats every money cell in its own debt's currency (%s)", (_order, debts) => {
    render(<DebtTermsTable debts={debts} cards={CARDS} />);

    const usd = rowOf("Visa");
    expect(within(usd).getByText("$2,500.00")).toBeInTheDocument(); // Amount owed
    expect(within(usd).getByText("$310.00")).toBeInTheDocument(); // Projected statement
    expect(within(usd).getByText("$123.00")).toBeInTheDocument(); // Monthly payment
    expect(within(usd).queryByText(/€/)).toBeNull();

    const eur = rowOf("Carte");
    expect(within(eur).getByText("€4,800.00")).toBeInTheDocument();
    expect(within(eur).getByText("€470.00")).toBeInTheDocument();
    expect(within(eur).getByText("€123.00")).toBeInTheDocument();
    expect(within(eur).queryByText(/\$/)).toBeNull();
  });

  it("renders a single currency as before", () => {
    render(<DebtTermsTable debts={[USD_DEBT]} cards={[card("usd", "USD", 31_000)]} />);
    const usd = rowOf("Visa");
    expect(within(usd).getByText("$2,500.00")).toBeInTheDocument();
    expect(within(usd).getByText("$310.00")).toBeInTheDocument();
    expect(within(usd).getByText("$123.00")).toBeInTheDocument();
  });
});
