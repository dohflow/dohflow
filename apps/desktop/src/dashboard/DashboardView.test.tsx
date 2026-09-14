import { fireEvent, screen, within } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ForecastDayDto, ForecastEventDto, ForecastViewDto } from "@/bindings";
import { DashboardView } from "./DashboardView";

const mocks = vi.hoisted(() => ({
  futureCashForecast: vi.fn(),
  cashAvailability: vi.fn(),
  forecastReadiness: vi.fn(),
}));

vi.mock("@/bindings", () => ({
  commands: {
    futureCashForecast: mocks.futureCashForecast,
    cashAvailability: mocks.cashAvailability,
    forecastReadiness: mocks.forecastReadiness,
  },
}));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;

function event(
  name: string,
  kind: string,
  minorUnits: number,
): ForecastEventDto {
  return {
    source_event_id: `evt-${name}-${minorUnits}`,
    name,
    kind,
    amount: { minor_units: minorUnits, currency: "USD" },
    assumption_basis: { kind: "recurring_schedule", frequency: "monthly" },
  };
}

function day(
  date: string,
  closingMinor: number,
  events: ForecastEventDto[] = [],
): ForecastDayDto {
  const closing = { minor_units: closingMinor, currency: "USD" };
  return {
    date,
    closing: { p10: closing, p50: closing, p90: closing },
    events,
  };
}

function forecast(over: Partial<ForecastViewDto> = {}): ForecastViewDto {
  return {
    currency: "USD",
    starting_balance: { minor_units: 250_000, currency: "USD" },
    start_date: "2026-06-20",
    horizon_days: 30,
    days: [
      day("2026-06-20", 250_000),
      day("2026-06-25", 400_000, [event("Acme Corp", "income", 150_000)]),
      day("2026-07-01", 220_000, [event("Rent", "recurring_bill", -180_000)]),
      day("2026-07-10", 215_000, [event("Phone", "recurring_bill", -5_000)]),
      day("2026-07-19", 365_000, [event("Acme Corp", "income", 150_000)]),
    ],
    ...over,
  };
}

beforeEach(() => {
  mocks.futureCashForecast.mockReset();
  mocks.futureCashForecast.mockResolvedValue(ok(forecast()));
  // The dashboard co-renders the self-contained SafeToSpendCard; give it a
  // resolved snapshot so its query never throws during these forecast tests.
  mocks.cashAvailability.mockReset();
  mocks.cashAvailability.mockResolvedValue(
    ok({
      currency: "USD",
      accounts: [],
      net_available: { minor_units: 500_000, currency: "USD" },
      net_committed: { minor_units: 180_000, currency: "USD" },
      net_headroom: { minor_units: 320_000, currency: "USD" },
      floor: { minor_units: 0, currency: "USD" },
      below_floor: false,
    }),
  );
  // The dashboard also co-renders the self-contained ReadinessCard.
  mocks.forecastReadiness.mockReset();
  mocks.forecastReadiness.mockResolvedValue(
    ok({
      score: 80,
      factors: [
        { key: "coverage", label: "Coverage", score: 100, detail: "All set." },
        { key: "freshness", label: "Balance freshness", score: 60, detail: "Current." },
        { key: "explained", label: "Explained activity", score: 100, detail: "Explained." },
      ],
    }),
  );
});

describe("DashboardView", () => {
  it("shows a loading state until the forecast resolves", () => {
    mocks.futureCashForecast.mockReturnValue(new Promise(() => {}));
    renderWithClient(<DashboardView />);
    expect(screen.getByText(/loading your forecast/i)).toBeInTheDocument();
  });

  it("shows liquid cash today, the lowest projected balance, and the end-of-horizon balance", async () => {
    renderWithClient(<DashboardView />);
    // Scope each assertion to its stat card: the shared chart's Y-axis labels can
    // repeat these amounts (e.g. $2,500.00 lands on a gridline too).
    const liquidToday = (await screen.findByText("Liquid cash today")).parentElement!;
    expect(within(liquidToday).getByText("$2,500.00")).toBeInTheDocument();
    // Lowest projected closing balance over the horizon (2026-07-10).
    const lowest = screen.getByText(/Lowest projected/).parentElement!;
    expect(within(lowest).getByText("$2,150.00")).toBeInTheDocument();
    // End-of-horizon balance.
    const projected = screen.getByText(/In 90 days/).parentElement!;
    expect(within(projected).getByText("$3,650.00")).toBeInTheDocument();
    // The shared Cash Flow chart renders an accessible figure.
    expect(screen.getByRole("img", { name: /projected liquid cash/i })).toBeInTheDocument();
  });

  it("opens the Cash Flow tab when the chart is clicked (d5qy)", async () => {
    const onOpenCashFlow = vi.fn();
    renderWithClient(<DashboardView onOpenCashFlow={onOpenCashFlow} />);
    fireEvent.click(
      await screen.findByRole("button", { name: /open the cash flow tab/i }),
    );
    expect(onOpenCashFlow).toHaveBeenCalledTimes(1);
  });

  it("derives upcoming income and bills with signed totals", async () => {
    renderWithClient(<DashboardView />);
    expect(await screen.findByText("Upcoming income")).toBeInTheDocument();
    // Two paychecks of $1,500 → +$3,000.00 total.
    expect(screen.getByText("+$3,000.00")).toBeInTheDocument();
    // Rent $1,800 + Phone $50 → -$1,850.00 total (distinct from either item).
    expect(screen.getByText("-$1,850.00")).toBeInTheDocument();
    expect(screen.getByText("Rent")).toBeInTheDocument();
    expect(screen.getByText("Phone")).toBeInTheDocument();
    expect(screen.getAllByText("Acme Corp").length).toBeGreaterThan(0);
  });

  it("shows explicit empty states when nothing is scheduled", async () => {
    mocks.futureCashForecast.mockResolvedValue(
      ok(
        forecast({
          starting_balance: { minor_units: 0, currency: "USD" },
          days: [day("2026-06-20", 0), day("2026-06-21", 0)],
        }),
      ),
    );
    renderWithClient(<DashboardView />);
    expect(await screen.findByText(/no income expected/i)).toBeInTheDocument();
    expect(screen.getByText(/no bills due/i)).toBeInTheDocument();
    // All three stat cards read $0.00 on an empty vault.
    expect(screen.getAllByText("$0.00").length).toBeGreaterThanOrEqual(3);
  });

  it("surfaces a load error", async () => {
    mocks.futureCashForecast.mockResolvedValue({
      status: "error",
      error: "VaultLocked",
    });
    renderWithClient(<DashboardView />);
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });
});
