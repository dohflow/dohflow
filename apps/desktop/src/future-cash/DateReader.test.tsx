import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";

import type { ForecastDayDto, ForecastEventDto, MultiSeriesForecastDto } from "@/bindings";
import { SessionViewStateProvider } from "@/lib/SessionViewStateProvider";
import { useSessionState } from "@/lib/sessionViewState";
import { DateReader } from "./DateReader";

const money = (minor: number) => ({ minor_units: minor, currency: "USD" });
const band = (minor: number, lo = minor, hi = minor) => ({ p10: money(lo), p50: money(minor), p90: money(hi) });
const event = (name: string, minor: number): ForecastEventDto => ({
  source_event_id: `evt-${name}`,
  name,
  kind: minor < 0 ? "recurring_bill" : "income",
  amount: money(minor),
  assumption_basis: { kind: "recurring_schedule", frequency: "monthly" },
});
const day = (date: string, closing: number, events: ForecastEventDto[] = [], lo = closing, hi = closing): ForecastDayDto => ({
  date,
  closing: band(closing, lo, hi),
  events,
});

// Two days of activity and a quiet one between (the plain "carries forward" case is a date with no event).
function projection(): MultiSeriesForecastDto {
  const checking = [
    day("2026-09-01", 2_513_317),
    day("2026-09-08", 2_494_885, [event("Utilities", -18_432)]),
    day("2026-09-09", 2_494_885),
    day("2026-09-11", 2_786_125, [event("Payroll", 291_240)], 2_700_000, 2_900_000),
  ];
  const reserve = ["2026-09-01", "2026-09-08", "2026-09-09", "2026-09-11"].map((d) => day(d, 800_000));
  const net = checking.map((d, i) => ({ date: d.date, closing: band(d.closing.p50.minor_units + 800_000, d.closing.p10.minor_units + 800_000, d.closing.p90.minor_units + 800_000 + i * 0) }));
  return {
    currency: "USD",
    start_date: "2026-09-01",
    horizon_days: 90,
    accounts: [
      { account_id: "acc-1", name: "Glassmoor Checking", subtype: "checking", tier: "spendable", days: checking },
      { account_id: "acc-2", name: "Glassmoor Savings", subtype: "savings", tier: "reserve", days: reserve },
    ],
    groups: [
      { tier: "spendable", closings: checking.map((d) => ({ date: d.date, closing: d.closing })) },
      { tier: "reserve", closings: reserve.map((d) => ({ date: d.date, closing: d.closing })) },
      { tier: "net", closings: net },
    ],
  };
}

function renderReader(initial?: string | null) {
  function Seed() {
    useSessionState<string | null>("forecast.selectedDate", null)[1](initial ?? null);
    return null;
  }
  return render(
    <SessionViewStateProvider>
      {initial !== undefined && <Seed />}
      <DateReader projection={projection()} />
    </SessionViewStateProvider>,
  );
}

const select = () => screen.getByRole("combobox", { name: "Forecast date" }) as HTMLSelectElement;

