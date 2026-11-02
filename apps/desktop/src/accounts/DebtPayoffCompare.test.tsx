import { fireEvent, screen, waitFor, within } from "@testing-library/react";

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

describe("the result of adding the scenario is announced (6wk.26)", () => {
  const SAVED = {
    id: "scen-1",
    name: "Extra $300.00/mo toward debt",
    description: null,
    status: "draft",
    created_at: "2026-07-01",
  };
  const ADDED = "Added “Extra $300.00/mo toward debt” — open Cash Flow to compare it against your base forecast.";

  async function addExtra() {
    mocks.debtPayoffComparison.mockResolvedValue(ok([plan()]));
    renderWithClient(<DebtPayoffCompare />);
    const input = await screen.findByLabelText("Extra toward debt each month");
    fireEvent.change(input, { target: { value: "300" } });
    const button = screen.getByRole("button", { name: /add to cash flow/i });
    button.focus();
    fireEvent.click(button);
    return button;
  }

  test("a polite status region is there, empty, before any result", async () => {
    mocks.debtPayoffComparison.mockResolvedValue(ok([plan()]));
    renderWithClient(<DebtPayoffCompare />);
    expect(await screen.findByRole("status")).toBeEmptyDOMElement();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("a success is announced politely, with no alert, and focus stays put", async () => {
    mocks.createScenario.mockResolvedValue(ok(SAVED));
    mocks.createForecastAssumption.mockResolvedValue(ok(null));
    const button = await addExtra();
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(ADDED));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(button).toHaveFocus();
  });

  test("the same region keeps its node, so the new text is what a screen reader hears", async () => {
    mocks.createScenario.mockResolvedValue(ok(SAVED));
    mocks.createForecastAssumption.mockResolvedValue(ok(null));
    mocks.debtPayoffComparison.mockResolvedValue(ok([plan()]));
    renderWithClient(<DebtPayoffCompare />);
    const region = await screen.findByRole("status");
    fireEvent.change(await screen.findByLabelText("Extra toward debt each month"), {
      target: { value: "300" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add to cash flow/i }));
    await waitFor(() => expect(region).toHaveTextContent(ADDED));
    expect(screen.getByRole("status")).toBe(region);
  });

  test("adding the same extra again announces again: the text clears, then returns", async () => {
    mocks.createScenario.mockResolvedValue(ok(SAVED));
    mocks.createForecastAssumption.mockResolvedValue(ok(null));
    await addExtra();
    const region = await screen.findByRole("status");
    await waitFor(() => expect(region).toHaveTextContent(ADDED));

    const seen: string[] = [];
    const observer = new MutationObserver(() => seen.push(region.textContent ?? ""));
    observer.observe(region, { childList: true, characterData: true, subtree: true });
    fireEvent.click(screen.getByRole("button", { name: /add to cash flow/i }));
    await waitFor(() => expect(mocks.createScenario).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(seen).toContain(""));
    await waitFor(() => expect(region).toHaveTextContent(ADDED));
    observer.disconnect();
  });

  test("a refusal is one persistent alert with the app's reason, and no status", async () => {
    mocks.createScenario.mockResolvedValue({
      status: "error",
      error: { Validation: "Pick a name." },
    });
    const button = await addExtra();
    expect(await screen.findByRole("alert")).toHaveTextContent("Pick a name.");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
    expect(button).toHaveFocus();
  });

  test("a call that threw is one alert with the existing sentence", async () => {
    mocks.createScenario.mockRejectedValue(new Error("connection lost"));
    await addExtra();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not add the scenario to Cash Flow. Please try again.",
    );
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByRole("status")).toBeEmptyDOMElement();
  });

  test("a result stays until the amount changes, and a later failure replaces a success in the other region", async () => {
    mocks.createScenario.mockResolvedValueOnce(ok(SAVED));
    mocks.createForecastAssumption.mockResolvedValue(ok(null));
    await addExtra();
    expect(await screen.findByRole("status")).toHaveTextContent(ADDED);

    mocks.createScenario.mockRejectedValueOnce(new Error("connection lost"));
    fireEvent.click(screen.getByRole("button", { name: /add to cash flow/i }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("status")).toBeEmptyDOMElement();

    fireEvent.change(screen.getByLabelText("Extra toward debt each month"), {
      target: { value: "310" },
    });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
