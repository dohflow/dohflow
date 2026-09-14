import type {
  AccountHistoryDto,
  AccountSeriesDto,
  CardStatementForecastDto,
} from "@/bindings";

import {
  buildCardChartRows,
  buildLiquidChartRows,
  cardCycleHalfWidthMinor,
} from "./accountDetailSeries";

const usd = (m: number) => ({ minor_units: m, currency: "USD" });
const band = (p10: number, p50: number, p90: number) => ({
  p10: usd(p10),
  p50: usd(p50),
  p90: usd(p90),
});

function history(days: Array<[string, number]>): AccountHistoryDto {
  return {
    account_id: "a1",
    name: "Checking",
    subtype: null,
    tier: "spendable",
    days: days.map(([date, minor]) => ({ date, closing: usd(minor) })),
  };
}

describe("buildLiquidChartRows", () => {
  const forward: AccountSeriesDto = {
    account_id: "a1",
    name: "Checking",
    subtype: null,
    tier: "spendable",
    days: [
      { date: "2026-07-14", closing: band(90_000, 90_000, 90_000), events: [] },
      { date: "2026-07-15", closing: band(80_000, 90_000, 100_000), events: [] },
    ],
  };

  it("merges history and forward around a shared today row", () => {
    const rows = buildLiquidChartRows(
      "liquid_cash",
      history([
        ["2026-07-12", 85_000],
        ["2026-07-13", 88_000],
        ["2026-07-14", 90_000],
      ]),
      forward,
    );
    expect(rows.map((r) => r.date)).toEqual([
      "2026-07-12",
      "2026-07-13",
      "2026-07-14",
      "2026-07-15",
    ]);
    // The today row carries BOTH series so the lines connect.
    const today = rows[2]!;
    expect(today.hist).toBe(90_000);
    expect(today.p50).toBe(90_000);
    // A projected day carries the band, low-first.
    expect(rows[3]!.band).toEqual([80_000, 100_000]);
    expect(rows[3]!.hist).toBeUndefined();
  });

  it("handles a missing forward series (history only)", () => {
    const rows = buildLiquidChartRows(
      "liquid_cash",
      history([["2026-07-14", 90_000]]),
      undefined,
    );
    expect(rows).toEqual([{ date: "2026-07-14", hist: 90_000 }]);
  });
});

describe("cardCycleHalfWidthMinor", () => {
  it("sizes the 80% half-width like the backend lump (z90 · sqrt(π/2) · mape · scale)", () => {
    // 20% MAPE on $1,000 of charges → 1.2816·1.2533·0.2·100000 ≈ 32,125.
    expect(cardCycleHalfWidthMinor(2_000, 60_000, 40_000)).toBe(32_125);
  });
  it("is zero without a recorded MAPE or without charges", () => {
    expect(cardCycleHalfWidthMinor(0, 60_000, 40_000)).toBe(0);
    expect(cardCycleHalfWidthMinor(2_000, 0, 0)).toBe(0);
  });
});

describe("buildCardChartRows", () => {
  const cardHistory: AccountHistoryDto = {
    account_id: "c1",
    name: "Venture X",
    subtype: "credit_card",
    tier: "card",
    // Stored signed: −$400 owed yesterday, −$450 today.
    days: [
      { date: "2026-07-13", closing: usd(-40_000) },
      { date: "2026-07-14", closing: usd(-45_000) },
    ],
  };
  const card: CardStatementForecastDto = {
    account_id: "c1",
    account_name: "Venture X",
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
        statement_balance_minor: 100_000,
        minimum_due_minor: 2_500,
        full_pay_minor: 100_000,
        forecast_payment_minor: 100_000,
        statement_is_actual: false,
        is_closed: false,
      },
    ],
  };

  it("shows owed history as positive and a banded estimate at the open close date", () => {
    const rows = buildCardChartRows("credit_facility", cardHistory, card, "2026-07-14", 120);
    const hist = rows.find((r) => r.date === "2026-07-13")!;
    expect(hist.hist).toBe(40_000); // shown positive owed
    const today = rows.find((r) => r.date === "2026-07-14")!;
    expect(today.hist).toBe(45_000);
    expect(today.p50).toBe(45_000); // forward path starts at today's owed
    const close = rows.find((r) => r.date === "2026-07-28")!;
    expect(close.p50).toBe(100_000);
    expect(close.band).toEqual([100_000 - 32_125, 100_000 + 32_125]);
    // A full payoff clears the statement whatever it turns out to be → no width at due.
    const due = rows.find((r) => r.date === "2026-08-17")!;
    expect(due.p50).toBe(0);
    expect(due.band).toEqual([0, 0]);
  });

  it("a recorded (actual) statement carries no width", () => {
    const actual = {
      ...card,
      cycles: [{ ...card.cycles[0]!, statement_is_actual: true }],
    };
    const rows = buildCardChartRows("credit_facility", cardHistory, actual, "2026-07-14", 120);
    const close = rows.find((r) => r.date === "2026-07-28")!;
    expect(close.band).toEqual([100_000, 100_000]);
  });
});