describe("Read forecast by date (personal-cfo-i93n3.27 slice C)", () => {
  it("opens on the first date with the forecast's own opening: no earlier balance is invented", () => {
    renderReader();
    expect(select().value).toBe("2026-09-01");
    expect(screen.getByText("The forecast starts here.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Previous day" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Next day" })).toBeEnabled();
  });

  it("offers exactly the dates the data has, and Previous/Next walk that spine with disabled ends", () => {
    renderReader();
    expect([...select().options].map((o) => o.value)).toEqual(["2026-09-01", "2026-09-08", "2026-09-09", "2026-09-11"]);
    fireEvent.click(screen.getByRole("button", { name: "Next day" }));
    expect(select().value).toBe("2026-09-08");
    fireEvent.click(screen.getByRole("button", { name: "Next day" }));
    expect(select().value).toBe("2026-09-09"); // the quiet day: no skipping to the next event
    fireEvent.click(screen.getByRole("button", { name: "Next day" }));
    expect(select().value).toBe("2026-09-11");
    expect(screen.getByRole("button", { name: "Next day" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Previous day" }));
    expect(select().value).toBe("2026-09-09");
  });

  it("reads the date: opening balance, each activity with its signed amount and account, then the closings", () => {
    renderReader();
    fireEvent.change(select(), { target: { value: "2026-09-08" } });
    const reading = screen.getByRole("region", { name: /forecast for/i });
    expect(within(reading).getByRole("heading", { level: 3 })).toHaveTextContent("Sep 8, 2026");
    // Opening is the previous date's closing, per group.
    // Net cash appears in the opening (the previous date's closing) and in the closing.
    expect(within(reading).getAllByText("Net cash", { selector: "dt" })).toHaveLength(2);
    expect(within(reading).getAllByText("$33,133.17").length).toBeGreaterThan(0); // 25,133.17 + 8,000.00
    expect(within(reading).getByText(/Utilities/)).toBeInTheDocument();
    expect(within(reading).getByText("· Glassmoor Checking")).toBeInTheDocument();
    expect(within(reading).getByText("-$184.32")).toBeInTheDocument();
    // Closing: Spendable $24,948.85, Net $32,948.85 (integer cents), per account too.
    expect(within(reading).getAllByText("$24,948.85").length).toBeGreaterThan(0);
    expect(within(reading).getByText("$32,948.85")).toBeInTheDocument();
    expect(within(reading).getByText("Glassmoor Savings", { selector: "dt" })).toBeInTheDocument();
  });

  it("says so on a date with no activity, and shows the data's own closing", () => {
    renderReader();
    fireEvent.change(select(), { target: { value: "2026-09-09" } });
    expect(screen.getByText("No activity on this date.")).toBeInTheDocument();
    expect(screen.getAllByText("$24,948.85").length).toBeGreaterThan(0);
  });

  it("shows P10 and P90 only where the data has a range, never summed", () => {
    renderReader();
    fireEvent.change(select(), { target: { value: "2026-09-11" } });
    const reading = screen.getByRole("region", { name: /forecast for/i });
    // The group (Spendable) and the account both carry the data's own range, each from its own band.
    expect(within(reading).getAllByText(/\$27,861\.25 \(P10 \$27,000\.00, P90 \$29,000\.00\)/).length).toBe(2);
    // A collapsed band shows the median only.
    fireEvent.change(select(), { target: { value: "2026-09-08" } });
    expect(screen.queryByText(/P10/)).toBeNull();
  });

  it("moves a restored date the forecast no longer offers to the nearest one and announces it", () => {
    renderReader("2026-09-10");
    expect(select().value).toMatch(/2026-09-(09|11)/);
    const live = document.querySelector('[aria-live="polite"]')!;
    expect(live.textContent).toMatch(/^Showing /);
    expect(live.className).not.toContain("sr-only");
  });

  it("says nothing when the selection is still valid", () => {
    renderReader("2026-09-09");
    expect(select().value).toBe("2026-09-09");
    expect(document.querySelector('[aria-live="polite"]')!.textContent).toBe("");
  });

  it("keeps the selected date across a remount inside the session", () => {
    function Host() {
      const [show, setShow] = useState(true);
      return (
        <SessionViewStateProvider>
          <button type="button" onClick={() => setShow((v) => !v)}>
            toggle
          </button>
          {show && <DateReader projection={projection()} />}
        </SessionViewStateProvider>
      );
    }
    render(<Host />);
    fireEvent.change(select(), { target: { value: "2026-09-11" } });
    fireEvent.click(screen.getByRole("button", { name: "toggle" }));
    fireEvent.click(screen.getByRole("button", { name: "toggle" }));
    expect(select().value).toBe("2026-09-11");
  });

  it("takes its dates from the data it is given, not from a fixed list", () => {
    const base = projection();
    const shifted = {
      ...base,
      groups: base.groups.map((g) => ({ ...g, closings: g.closings.map((c, i) => ({ ...c, date: `2027-02-0${i + 2}` })) })),
      accounts: base.accounts.map((a) => ({ ...a, days: a.days.map((d, i) => ({ ...d, date: `2027-02-0${i + 2}` })) })),
    };
    render(
      <SessionViewStateProvider>
        <DateReader projection={shifted} />
      </SessionViewStateProvider>,
    );
    expect([...select().options].map((o) => o.value)).toEqual(["2027-02-02", "2027-02-03", "2027-02-04", "2027-02-05"]);
  });

  it("gives the date control its own full-width row on a phone, above Previous day and Next day", () => {
    renderReader();
    const control = select().parentElement!;
    expect(control).toHaveClass("compact:order-first", "compact:basis-full");
    expect(screen.getByRole("button", { name: "Previous day" })).toHaveClass("compact:flex-1");
    expect(screen.getByRole("button", { name: "Next day" })).toHaveClass("compact:flex-1");
  });
});
