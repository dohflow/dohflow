import { fireEvent, screen, waitFor, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";
import type {
  ForecastDayDto,
  ForecastEventDto,
  ForecastViewDto,
  MultiSeriesForecastDto,
} from "@/bindings";
import { UnlockedHome } from "@/vault/screens/UnlockedHome";
import { SessionViewStateProvider } from "@/lib/SessionViewStateProvider";
import { useSessionState } from "@/lib/sessionViewState";
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


// The shell around the views (the same stubs the shell's own tests use).
vi.mock("@/accounts/useAccounts", () => ({ useAccounts: () => ({ accounts: [{ id: "acc-1" }] }) }));
vi.mock("@/money-inbox/useMoneyInbox", () => ({ useMoneyInbox: () => ({ items: [] }) }));
vi.mock("@/vault/useVault", () => ({ useVault: () => ({ lockVault: vi.fn() }) }));
vi.mock("@/backup/BackupNudge", () => ({ BackupNudge: () => null }));
vi.mock("@/dashboard/DashboardView", () => ({ DashboardView: () => <h2>Dashboard</h2> }));
vi.mock("@/settings/SettingsView", () => ({ SettingsView: () => <h2>Settings</h2> }));
vi.mock("@/settings/BuildBadge", () => ({ BuildBadge: () => null }));
vi.mock("@/settings/UpdateAvailableNotice", () => ({ UpdateAvailableNotice: () => null }));
vi.mock("@/forecast-activation/CapabilityUnlockNotice", () => ({ CapabilityUnlockNotice: () => null }));
vi.mock("@/settings/connections/ConnectionAttentionNotice", () => ({ ConnectionAttentionNotice: () => null }));

/// The activity table's "Accounts" column picker, not the sidebar's Accounts destination.
const columnsTrigger = () =>
  screen.getAllByRole("button", { name: /^accounts/i }).find((b) => b.hasAttribute("aria-expanded"))!;

async function goTo(name: RegExp) {
  fireEvent.click(screen.getByRole("button", { name }));
}

describe("Future Cash view state survives a visit to another destination (personal-cfo-i93n3.27 slice D)", () => {
  afterEach(() => vi.restoreAllMocks());

  it("keeps the horizon, the lookback, the activity search, the type filter and the account columns in the real shell", async () => {
    renderWithClient(<UnlockedHome />);
    await goTo(/^Cash Flow$/);
    await screen.findByRole("heading", { name: "Cash Flow" });
    await screen.findByRole("group", { name: "History lookback" }, { timeout: 5000 });
    // A change re-reads the forecast and the view shows a loading state meanwhile: wait for each control.
    const horizonGroup = async () => within(await screen.findByRole("group", { name: "Forecast horizon" }, { timeout: 5000 }));
    const lookbackGroup = async () => within(await screen.findByRole("group", { name: "History lookback" }, { timeout: 5000 }));
    fireEvent.click((await horizonGroup()).getByRole("button", { name: "1Y" }));
        fireEvent.click((await lookbackGroup()).getByRole("button", { name: "6M" }));
    const search = await screen.findByRole("searchbox");
    fireEvent.change(search, { target: { value: "acme" } });
    fireEvent.change(screen.getByRole("combobox", { name: /type|all types/i }), { target: { value: "income" } });
    fireEvent.click(columnsTrigger());
    fireEvent.click(within(screen.getByRole("group", { name: "Account columns" })).getByLabelText("Checking"));

    await goTo(/^Dashboard$/);
    await screen.findByRole("heading", { name: "Dashboard" });
    await goTo(/^Cash Flow$/);

    await screen.findByRole("group", { name: "History lookback" }, { timeout: 5000 });
    expect((await horizonGroup()).getByRole("button", { name: "1Y" })).toHaveAttribute("aria-pressed", "true");
    expect((await lookbackGroup()).getByRole("button", { name: "6M" })).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByRole("searchbox")).toHaveValue("acme");
    expect(screen.getByRole("combobox", { name: /type|all types/i })).toHaveValue("income");
    expect(columnsTrigger()).toHaveTextContent("1");
  });

  it("starts empty again when the shell is mounted afresh (what a lock or a vault switch does)", async () => {
    const first = renderWithClient(<UnlockedHome />);
    await goTo(/^Cash Flow$/);
    await screen.findByRole("heading", { name: "Cash Flow" });
    fireEvent.click(within(screen.getByRole("group", { name: "Forecast horizon" })).getByRole("button", { name: "1Y" }));
    first.unmount();
    renderWithClient(<UnlockedHome />);
    await goTo(/^Cash Flow$/);
    await screen.findByRole("heading", { name: "Cash Flow" });
    expect(within(screen.getByRole("group", { name: "Forecast horizon" })).getByRole("button", { name: "3M" })).toHaveAttribute("aria-pressed", "true");
  });

  it("falls back to the default horizon when a restored value is not one of the options", async () => {
    function Seed() {
      useSessionState<number>("futureCash.horizon", 90)[1](45);
      return null;
    }
    renderWithClient(
      <SessionViewStateProvider>
        <Seed />
        <FutureCashView />
      </SessionViewStateProvider>,
    );
    await screen.findByRole("heading", { name: "Cash Flow" });
    expect(within(screen.getByRole("group", { name: "Forecast horizon" })).getByRole("button", { name: "3M" })).toHaveAttribute("aria-pressed", "true");
  });

  it("puts focus on the Projected activity heading when the open event is gone (the missing-event rule)", async () => {
    function Seed() {
      useSessionState<string | null>("activity.openKey", null)[1]("evt-gone");
      return null;
    }
    renderWithClient(
      <SessionViewStateProvider>
        <Seed />
        <FutureCashView />
      </SessionViewStateProvider>,
    );
    const heading = await screen.findByText("Projected activity", {}, { timeout: 5000 });
    await waitFor(() => expect(heading).toHaveFocus());
  });
});
