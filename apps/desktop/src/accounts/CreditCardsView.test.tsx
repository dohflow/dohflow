import { fireEvent, screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { CardStatementForecastDto, ForecastViewDto } from "@/bindings";
import { CreditCardsView } from "./CreditCardsView";

const mocks = vi.hoisted(() => ({
  cardStatementForecast: vi.fn(),
  futureCashForecast: vi.fn(),
  setCardStatementBalance: vi.fn(),
  cardStatementHistory: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    cardStatementForecast: mocks.cardStatementForecast,
    futureCashForecast: mocks.futureCashForecast,
    setCardStatementBalance: mocks.setCardStatementBalance,
    cardStatementHistory: mocks.cardStatementHistory,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function card(over: Partial<CardStatementForecastDto> = {}): CardStatementForecastDto {
  return {
    account_id: "card-1",
    account_name: "Visa",
    currency: "USD",
    credit_limit_minor: 500_000, // $5,000
    repayment_philosophy: "pay_statement_balance",
    estimate_mape_bps: 0,
    cycles: [
      {
        close_date: "2026-08-05",
        due_date: "2026-08-25",
        carried_opening_balance_minor: 100_000, // $1,000 owed → 20% utilization
        known_charges_minor: 5_000,
        projected_variable_minor: 20_000,
        accrued_interest_minor: 2_000,
        statement_balance_minor: 127_000, // $1,270
        minimum_due_minor: 2_500, // $25
        full_pay_minor: 127_000,
        forecast_payment_minor: 127_000,
        statement_is_actual: false,
        is_closed: true,
      },
      {
        close_date: "2026-09-05",
        due_date: "2026-09-25",
        carried_opening_balance_minor: 0,
        known_charges_minor: 5_000,
        projected_variable_minor: 20_000,
        accrued_interest_minor: 0,
        statement_balance_minor: 25_000,
        minimum_due_minor: 2_500,
        full_pay_minor: 25_000,
        forecast_payment_minor: 25_000,
        statement_is_actual: false,
        is_closed: false,
      },
    ],
    stored_statements: [],
    estimate_basis: "card_history",
    estimate_sample_cycles: 11,
    ...over,
  };
}

/// A card whose leading cycle is (or is not) closed per the server's household-day flag.
function cardWithLeadingCycle(isClosed: boolean): CardStatementForecastDto {
  const base = card();
  return {
    ...base,
    cycles: base.cycles.map((c, i) => (i === 0 ? { ...c, is_closed: isClosed } : c)),
  };
}

function forecast(dueBalanceMinor: number): ForecastViewDto {
  const money = (minor_units: number) => ({ minor_units, currency: "USD" });
  return {
    currency: "USD",
    starting_balance: money(300_000),
    start_date: "2026-07-20",
    horizon_days: 90,
    days: [
      {
        date: "2026-08-25",
        closing: {
          p10: money(dueBalanceMinor),
          p50: money(dueBalanceMinor),
          p90: money(dueBalanceMinor),
        },
        events: [],
      },
    ],
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.futureCashForecast.mockResolvedValue(ok(forecast(50_000))); // $500 cash on the due date
});

test("shows utilization and the upcoming statement details", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([card()]));
  renderWithClient(<CreditCardsView />);
  expect(await screen.findByText("Visa")).toBeInTheDocument();
  expect(screen.getByText("20% utilization")).toBeInTheDocument();
  // The minimum due ($25.00) is a unique rendered value.
  expect(screen.getByText("$25.00")).toBeInTheDocument();
});

test("flags an interest risk when the projected statement exceeds projected cash on the due date", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([card()]));
  renderWithClient(<CreditCardsView />);
  // $1,270 statement > $500 projected cash → the descriptive interest-risk note.
  expect(
    await screen.findByText(/larger than your projected cash/i),
  ).toBeInTheDocument();
});

test("no interest-risk note when projected cash covers the statement", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([card()]));
  mocks.futureCashForecast.mockResolvedValue(ok(forecast(200_000))); // $2,000 > $1,270
  renderWithClient(<CreditCardsView />);
  await screen.findByText("Visa");
  expect(screen.queryByText(/larger than your projected cash/i)).not.toBeInTheDocument();
});

test("shows an empty state when there are no cards with a cycle", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([]));
  renderWithClient(<CreditCardsView />);
  expect(
    await screen.findByText(/No credit cards with a billing cycle yet/i),
  ).toBeInTheDocument();
});

