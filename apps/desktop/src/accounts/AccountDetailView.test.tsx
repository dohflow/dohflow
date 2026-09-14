import { fireEvent, screen } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  AccountViewDto,
  CardStatementForecastDto,
  CashFlowHistoryDto,
  MultiSeriesForecastDto,
} from "@/bindings";
import { AccountDetailView } from "./AccountDetailView";

const mocks = vi.hoisted(() => ({
  accountList: vi.fn(),
  cashFlowHistory: vi.fn(),
  futureCashByAccount: vi.fn(),
  cardStatementForecast: vi.fn(),
  transactionPage: vi.fn(),
  assertBalance: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    accountList: mocks.accountList,
    cashFlowHistory: mocks.cashFlowHistory,
    futureCashByAccount: mocks.futureCashByAccount,
    cardStatementForecast: mocks.cardStatementForecast,
    transactionPage: mocks.transactionPage,
    assertBalance: mocks.assertBalance,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (m: number) => ({ minor_units: m, currency: "USD" });
const band = (p10: number, p50: number, p90: number) => ({
  p10: usd(p10),
  p50: usd(p50),
  p90: usd(p90),
});

const CHECKING: AccountViewDto = {
  id: "acct-checking",
  name: "Everyday Checking",
  cashflow_role: "liquid_cash",
  subtype: "checking",
  active: true,
  balance: usd(841_255),
  notes: null,
  linked_account_id: null,
  linked_account_name: null,
};
const CARD: AccountViewDto = {
  id: "acct-card",
  name: "Venture X",
  cashflow_role: "credit_facility",
  subtype: "credit_card",
  active: true,
  balance: usd(-324_872),
  notes: null,
  linked_account_id: null,
  linked_account_name: null,
};

const HISTORY: CashFlowHistoryDto = {
  currency: "USD",
  start_date: "2026-07-10",
  end_date: "2026-07-14",
  accounts: [
    {
      account_id: CHECKING.id,
      name: CHECKING.name,
      subtype: "checking",
      tier: "spendable",
      days: [
        { date: "2026-07-10", closing: usd(900_000) },
        { date: "2026-07-14", closing: usd(841_255) },
      ],
    },
    {
      account_id: CARD.id,
      name: CARD.name,
      subtype: "credit_card",
      tier: "card",
      days: [
        { date: "2026-07-10", closing: usd(-300_000) },
        { date: "2026-07-14", closing: usd(-324_872) },
      ],
    },
  ],
};

const FORECAST: MultiSeriesForecastDto = {
  currency: "USD",
  start_date: "2026-07-14",
  horizon_days: 90,
  accounts: [
    {
      account_id: CHECKING.id,
      name: CHECKING.name,
      subtype: "checking",
      tier: "spendable",
      days: [
        { date: "2026-07-14", closing: band(841_255, 841_255, 841_255), events: [] },
        {
          date: "2026-07-20",
          closing: band(700_000, 750_000, 800_000),
          events: [
            {
              source_event_id: "evt-payroll",
              name: "Payroll",
              kind: "income",
              amount: usd(318_000),
              assumption_basis: { kind: "recurring_schedule", frequency: "biweekly" },
            },
          ],
        },
      ],
    },
  ],
  groups: [],
};

const CARD_FORECAST: CardStatementForecastDto = {
  account_id: CARD.id,
  account_name: CARD.name,
  currency: "USD",
  credit_limit_minor: 1_500_000,
  repayment_philosophy: "pay_statement_balance",
  estimate_basis: "card_history",
  estimate_mape_bps: 2_000,
  estimate_sample_cycles: 6,
  stored_statements: [],
  cycles: [
    {
      close_date: "2026-07-28",
      due_date: "2026-08-17",
      carried_opening_balance_minor: 0,
      known_charges_minor: 60_000,
      projected_variable_minor: 40_000,
      accrued_interest_minor: 0,
      statement_balance_minor: 291_044,
      minimum_due_minor: 2_910,
      full_pay_minor: 291_044,
      forecast_payment_minor: 291_044,
      statement_is_actual: false,
      is_closed: false,
    },
  ],
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.accountList.mockResolvedValue(ok([CHECKING, CARD]));
  mocks.cashFlowHistory.mockResolvedValue(ok(HISTORY));
  mocks.futureCashByAccount.mockResolvedValue(ok(FORECAST));
  mocks.cardStatementForecast.mockResolvedValue(ok([CARD_FORECAST]));
  mocks.transactionPage.mockResolvedValue(
    ok({
      rows: [
        {
          transaction_id: "t1",
          account_id: CHECKING.id,
          account_name: CHECKING.name,
          occurred_at: "2026-07-13T12:00:00Z",
          transaction_date: null,
          balance_after_minor: null,
          amount: usd(-4_387),
          memo: "TRADER JOES",
          counterparty: null,
          category_id: null,
          reviewed: true,
          note: null,
          tag_ids: [],
          split_count: 0,
          category_source: null,
          category_confidence_bps: null,
        },
        {
          transaction_id: "t2",
          account_id: CHECKING.id,
          account_name: CHECKING.name,
          occurred_at: "2026-07-12T12:00:00Z",
          transaction_date: null,
          balance_after_minor: null,
          amount: usd(-10_000),
          memo: "GAS STATION",
          counterparty: null,
          category_id: null,
          reviewed: true,
          note: null,
          tag_ids: [],
          split_count: 0,
          category_source: null,
          category_confidence_bps: null,
        },
      ],
      total: 2,
    }),
  );
});

describe("AccountDetailView", () => {
  it("renders a liquid account: balance, chart, upcoming event, and posted activity with a running balance", async () => {
    renderWithClient(
      <AccountDetailView accountId={CHECKING.id} onBack={vi.fn()} />,
    );
    expect(
      await screen.findByRole("heading", { name: "Everyday Checking" }),
    ).toBeInTheDocument();
    // Headline balance (shown sign = stored for an asset); it also appears as the
    // newest row's running balance, asserted below.
    expect((await screen.findAllByText("$8,412.55")).length).toBeGreaterThanOrEqual(1);
    // The hero chart names the plotted figure.
    expect(
      await screen.findByRole("img", { name: /Balance from .* realized history/i }),
    ).toBeInTheDocument();
    // Upcoming (projected) section shows the attributed forecast event…
    expect(await screen.findByText("Payroll")).toBeInTheDocument();
    // "Upcoming" appears as the section header (and again in the chart's marker legend).
    expect(screen.getAllByText("Upcoming").length).toBeGreaterThanOrEqual(1);
    // …and the posted row carries the walk-back running balance (= current balance
    // on the newest row).
    expect(await screen.findByText("TRADER JOES")).toBeInTheDocument();
    const cells = screen.getAllByText("$8,412.55");
    expect(cells.length).toBeGreaterThanOrEqual(2); // headline + newest row balance
    // The walk actually advances: the older row's balance = newest + |newest spend|
    // (841,255 − (−4,387) = 845,642), proving the accumulation direction.
    expect(screen.getByText("GAS STATION")).toBeInTheDocument();
    expect(screen.getByText("$8,456.42")).toBeInTheDocument();
  });

  it("renders a card: positive owed headline, stats strip, and upcoming statement + payment", async () => {
    renderWithClient(<AccountDetailView accountId={CARD.id} onBack={vi.fn()} />);
    expect(
      await screen.findByRole("heading", { name: "Venture X" }),
    ).toBeInTheDocument();
    // Owed shown positive (headline; may also appear as a row running balance).
    expect((await screen.findAllByText("$3,248.72")).length).toBeGreaterThanOrEqual(1);
    // Stats strip: estimated statement + due date + utilization.
    expect(await screen.findByText(/Statement balance \(est\.\)/)).toBeInTheDocument();
    expect(screen.getByText("$2,910.44")).toBeInTheDocument();
    expect(screen.getByText(/Payment due/)).toBeInTheDocument();
    expect(screen.getByText(/22% used/)).toBeInTheDocument();
    // Upcoming rows: the statement close (est.) and the payment.
    expect(screen.getByText("Statement closes (est.)")).toBeInTheDocument();
    expect(screen.getByText("Payment")).toBeInTheDocument();
    // The chart labels the owed figure.
    expect(
      screen.getByRole("img", { name: /Amount owed from .* realized history/i }),
    ).toBeInTheDocument();
  });

  it("switches accounts with the selector", async () => {
    renderWithClient(
      <AccountDetailView accountId={CHECKING.id} onBack={vi.fn()} />,
    );
    await screen.findByRole("heading", { name: "Everyday Checking" });
    fireEvent.change(screen.getByLabelText("Switch account"), {
      target: { value: CARD.id },
    });
    expect(
      await screen.findByRole("heading", { name: "Venture X" }),
    ).toBeInTheDocument();
    expect((await screen.findAllByText("$3,248.72")).length).toBeGreaterThanOrEqual(1);
  });

  it("opens the balance editor from the Edit button", async () => {
    renderWithClient(
      <AccountDetailView accountId={CHECKING.id} onBack={vi.fn()} />,
    );
    await screen.findByRole("heading", { name: "Everyday Checking" });
    fireEvent.click(screen.getByRole("button", { name: /Edit/ }));
    // SetBalanceModal's dialog opens for this account.
    expect(
      await screen.findByRole("dialog", { name: "Set balance for Everyday Checking" }),
    ).toBeInTheDocument();
  });

  it("shows the shared empty state when the account has no posted activity", async () => {
    mocks.transactionPage.mockResolvedValue(ok({ rows: [], total: 0 }));
    renderWithClient(
      <AccountDetailView accountId={CHECKING.id} onBack={vi.fn()} />,
    );
    expect(
      await screen.findByText("No posted transactions on this account yet."),
    ).toBeInTheDocument();
    // The projected block is pinned above it and survives an empty posted list.
    expect(screen.getByText("Payroll")).toBeInTheDocument();
  });

  it("replaces the posted rows with an error rather than claiming there are none", async () => {
    // Before the DataTable migration the error rendered ABOVE the table while the body
    // still read "No posted transactions on this account yet" — which says "there are
    // none" when the truth is "we could not load them" (personal-cfo-wxy7).
    mocks.transactionPage.mockRejectedValue(new Error("nope"));
    renderWithClient(
      <AccountDetailView accountId={CHECKING.id} onBack={vi.fn()} />,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not load transactions.",
    );
    expect(
      screen.queryByText("No posted transactions on this account yet."),
    ).not.toBeInTheDocument();
  });

  it("shows a not-found state for an unknown account", async () => {
    renderWithClient(<AccountDetailView accountId="nope" onBack={vi.fn()} />);
    expect(await screen.findByText("Account not found")).toBeInTheDocument();
  });
});
