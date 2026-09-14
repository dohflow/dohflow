import { screen, waitFor } from "@testing-library/react";

import { renderWithClient } from "@/test/renderWithClient";

import type { ForecastDayDto, MoneyDto } from "@/bindings";
import { ComfortBandSignals } from "./ComfortBandSignals";

const mocks = vi.hoisted(() => ({ bandDriftSignal: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: mocks }));

const ok = <T,>(data: T) => ({ status: "ok", data }) as const;
const usd = (minor: number): MoneyDto => ({ minor_units: minor, currency: "USD" });

function day(date: string, netMinor: number): ForecastDayDto {
  const m = usd(netMinor);
  return { date, closing: { p10: m, p50: m, p90: m }, events: [] };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.bandDriftSignal.mockResolvedValue(ok(null)); // no drift by default
});

test("renders a plain below-band crossing when there's no drift attribution", async () => {
  renderWithClient(
    <ComfortBandSignals
      days={[day("2026-07-01", 600_000), day("2026-07-14", 300_000)]}
      band={{ lower: 500_000, upper: null }}
      currency="USD"
    />,
  );
  expect(
    await screen.findByText(/crosses below the comfort band/i),
  ).toBeInTheDocument();
  expect(screen.getByText(/at its lowest/i)).toBeInTheDocument();
});

test("shows the drift attribution (the why) when the crossing is spending-driven", async () => {
  mocks.bandDriftSignal.mockResolvedValue(
    ok({
      crossing_date: "2026-09-20",
      magnitude: usd(70_000),
      factors: [
        {
          category_id: "c1",
          category_name: "Dining",
          recent_monthly: usd(70_000),
          delta: usd(30_000),
        },
      ],
    }),
  );
  renderWithClient(
    <ComfortBandSignals
      days={[day("2026-09-20", 400_000)]}
      band={{ lower: 500_000, upper: null }}
      currency="USD"
    />,
  );
  // Once the drift query resolves, the richer "why" copy replaces the plain crossing.
  await waitFor(() =>
    expect(screen.getByRole("status").textContent).toContain(
      "driven mostly by Dining (up about",
    ),
  );
  expect(screen.queryByText(/at its lowest/i)).toBeNull();
});

test("renders the above-band excess (drift never applies to the upper edge)", async () => {
  renderWithClient(
    <ComfortBandSignals
      days={[day("2026-07-01", 600_000), day("2026-07-20", 1_300_000)]}
      band={{ lower: 500_000, upper: 1_000_000 }}
      currency="USD"
    />,
  );
  expect(await screen.findByText(/above the comfort band/i)).toBeInTheDocument();
});

test("renders nothing when the projection stays within the band and there's no drift", async () => {
  const { container } = renderWithClient(
    <ComfortBandSignals
      days={[day("2026-07-01", 700_000)]}
      band={{ lower: 500_000, upper: 1_000_000 }}
      currency="USD"
    />,
  );
  await waitFor(() => expect(mocks.bandDriftSignal).toHaveBeenCalled());
  expect(container).toBeEmptyDOMElement();
});