// ADR 0039 addendum 2026-07-10 §1 (personal-cfo-4d8.25.2): an actual statement can only be
// recorded once the leading cycle has CLOSED (per the server's household-day flag) —
// outside the grace window the affordance is gone.
test("offers recording only when the leading cycle has closed", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([cardWithLeadingCycle(true)]));
  renderWithClient(<CreditCardsView />);
  await screen.findByText("Visa");
  expect(
    screen.getByRole("button", { name: /set actual statement balance/i }),
  ).toBeInTheDocument();
});

test("does not offer recording while the leading cycle is still open", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([cardWithLeadingCycle(false)]));
  renderWithClient(<CreditCardsView />);
  await screen.findByText("Visa");
  expect(
    screen.queryByRole("button", { name: /set actual statement balance/i }),
  ).not.toBeInTheDocument();
});

// ADR 0039 addendum 2026-07-10 §2 (personal-cfo-4d8.25.5): the estimate's signal tier is
// explained on the card.
test("explains the projected-spend estimate basis", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([card()]));
  renderWithClient(<CreditCardsView />);
  await screen.findByText("Visa");
  expect(
    screen.getByText(/projected spend from this card's transaction history \(11 cycles\)/i),
  ).toBeInTheDocument();
});

// personal-cfo-4d8.25.4: the statement-history section fetches on expand, shows derived
// import totals + recorded actuals, and records a past statement (prefilled from derived).
test("statement history expands, lists windows, and records a past statement", async () => {
  mocks.cardStatementForecast.mockResolvedValue(ok([card()]));
  mocks.setCardStatementBalance.mockResolvedValue(ok(null));
  mocks.cardStatementHistory.mockResolvedValue(
    ok([
      {
        window_open: "2026-06-05",
        close_date: "2026-07-05",
        derived_charges_minor: 123_400,
        stored_statement_minor: null,
      },
      {
        window_open: "2026-05-05",
        close_date: "2026-06-05",
        derived_charges_minor: 98_700,
        stored_statement_minor: 55_500,
      },
    ]),
  );
  renderWithClient(<CreditCardsView />);
  await screen.findByText("Visa");
  expect(mocks.cardStatementHistory).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /statement history/i }));
  expect(await screen.findByText(/\$1,234\.00 from imports/)).toBeInTheDocument();
  expect(screen.getByText("$555.00")).toBeInTheDocument();
  // Record the July window: the input prefills from the derived total; save persists it.
  fireEvent.click(
    screen.getByRole("button", { name: /record actual statement for 2026-07-05/i }),
  );
  const input = screen.getByLabelText(/actual statement for 2026-07-05/i);
  expect(input).toHaveValue("1234.00");
  fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
  await waitFor(() =>
    expect(mocks.setCardStatementBalance).toHaveBeenCalledWith(
      expect.objectContaining({
        account_id: "card-1",
        cycle_close: "2026-07-05",
        statement_balance_minor: 123_400,
      }),
    ),
  );
});

// ADR 0039 addendum 2026-07-10 §1: every stored statement row is visible, a not-applied
// (future-keyed stale) row is flagged as ignored, and each row clears independently.
test("lists recorded statements, flags a future-keyed row, and clears per row", async () => {
  mocks.setCardStatementBalance.mockResolvedValue(ok(null));
  mocks.cardStatementForecast.mockResolvedValue(
    ok([
      card({
        stored_statements: [
          { close_date: "2999-02-02", statement_balance_minor: 1_387_308, applied: false },
          { close_date: "2000-01-01", statement_balance_minor: 51_234, applied: true },
        ],
      }),
    ]),
  );
  renderWithClient(<CreditCardsView />);
  expect(await screen.findByText("Recorded statements")).toBeInTheDocument();
  expect(screen.getByText("$13,873.08")).toBeInTheDocument();
  expect(screen.getByText("$512.34")).toBeInTheDocument();
  expect(screen.getByText(/not yet closed — ignored/i)).toBeInTheDocument();
  const clears = screen.getAllByRole("button", { name: /^clear$/i });
  expect(clears).toHaveLength(2);
  fireEvent.click(clears[0]!);
  await waitFor(() =>
    expect(mocks.setCardStatementBalance).toHaveBeenCalledWith(
      expect.objectContaining({
        account_id: "card-1",
        cycle_close: "2999-02-02",
        statement_balance_minor: null,
      }),
    ),
  );
});
