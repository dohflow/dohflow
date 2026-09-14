import { fireEvent, screen, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { DebtPayoffPlanDto } from "@/bindings";
import { DebtPayoffCompare } from "./DebtPayoffCompare";

const mocks = vi.hoisted(() => ({
  debtPayoffComparison: vi.fn(),
  baseCurrency: vi.fn(),
  createScenario: vi.fn(),
  createForecastAssumption: vi.fn(),
  deleteScenario: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    debtPayoffComparison: mocks.debtPayoffComparison,
    baseCurrency: mocks.baseCurrency,
    createScenario: mocks.createScenario,
    createForecastAssumption: mocks.createForecastAssumption,
    deleteScenario: mocks.deleteScenario,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function plan(over: Partial<DebtPayoffPlanDto> = {}): DebtPayoffPlanDto {
  return {
    strategy: "minimum_only",
    debt_free_month: 24,
    total_interest_minor: 50_000,
    currency: "USD",
    monthly_total_owed_minor: [],
    per_debt: [],
    ...over,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.baseCurrency.mockResolvedValue(ok("USD"));
});

test("compares the strategies with debt-free duration and total interest", async () => {
  mocks.debtPayoffComparison.mockResolvedValue(
    ok([
      plan({ strategy: "minimum_only", debt_free_month: null, total_interest_minor: 300_000 }),
      plan({ strategy: "snowball", debt_free_month: 38, total_interest_minor: 120_000 }),
      plan({ strategy: "avalanche", debt_free_month: 36, total_interest_minor: 110_000 }),
    ]),
  );
  renderWithClient(<DebtPayoffCompare />);

  // The table is on the shared DataTable now, so it exists from first paint (with
  // skeleton rows) — awaiting the table itself would no longer wait for the data. Await
  // a row, THEN scope: the strategy labels also appear in the per-debt selector.
  await screen.findAllByText("Minimum only");
  const table = screen.getByRole("table");
  expect(within(table).getByText("Minimum only")).toBeInTheDocument();
  expect(within(table).getByText("Snowball")).toBeInTheDocument();
  expect(within(table).getByText("Avalanche")).toBeInTheDocument();
  // A minimum-only baseline that never amortizes within the 50-year horizon.
  expect(screen.getByText("Not within 50 years")).toBeInTheDocument();
  // 38 months → "3 yr 2 mo"; 36 → "3 yr".
  expect(screen.getByText("3 yr 2 mo")).toBeInTheDocument();
  expect(screen.getByText("3 yr")).toBeInTheDocument();
  // Total interest formatted in the base currency (avalanche's $1,100.00 is unique).
  expect(screen.getByText("$1,100.00")).toBeInTheDocument();
});

test("distinguishes a plan that clears at the 50-year cap from one that never clears", async () => {
  // debt_free_month 600 (cleared at the horizon cap) → "50 yr"; null (never clears) →
  // "Not within 50 years" — the two must render distinctly (review boundary).
  mocks.debtPayoffComparison.mockResolvedValue(
    ok([
      plan({ strategy: "minimum_only", debt_free_month: null, total_interest_minor: 900_000 }),
      plan({ strategy: "avalanche", debt_free_month: 600, total_interest_minor: 800_000 }),
    ]),
  );
  renderWithClient(<DebtPayoffCompare />);
  expect(await screen.findByText("Not within 50 years")).toBeInTheDocument();
  expect(screen.getByText("50 yr")).toBeInTheDocument();
});

test("renders the burndown chart alongside the table when trajectories are present", async () => {
  // The shipped path: non-empty trajectories → the chart renders its legend, so each strategy
  // label appears in BOTH the chart legend and the table (distinct figure vs table regions).
  mocks.debtPayoffComparison.mockResolvedValue(
    ok([
      plan({
        strategy: "minimum_only",
        debt_free_month: null,
        monthly_total_owed_minor: [700_000, 690_000, 680_000],
      }),
      plan({
        strategy: "snowball",
        debt_free_month: 2,
        monthly_total_owed_minor: [700_000, 350_000, 0],
      }),
      plan({
        strategy: "avalanche",
        debt_free_month: 2,
        monthly_total_owed_minor: [700_000, 330_000, 0],
      }),
    ]),
  );
  renderWithClient(<DebtPayoffCompare />);

  const figure = await screen.findByRole("figure", { name: /debt owed over time/i });
  expect(figure).toBeInTheDocument();
  const table = screen.getByRole("table");
  // "Avalanche" appears in the table, the burndown legend, and the per-debt selector.
  expect(screen.getAllByText("Avalanche")).toHaveLength(3);
  expect(within(table).getByText("Snowball")).toBeInTheDocument();
  // The per-debt breakdown section is present.
  expect(screen.getByText("Per debt")).toBeInTheDocument();
});

test("shows an empty state when there is no carry-debt", async () => {
  mocks.debtPayoffComparison.mockResolvedValue(ok([]));
  renderWithClient(<DebtPayoffCompare />);
  expect(
    await screen.findByText(/No revolving debt to plan a payoff/i),
  ).toBeInTheDocument();
});

test("surfaces a failed comparison in the table rather than beside it", async () => {
  // The error treatment moved onto the shared DataTable (personal-cfo-wxy7): it now
  // replaces the rows and carries role="alert", instead of a bespoke text-destructive
  // line above a table that did not render at all.
  mocks.debtPayoffComparison.mockRejectedValue(new Error("nope"));
  renderWithClient(<DebtPayoffCompare />);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not load your debt-payoff comparison.",
  );
  // Not an empty vault — never tell the user they have no debt when the load failed.
  expect(
    screen.queryByText(/No revolving debt to plan a payoff/i),
  ).not.toBeInTheDocument();
});

test("adds the entered extra payment to Cash Flow as a scenario", async () => {
  mocks.debtPayoffComparison.mockResolvedValue(ok([plan()]));
  mocks.createScenario.mockResolvedValue(
    ok({
      id: "scen-1",
      name: "Extra $300.00/mo toward debt",
      description: null,
      status: "draft",
      created_at: "2026-07-01",
    }),
  );
  mocks.createForecastAssumption.mockResolvedValue(ok(null));
  renderWithClient(<DebtPayoffCompare />);

  const input = await screen.findByLabelText("Extra toward debt each month");
  fireEvent.change(input, { target: { value: "300" } });
  fireEvent.click(screen.getByRole("button", { name: /add to cash flow/i }));

  // A confirmation appears; the overlay carries the $300 extra as a recurring_debt_payment.
  expect(await screen.findByText(/open Cash Flow to compare/i)).toBeInTheDocument();
  expect(mocks.createScenario).toHaveBeenCalledWith(
    expect.objectContaining({ name: expect.stringContaining("Extra") }),
  );
  expect(mocks.createForecastAssumption).toHaveBeenCalledWith(
    expect.objectContaining({
      kind: "recurring_debt_payment",
      scenario_id: "scen-1",
      amount: expect.objectContaining({ minor_units: 30_000 }),
    }),
  );
});

test("the add-to-Cash-Flow button is disabled without an extra amount", async () => {
  mocks.debtPayoffComparison.mockResolvedValue(ok([plan()]));
  renderWithClient(<DebtPayoffCompare />);
  // The default extra is 0 → nothing to project, so the action is disabled.
  expect(
    await screen.findByRole("button", { name: /add to cash flow/i }),
  ).toBeDisabled();
});
