import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";

import { queryKeys } from "@/lib/query";
import { renderWithClient } from "@/test/renderWithClient";

import type {
  ForecastDayDto,
  ForecastEventDto,
  ForecastViewDto,
  MultiSeriesForecastDto,
} from "@/bindings";
import { FutureCashView } from "./FutureCashView";

const mocks = vi.hoisted(() => ({
  householdTimezone: vi.fn(),
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
  loanDoubleCountWarnings: vi.fn(),
  setHouseholdTimezone: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    householdTimezone: mocks.householdTimezone,
    setHouseholdTimezone: mocks.setHouseholdTimezone,
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
    loanDoubleCountWarnings: mocks.loanDoubleCountWarnings,
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
  mocks.householdTimezone.mockReset();
  mocks.householdTimezone.mockResolvedValue(ok("UTC"));
  mocks.loanDoubleCountWarnings.mockReset();
  mocks.loanDoubleCountWarnings.mockResolvedValue(ok([]));
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
  it("offers the forecast date reader under the projected activity (i93n3.27 slice C)", async () => {
    renderWithClient(<FutureCashView />);
    expect(await screen.findByRole("combobox", { name: "Forecast date" })).toBeInTheDocument();
    expect(screen.getByText("Read forecast by date")).toBeInTheDocument();
  // personal-cfo-i93n3.27: on a phone the chart card's controls go under its title and description instead of
  // squeezing them into a narrow column, and the page title and horizon control wrap when they do not fit.
  it("lays out its headers for a phone (i93n3.27)", async () => {
    renderWithClient(<FutureCashView />);
    const title = await screen.findByText("Liquid cash — history & projection");
    const cardHeader = title.parentElement!.parentElement!;
    expect(cardHeader).toHaveClass("compact:flex-col", "compact:items-stretch");
    expect(cardHeader.lastElementChild).toHaveClass("compact:justify-start");
    const pageHeader = screen.getByRole("heading", { name: "Cash Flow" }).parentElement!;
    expect(pageHeader).toHaveClass("flex-wrap");
  });

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

// ----- a loan's payment counted twice is shown on Cash Flow too (personal-cfo-6wk.20, AC5) -----------

const overlap = (over: Record<string, unknown> = {}) => ({
  loan_account_id: "loan-1",
  loan_name: "Test Mortgage",
  bill_event_id: "bill-1",
  bill_name: "ACME HOME LOANS",
  name_match: false,
  amount_match: true,
  strength: "strong",
  matched_by: "loan_typed",
  bill_type: "loan_payment",
  paid_from_account_name: "Checking",
  ...over,
});

test("Cash Flow shows the strong overlap, so the double count is visible where it distorts", async () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(ok([overlap()]));
  renderWithClient(<FutureCashView />);
  const region = await screen.findByRole("region", { name: "Loan payment counted twice" });
  expect(region).toHaveTextContent("Test Mortgage");
  expect(region).toHaveTextContent("ACME HOME LOANS");
});

test("Cash Flow shows a possible overlap as the soft note only", async () => {
  mocks.loanDoubleCountWarnings.mockResolvedValue(
    ok([overlap({ strength: "possible", matched_by: null })]),
  );
  renderWithClient(<FutureCashView />);
  const region = await screen.findByRole("region", { name: "Possible duplicate loan payment" });
  expect(region).not.toHaveTextContent(/counted twice/i);
  expect(screen.queryByRole("region", { name: "Loan payment counted twice" })).toBeNull();
});

test("Cash Flow warns about a linked bill that is below its loan's scheduled payment", async () => {
  mocks.recurringBillList.mockResolvedValue(
    ok([
      {
        id: "bill-1",
        name: "ACME HOME LOANS",
        bill_type: "subscription",
        amount: money(100_000),
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
        not_loan_payment: null,
        loan_link: {
          loan_account_id: "loan-1",
          loan_name: "Test Mortgage",
          in_force: true,
          cadence_changed: false,
          below_scheduled: true,
          scheduled_payment: money(150_000),
          other: null,
        },
      },
    ]),
  );
  renderWithClient(<FutureCashView />);
  const region = await screen.findByRole("region", { name: "Linked bills" });
  expect(region).toHaveTextContent("This linked bill is less than Test Mortgage's scheduled payment.");
});

test("Cash Flow shows no overlap card when there is none", async () => {
  renderWithClient(<FutureCashView />);
  await screen.findByRole("heading", { name: "Cash Flow" });
  expect(screen.queryByRole("region", { name: /loan payment/i })).toBeNull();
});


describe("failed reads can be asked for again, and say so (5ie.18)", () => {
  it("offers Retry on a failed forecast, and a successful read replaces the error", async () => {
    mocks.futureCashForecast.mockResolvedValueOnce({ status: "error", error: "engine timed out" });
    mocks.futureCashForecast.mockResolvedValue(ok(forecast()));
    renderWithClient(<FutureCashView />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(/engine timed out/i);
    const reads = mocks.futureCashForecast.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByRole("figure", { name: /projected liquid cash/i });
    expect(mocks.futureCashForecast.mock.calls.length).toBeGreaterThan(reads);
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("shows a failed breakdown as an error with Retry, never endless Loading", async () => {
    mocks.futureCashByAccount.mockResolvedValueOnce({ status: "error", error: "breakdown broke" });
    mocks.futureCashByAccount.mockResolvedValue(ok(multiSeries()));
    renderWithClient(<FutureCashView />);
    // The aggregate loaded, so the headline cards are there; the breakdown sections say why not.
    const alerts = await screen.findAllByText(/breakdown broke/i);
    expect(alerts.length).toBeGreaterThan(0);
    expect(screen.queryByText(/^Loading…$/)).toBeNull();
    expect(screen.queryByText(/projected to reach about/i)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByRole("figure", { name: /projected liquid cash/i });
    expect(screen.queryByText(/breakdown broke/i)).toBeNull();
  });
});

describe("the stack only lists what the forecast includes (5ie.18)", () => {
  const dto = (id: string, name: string, extra: object = {}) => ({
    id,
    name,
    description: null,
    status: "draft",
    created_at: "2026-06-20T00:00:00Z",
    updated_at: "2026-06-20T00:00:00Z",
    expires_on: null,
    event_count: 0,
    applied_at: null,
    ...extra,
  });

  it("drops a stacked scenario that was archived, one that expired, and one that was deleted", async () => {
    const live = [dto("p", "Primary"), dto("a", "Archived later"), dto("e", "Expires later"), dto("d", "Deleted later")];
    mocks.scenarioList.mockResolvedValue(ok(live));
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    renderWithClient(<FutureCashView scenarioId="p" onScenarioChange={vi.fn()} />, client);

    for (const name of ["Archived later", "Expires later", "Deleted later"]) {
      const picker = await screen.findByLabelText("Add a scenario to the stack");
      await within(picker).findByRole("option", { name });
      fireEvent.change(picker, { target: { value: live.find((s) => s.name === name)!.id } });
    }
    await waitFor(() =>
      expect(mocks.futureCashForecast).toHaveBeenLastCalledWith(90, ["p", "a", "e", "d"]),
    );

    // Changed elsewhere (the Scenarios tab): archived, expired, deleted.
    client.setQueryData(queryKeys.scenarios, [
      dto("p", "Primary"),
      dto("a", "Archived later", { status: "archived" }),
      dto("e", "Expires later", { expires_on: "2001-01-01" }),
    ]);

    await waitFor(() =>
      expect(mocks.futureCashForecast).toHaveBeenLastCalledWith(90, ["p"]),
    );
    expect(screen.queryByRole("button", { name: /Remove .* from the stack/ })).toBeNull();
  });
});

describe("a failed refresh outranks the breakdown it replaces (5ie.18)", () => {
  it("shows the failure with one Retry, drops the cached figures and Cover it, and a good read brings them back", async () => {
    const short = multiSeries();
    short.accounts[0]!.days = [day("2026-06-25", 400_000), day("2026-07-01", -50_000)];
    mocks.futureCashByAccount.mockResolvedValue(ok(short));
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    renderWithClient(<FutureCashView />, client);
    await screen.findByRole("figure", { name: /projected liquid cash/i });
    expect(screen.getByRole("button", { name: /cover it/i })).toBeInTheDocument();
    expect(screen.getByRole("table")).toBeInTheDocument();

    // The next read of the breakdown fails while the earlier one is still cached.
    mocks.futureCashByAccount.mockResolvedValue({ status: "error", error: "breakdown broke" });
    await act(async () => {
      await client.refetchQueries({ queryKey: queryKeys.forecastByAccount(90, "") });
    });
    const alerts = await screen.findAllByText(/breakdown broke/i);
    expect(alerts.length).toBeGreaterThan(0);
    expect(screen.getAllByRole("button", { name: "Retry" })).toHaveLength(1);
    expect(screen.queryByRole("figure", { name: /projected liquid cash/i })).toBeNull();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByRole("button", { name: /cover it/i })).toBeNull();

    mocks.futureCashByAccount.mockResolvedValue(ok(short));
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByRole("figure", { name: /projected liquid cash/i });
    expect(screen.getByRole("button", { name: /cover it/i })).toBeInTheDocument();
    expect(screen.queryByText(/breakdown broke/i)).toBeNull();
  });
});

describe("scenario selections wait for the household date (5ie.18, AC8)", () => {
  // 2026-09-02 00:30 UTC is still September 1 in Los Angeles: a scenario that expires
  // September 1 is live there and expired in UTC.
  const LA = "America/Los_Angeles";
  const dto = (id: string, name: string, extra: object = {}) => ({
    id,
    name,
    description: null,
    status: "draft",
    created_at: "2026-06-20T00:00:00Z",
    updated_at: "2026-06-20T00:00:00Z",
    expires_on: null,
    event_count: 0,
    applied_at: null,
    ...extra,
  });
  const scenarios = () => [dto("p", "Primary"), dto("s", "Ends Sep 1", { expires_on: "2026-09-01" })];

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-09-02T00:30:00Z"));
    mocks.householdTimezone.mockResolvedValue(ok(LA));
    mocks.scenarioList.mockResolvedValue(ok(scenarios()));
    mocks.setHouseholdTimezone.mockReset();
  });
  afterEach(() => vi.useRealTimers());

  async function stackedView() {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const onScenarioChange = vi.fn();
    renderWithClient(<FutureCashView scenarioId="p" onScenarioChange={onScenarioChange} />, client);
    const picker = await screen.findByLabelText("Add a scenario to the stack");
    await within(picker).findByRole("option", { name: "Ends Sep 1" });
    fireEvent.change(picker, { target: { value: "s" } });
    await waitFor(() => expect(mocks.futureCashForecast).toHaveBeenLastCalledWith(90, ["p", "s"]));
    // With the date known, the pile that explains the order is there too.
    expect(await screen.findByRole("list", { name: /scenario stack/i })).toBeInTheDocument();
    return { client, onScenarioChange };
  }

  /// The household timezone read starts over (as after a vault lock and unlock), still unread.
  function forgetTimezone(client: QueryClient, next: () => Promise<unknown>) {
    // A factory, so a rejection is created when the read is made, not before anything handles it.
    mocks.householdTimezone.mockImplementation(next);
    return act(async () => {
      client.removeQueries({ queryKey: queryKeys.householdTimezone });
      // Any change to what the view reads makes it build the (now empty) timezone query again.
      client.setQueryData(queryKeys.scenarios, [
        dto("p", "Primary", { updated_at: "2026-06-21T00:00:00Z" }),
        dto("s", "Ends Sep 1", { expires_on: "2026-09-01" }),
      ]);
    });
  }

  it("keeps the primary and the stacked ids while the timezone has not been read, and still has them when Los Angeles arrives", async () => {
    const { client, onScenarioChange } = await stackedView();
    let resolveTz: (value: unknown) => void = () => {};
    await forgetTimezone(
      client,
      () => new Promise((resolve) => (resolveTz = resolve)),
    );
    // Nothing is pruned or reselected on the UTC fallback; no eligibility is claimed.
    await waitFor(() =>
      expect(screen.queryByRole("list", { name: /scenario stack/i })).toBeNull(),
    );
    expect(screen.queryByLabelText("Add a scenario to the stack")).toBeNull();
    expect(mocks.futureCashForecast).not.toHaveBeenLastCalledWith(90, ["p"]);
    expect(onScenarioChange).not.toHaveBeenCalled();

    await act(async () => resolveTz(ok(LA)));
    await screen.findByRole("button", { name: /Remove Ends Sep 1 from the stack/ });
    expect(mocks.futureCashForecast).toHaveBeenLastCalledWith(90, ["p", "s"]);
    expect(onScenarioChange).not.toHaveBeenCalled();
  });

  it("keeps them when the read failed, and a Retry that reads Los Angeles brings the stack back without a write", async () => {
    const { client, onScenarioChange } = await stackedView();
    await forgetTimezone(client, () => Promise.reject(new Error("host down")));
    expect((await screen.findAllByText("Could not load the household timezone."))[0]).toBeInTheDocument();
    expect(mocks.futureCashForecast).not.toHaveBeenLastCalledWith(90, ["p"]);

    mocks.householdTimezone.mockResolvedValue(ok(LA));
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await screen.findByRole("button", { name: /Remove Ends Sep 1 from the stack/ });
    expect(mocks.setHouseholdTimezone).not.toHaveBeenCalled();
    expect(mocks.futureCashForecast).toHaveBeenLastCalledWith(90, ["p", "s"]);
    expect(onScenarioChange).not.toHaveBeenCalled();
  });

  it("a timezone read as UTC is a real answer: the scenario that ended is dropped", async () => {
    const { client } = await stackedView();
    await forgetTimezone(client, () => Promise.resolve(ok("UTC")));
    await waitFor(() => expect(mocks.futureCashForecast).toHaveBeenLastCalledWith(90, ["p"]));
    expect(screen.queryByRole("button", { name: /Remove Ends Sep 1 from the stack/ })).toBeNull();
  });

  it("after a good read, a stacked scenario that was archived or deleted is still dropped", async () => {
    const { client } = await stackedView();
    client.setQueryData(queryKeys.scenarios, [dto("p", "Primary")]);
    await waitFor(() => expect(mocks.futureCashForecast).toHaveBeenLastCalledWith(90, ["p"]));
  });
});
