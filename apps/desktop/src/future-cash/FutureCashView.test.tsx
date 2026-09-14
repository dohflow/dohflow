import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type {
  ForecastDayDto,
  ForecastEventDto,
  ForecastViewDto,
  MultiSeriesForecastDto,
} from "@/bindings";
import { FutureCashView } from "./FutureCashView";

const mocks = vi.hoisted(() => ({
  futureCashForecast: vi.fn(),
  futureCashByAccount: vi.fn(),
  manualFutureEntryList: vi.fn(),
  createManualFutureEntry: vi.fn(),
  scenarioList: vi.fn(),
  createScenario: vi.fn(),
  deleteScenario: vi.fn(),
  forecastAssumptionList: vi.fn(),
  createForecastAssumption: vi.fn(),
  deleteForecastAssumption: vi.fn(),
  recurringBillList: vi.fn(),
  incomeSourceList: vi.fn(),
  cashFlowHistory: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    futureCashForecast: mocks.futureCashForecast,
    futureCashByAccount: mocks.futureCashByAccount,
    cashFlowHistory: mocks.cashFlowHistory,
    manualFutureEntryList: mocks.manualFutureEntryList,
    createManualFutureEntry: mocks.createManualFutureEntry,
    scenarioList: mocks.scenarioList,
    createScenario: mocks.createScenario,
    deleteScenario: mocks.deleteScenario,
    forecastAssumptionList: mocks.forecastAssumptionList,
    createForecastAssumption: mocks.createForecastAssumption,
    deleteForecastAssumption: mocks.deleteForecastAssumption,
    recurringBillList: mocks.recurringBillList,
    incomeSourceList: mocks.incomeSourceList,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function money(minorUnits: number) {
  return { minor_units: minorUnits, currency: "USD" };
}

function event(
  name: string,
  kind: string,
  minorUnits: number,
): ForecastEventDto {
  return {
    source_event_id: `evt-${name}-${minorUnits}`,
    name,
    kind,
    amount: money(minorUnits),
    assumption_basis: { kind: "recurring_schedule", frequency: "monthly" },
  };
}

// A collapsed band (P10 == P50 == P90) — deterministic Layer-1 data.
function day(
  date: string,
  closingMinor: number,
  events: ForecastEventDto[] = [],
): ForecastDayDto {
  const closing = money(closingMinor);
  return { date, closing: { p10: closing, p50: closing, p90: closing }, events };
}

const SAMPLE_DAYS = (): ForecastDayDto[] => [
  day("2026-06-20", 250_000),
  day("2026-06-25", 400_000, [event("Acme Corp", "income", 150_000)]),
  day("2026-07-01", 220_000, [event("Rent", "recurring_bill", -180_000)]),
  day("2026-07-10", 215_000, [event("Phone", "recurring_bill", -5_000)]),
  day("2026-07-19", 365_000, [event("Acme Corp", "income", 150_000)]),
];

function forecast(over: Partial<ForecastViewDto> = {}): ForecastViewDto {
  return {
    currency: "USD",
    starting_balance: money(250_000),
    start_date: "2026-06-20",
    horizon_days: 90,
    days: SAMPLE_DAYS(),
    ...over,
  };
}

// The per-account/per-group projection backing the chart + table — one Checking
// account (spendable) carrying the events, plus the group rollups.
function multiSeries(
  over: Partial<MultiSeriesForecastDto> = {},
): MultiSeriesForecastDto {
  const days = SAMPLE_DAYS();
  const closings = days.map((d) => ({ date: d.date, closing: d.closing }));
  const zeros = days.map((d) => ({ date: d.date, closing: money(0) })).map((c) => ({
    date: c.date,
    closing: { p10: c.closing, p50: c.closing, p90: c.closing },
  }));
  return {
    currency: "USD",
    start_date: "2026-06-20",
    horizon_days: 90,
    accounts: [
      {
        account_id: "acc-1",
        name: "Checking",
        subtype: "checking",
        tier: "spendable",
        days,
      },
    ],
    groups: [
      { tier: "spendable", closings },
      { tier: "reserve", closings: zeros },
      { tier: "net", closings },
    ],
    ...over,
  };
}

beforeEach(() => {
  mocks.futureCashForecast.mockReset();
  mocks.futureCashForecast.mockResolvedValue(ok(forecast()));
  mocks.futureCashByAccount.mockReset();
  mocks.futureCashByAccount.mockResolvedValue(ok(multiSeries()));
  mocks.manualFutureEntryList.mockReset();
  mocks.manualFutureEntryList.mockResolvedValue(ok([]));
  mocks.createManualFutureEntry.mockReset();
  mocks.scenarioList.mockReset();
  mocks.scenarioList.mockResolvedValue(ok([]));
  mocks.createScenario.mockReset();
  mocks.deleteScenario.mockReset();
  mocks.forecastAssumptionList.mockReset();
  mocks.forecastAssumptionList.mockResolvedValue(ok([]));
  mocks.createForecastAssumption.mockReset();
  mocks.deleteForecastAssumption.mockReset();
  mocks.recurringBillList.mockReset();
  mocks.recurringBillList.mockResolvedValue(ok([]));
  mocks.incomeSourceList.mockReset();
  mocks.incomeSourceList.mockResolvedValue(ok([]));
  mocks.cashFlowHistory.mockReset();
  mocks.cashFlowHistory.mockResolvedValue(
    ok({
      currency: "USD",
      start_date: "2026-06-01",
      end_date: "2026-06-20",
      accounts: [],
    }),
  );
});

describe("FutureCashView", () => {
  it("shows a loading state until the forecast resolves", () => {
    mocks.futureCashForecast.mockReturnValue(new Promise(() => {}));
    renderWithClient(<FutureCashView />);
    expect(screen.getByText(/loading your forecast/i)).toBeInTheDocument();
  });

  it("renders the chart, key balances, and the projected-activity table", async () => {
    renderWithClient(<FutureCashView />);
    // The accessible chart renders once the forecast resolves (collapsed band).
    expect(
      await screen.findByRole("figure", { name: /projected liquid cash/i }),
    ).toBeInTheDocument();
    // Balance summary cards.
    expect(screen.getByText("Liquid cash today")).toBeInTheDocument();
    expect(screen.getByText(/Lowest projected/)).toBeInTheDocument();
    expect(screen.getByText("Projected at horizon end")).toBeInTheDocument();
    expect(screen.getAllByText("$2,500.00").length).toBeGreaterThanOrEqual(1);
    // The ledger lists the projected income and bills.
    expect(screen.getByText("Rent")).toBeInTheDocument();
    expect(screen.getByText("Phone")).toBeInTheDocument();
    expect(screen.getAllByText("Acme Corp")).toHaveLength(2);
  });

  it("opens on the 3-month horizon and re-queries when it changes", async () => {
    renderWithClient(<FutureCashView />);
    await screen.findByRole("figure", { name: /projected liquid cash/i });
    expect(mocks.futureCashForecast).toHaveBeenCalledWith(90, []);

    fireEvent.click(
      within(screen.getByRole("group", { name: "Forecast horizon" })).getByRole(
        "button",
        { name: "1Y" },
      ),
    );
    await waitFor(() =>
      expect(mocks.futureCashForecast).toHaveBeenCalledWith(365, []),
    );
  });

  it("reveals a typed explanation when a table row is clicked (vkge)", async () => {
    renderWithClient(<FutureCashView />);
    await screen.findByRole("figure", { name: /projected liquid cash/i });

    const rentRow = screen.getByRole("button", { name: /Rent/ });
    expect(rentRow).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(rentRow);
    expect(rentRow).toHaveAttribute("aria-expanded", "true");
    // The typed explanation blocks appear (never raw HTML/Markdown).
    expect(screen.getByText("Basis")).toBeInTheDocument();
    expect(screen.getByText("Recurring schedule · Monthly")).toBeInTheDocument();

    // Clicking again collapses it.
    fireEvent.click(rentRow);
    expect(rentRow).toHaveAttribute("aria-expanded", "false");
  });

  it("refetches the forecast when a manual entry is added (q6gh cross-cutting)", async () => {
    mocks.createManualFutureEntry.mockResolvedValue(
      ok({
        id: "new",
        amount: { minor_units: 500_000, currency: "USD" },
        date: "2026-08-01",
        label: "Bonus",
      }),
    );
    renderWithClient(<FutureCashView />);
    await screen.findByRole("figure", { name: /projected liquid cash/i });
    const before = mocks.futureCashForecast.mock.calls.length;

    // Add a future entry from the view (toggle → fill → submit).
    fireEvent.click(screen.getByRole("button", { name: /add entry/i }));
    fireEvent.change(screen.getByLabelText("Label"), {
      target: { value: "Bonus" },
    });
    fireEvent.change(screen.getByLabelText(/Amount/), {
      target: { value: "5000" },
    });
    fireEvent.click(screen.getByRole("button", { name: /add entry/i }));

    // The mutation invalidates ["forecast"], so the chart's query refetches.
    await waitFor(() =>
      expect(mocks.createManualFutureEntry).toHaveBeenCalled(),
    );
    await waitFor(() =>
      expect(mocks.futureCashForecast.mock.calls.length).toBeGreaterThan(before),
    );
  });

  it("switches to a scenario and shows the compare-vs-base delta (6zep)", async () => {
    mocks.scenarioList.mockResolvedValue(
      ok([
        {
          id: "scn-1",
          name: "Raise",
          description: null,
          status: "draft",
          created_at: "2026-06-20T00:00:00Z",
        },
      ]),
    );
    // Base ends at 365_000; the scenario folds in a $5,000 bonus and ends higher.
    // Branch on LENGTH, not truthiness: the selection is now an array and `[]` (base) is
    // truthy, so a truthiness check would hand the base call the scenario's forecast and
    // silently collapse the compare delta to zero.
    mocks.futureCashForecast.mockImplementation((_horizon, scenarioIds) =>
      Promise.resolve(
        ok(
          scenarioIds.length > 0
            ? forecast({
                days: [
                  day("2026-06-20", 250_000),
                  day("2026-06-25", 400_000, [
                    event("Acme Corp", "income", 150_000),
                  ]),
                  day("2026-07-01", 220_000, [
                    event("Rent", "recurring_bill", -180_000),
                  ]),
                  day("2026-07-10", 215_000, [
                    event("Phone", "recurring_bill", -5_000),
                  ]),
                  day("2026-07-19", 865_000, [
                    event("Bonus", "manual_entry", 500_000),
                  ]),
                ],
              })
            : forecast(),
        ),
      ),
    );

    renderWithClient(<FutureCashView />);
    await screen.findByRole("figure", { name: /projected liquid cash/i });
    // No scenario selected → no compare banner yet.
    expect(screen.queryByText(/compared to base/i)).not.toBeInTheDocument();

    // Select the scenario (wait for its option to load first).
    await screen.findByRole("option", { name: "Raise" });
    fireEvent.change(screen.getByLabelText("Scenario"), {
      target: { value: "scn-1" },
    });

    // The scenario forecast is fetched, the compare delta appears, and the
    // per-scenario "what-if changes" card mounts.
    await waitFor(() =>
      expect(mocks.futureCashForecast).toHaveBeenCalledWith(90, ["scn-1"]),
    );
    const banner = await screen.findByRole("status");
    expect(banner).toHaveTextContent("Compared to base");
    expect(within(banner).getByText("+$5,000.00")).toBeInTheDocument();
    expect(screen.getByText(/what-if changes/i)).toBeInTheDocument();
  });

  it("re-queries the realized history when the lookback changes (4d8.27.5.3)", async () => {
    renderWithClient(<FutureCashView />);
    await screen.findByRole("figure");
    // Default lookback 3M = 90 days.
    expect(mocks.cashFlowHistory).toHaveBeenCalledWith(90);
    const picker = screen.getByRole("group", { name: "History lookback" });
    fireEvent.click(within(picker).getByRole("button", { name: "1Y" }));
    await waitFor(() => expect(mocks.cashFlowHistory).toHaveBeenCalledWith(365));
  });
});

describe("the three states are deliberate (personal-cfo-4fbl)", () => {
  it("says the vault is still readable when the forecast fails", () => {
    // The forecast is a COMPUTED read over the ledger, so it can fail while every account
    // behind it is fine. Without saying so, the reader can't tell a failed projection from
    // a damaged vault — the difference between "try again" and "restore from backup".
    mocks.futureCashForecast.mockResolvedValue({
      status: "error",
      error: "engine timed out",
    });
    renderWithClient(<FutureCashView />);

    return screen.findByRole("alert").then((alert) => {
      expect(alert).toHaveTextContent(/vault is readable/i);
      expect(alert).toHaveTextContent(/accounts and transactions are unaffected/i);
      // The underlying cause is still named, not swallowed.
      expect(alert).toHaveTextContent(/engine timed out/i);
    });
  });

  it("names the missing ANCHOR rather than saying no data", async () => {
    mocks.futureCashByAccount.mockResolvedValue(ok(multiSeries({ accounts: [] })));
    renderWithClient(<FutureCashView />);

    // Twice on the page: the chart states it fully, Projected Activity states it briefly.
    expect(await screen.findAllByText(/no account to project from/i)).toHaveLength(2);
    // The actionable detail appears ONCE — repeating it would read as two problems.
    expect(screen.getByText(/add a checking or savings account/i)).toBeInTheDocument();
    expect(screen.queryByText(/no data/i)).not.toBeInTheDocument();
  });

  it("names the missing SCHEDULE when the vault is anchored but nothing is expected", async () => {
    const flat = SAMPLE_DAYS().map((d) => ({ ...d, events: [] }));
    mocks.futureCashForecast.mockResolvedValue(ok(forecast({ days: flat })));
    renderWithClient(<FutureCashView />);

    expect(await screen.findAllByText(/nothing scheduled to project/i)).toHaveLength(2);
    expect(screen.getByText(/add your income and a bill/i)).toBeInTheDocument();
  });

  it("does NOT show an empty state on the ordinary populated path", async () => {
    // Guards the guard: the classifier sits in front of the chart, so a wrong condition
    // would replace a perfectly good forecast with a "go add an account" message.
    renderWithClient(<FutureCashView />);

    await screen.findByRole("figure");
    expect(screen.queryByText(/no account to project from/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/nothing scheduled to project/i)).not.toBeInTheDocument();
  });
});
