import { fireEvent, render, screen } from "@testing-library/react";

import type {
  AccountHistoryDto,
  AccountSeriesDto,
  DayBalanceDto,
  GroupSeriesDto,
} from "@/bindings";
import { MultiSeriesChart } from "./MultiSeriesChart";

function days(values: number[]): AccountSeriesDto["days"] {
  return values.map((v, i) => ({
    date: `2026-06-${String(20 + i).padStart(2, "0")}`,
    closing: band(v),
    events: [],
  }));
}

function band(minor: number) {
  const m = { minor_units: minor, currency: "USD" };
  return { p10: m, p50: m, p90: m };
}

function closings(values: number[]): DayBalanceDto[] {
  return values.map((v, i) => ({
    date: `2026-06-${String(20 + i).padStart(2, "0")}`,
    closing: band(v),
  }));
}

const GROUPS: GroupSeriesDto[] = [
  { tier: "spendable", closings: closings([120_000, 130_000, 90_000]) },
  { tier: "reserve", closings: closings([500_000, 500_000, 500_000]) },
  { tier: "net", closings: closings([620_000, 630_000, 590_000]) },
];

describe("MultiSeriesChart", () => {
  it("renders an accessible figure with a group legend on the chart primitive", () => {
    render(<MultiSeriesChart groups={GROUPS} currency="USD" />);
    // The figure names the net + group series for screen readers.
    const chart = screen.getByRole("figure", { name: /by group/i });
    expect(chart).toHaveAccessibleName(/net cash/i);
    expect(chart).toHaveAccessibleName(/spendable/i);
    // The visible legend lists each plotted series.
    expect(screen.getByText("Net cash")).toBeInTheDocument();
    expect(screen.getByText("Spendable")).toBeInTheDocument();
    expect(screen.getByText("Reserve")).toBeInTheDocument();
  });

  it("names Spendable as the primary series (personal-cfo-4d8.25.25)", () => {
    render(<MultiSeriesChart groups={GROUPS} currency="USD" />);
    expect(screen.getByRole("figure")).toHaveAccessibleName(/spendable \(primary\)/i);
  });

  it("does not claim a Spendable primary when Spendable is flat-zero (4d8.25.25 review)", () => {
    // A vault with only reserve accounts: spendable stays flat at zero and is dropped.
    render(
      <MultiSeriesChart
        groups={[
          { tier: "spendable", closings: closings([0, 0, 0]) },
          { tier: "reserve", closings: closings([500_000, 500_000, 500_000]) },
          { tier: "net", closings: closings([500_000, 500_000, 500_000]) },
        ]}
        currency="USD"
      />,
    );
    const chart = screen.getByRole("figure");
    expect(chart).not.toHaveAccessibleName(/spendable \(primary\)/i);
    expect(chart).toHaveAccessibleName(/reserve/i);
  });

  it("toggles a series' visibility from the legend (personal-cfo-4d8.25.25)", () => {
    render(<MultiSeriesChart groups={GROUPS} currency="USD" />);
    const spendable = screen.getByRole("button", { name: /spendable/i });
    // Starts shown (pressed); clicking hides it, clicking again shows it.
    expect(spendable).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(spendable);
    expect(spendable).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(spendable);
    expect(spendable).toHaveAttribute("aria-pressed", "true");
  });

  it("plots an individual account when the selection picks it (personal-cfo-4d8.25.26)", () => {
    const accounts: AccountSeriesDto[] = [
      { account_id: "chk-1", name: "Everyday Checking", subtype: "checking", tier: "spendable", days: days([120_000, 130_000, 90_000]) },
    ];
    render(
      <MultiSeriesChart
        groups={GROUPS}
        accounts={accounts}
        // Reserve tier + the one checking account (Spendable tier deselected).
        selection={["net", "reserve", "acct:chk-1"]}
        currency="USD"
      />,
    );
    // The account appears as its own legend series; the Spendable aggregate does not.
    expect(
      screen.getByRole("button", { name: /everyday checking/i }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^spendable/i }),
    ).not.toBeInTheDocument();
  });

  it("never paints two series the same colour, and names what it left out (hnba)", () => {
    // Five picked accounts, four categorical slots — and two of those slots are already
    // taken by the plotted Net and Reserve tiers. This used to index the palette with
    // `i % length`, so several accounts were drawn in colours already on the chart: two
    // different accounts as one line, and the legend showing the same swatch twice.
    const accounts: AccountSeriesDto[] = Array.from({ length: 5 }, (_, i) => ({
      account_id: `acc-${i}`,
      name: `Account ${i}`,
      subtype: "checking",
      tier: "spendable",
      days: days([120_000, 130_000, 90_000]),
    }));
    render(
      <MultiSeriesChart
        groups={GROUPS}
        accounts={accounts}
        selection={["net", "reserve", ...accounts.map((a) => `acct:${a.account_id}`)]}
        currency="USD"
      />,
    );

    // Every legend swatch is a distinct colour — the invariant, asserted directly.
    const swatches = Array.from(
      document.querySelectorAll<HTMLElement>("figcaption button span[style*='background']"),
    ).map((el) => el.style.backgroundColor);
    expect(swatches.length).toBeGreaterThan(1);
    expect(new Set(swatches).size).toBe(swatches.length);

    // …and the accounts that could not get a colour are named rather than vanishing.
    expect(screen.getByText(/not plotted, to keep every line a distinct colour/i))
      .toBeInTheDocument();
  });

  it("renders the comfort band without breaking the figure (3v6d)", () => {
    // A two-edge band shades behind the series; the chart still renders normally.
    render(
      <MultiSeriesChart
        groups={GROUPS}
        currency="USD"
        band={{ lower: 200_000, upper: 800_000 }}
      />,
    );
    expect(screen.getByRole("figure", { name: /by group/i })).toBeInTheDocument();
    expect(screen.getByText("Net cash")).toBeInTheDocument();
  });

  it("shows an empty hint when there is not enough data to chart", () => {
    render(
      <MultiSeriesChart
        groups={[{ tier: "net", closings: closings([100_000]) }]}
        currency="USD"
      />,
    );
    expect(screen.getByText(/not enough data/i)).toBeInTheDocument();
  });

  it("draws realized history behind a TODAY divider and says so accessibly (4d8.27.5.3)", () => {
    const history: AccountHistoryDto[] = [
      {
        account_id: "a1",
        name: "Checking",
        subtype: "checking",
        tier: "spendable",
        days: [
          { date: "2026-06-18", closing: { minor_units: 100_000, currency: "USD" } },
          { date: "2026-06-19", closing: { minor_units: 110_000, currency: "USD" } },
          { date: "2026-06-20", closing: { minor_units: 120_000, currency: "USD" } },
        ],
      },
      // A card's owed history is excluded from the liquid chart.
      {
        account_id: "c1",
        name: "Visa",
        subtype: "credit_card",
        tier: "card",
        days: [
          { date: "2026-06-18", closing: { minor_units: -40_000, currency: "USD" } },
        ],
      },
    ];
    render(
      <MultiSeriesChart
        groups={GROUPS}
        currency="USD"
        history={history}
        todayIso="2026-06-20"
      />,
    );
    // The figure names both windows: realized days behind, projection ahead.
    const chart = screen.getByRole("figure");
    expect(chart).toHaveAccessibleName(/2 days of realized history/i);
    expect(chart).toHaveAccessibleName(/projected range over the next 3 days/i);
    // The legend explains the solid-vs-dashed split.
    expect(screen.getByText("Realized")).toBeInTheDocument();
    expect(screen.getByText("Projected")).toBeInTheDocument();
  });

  it("keeps the projected-only label when no history is passed", () => {
    render(<MultiSeriesChart groups={GROUPS} currency="USD" />);
    expect(screen.getByRole("figure")).toHaveAccessibleName(
      /projected liquid cash by group over the next 3 days/i,
    );
    expect(screen.queryByText("Realized")).not.toBeInTheDocument();
  });
});
