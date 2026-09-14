import { describe, expect, it } from "vitest";

import type { ForecastDayDto } from "@/bindings";

import { classifyCashFlow } from "./cashFlowState";

const money = (minor: number) => ({ minor_units: minor, currency: "USD" });
const band = (minor: number) => ({
  p10: money(minor),
  p50: money(minor),
  p90: money(minor),
});

function day(date: string, events: ForecastDayDto["events"]): ForecastDayDto {
  return { date, closing: band(250_000), events };
}

const AN_EVENT = [
  {
    kind: "income",
    label: "Paycheck",
    amount: { minor_units: 300_000, currency: "USD" },
    source_id: null,
    source_type: null,
  },
] as unknown as ForecastDayDto["events"];

describe("classifyCashFlow (personal-cfo-4fbl)", () => {
  it("names the missing ANCHOR when there is no liquid account", () => {
    // Nothing to project FROM. Reported even though days exist, because a horizon
    // without a starting balance has nothing to be a forecast of.
    expect(
      classifyCashFlow({
        liquidAccountCount: 0,
        days: [day("2026-06-20", AN_EVENT)],
      }),
    ).toBe("no-anchor");
  });

  it("names the missing SCHEDULE when nothing is expected to move the balance", () => {
    // The distinction that makes this a classifier rather than a boolean: an anchored
    // vault with no schedule would otherwise draw a flat line restating today's balance
    // as though it were a projection.
    expect(
      classifyCashFlow({
        liquidAccountCount: 1,
        days: [day("2026-06-20", []), day("2026-06-21", [])],
      }),
    ).toBe("no-schedule");
  });

  it("is ready as soon as a single event lands anywhere in the horizon", () => {
    // One event is enough — the forecast has something to say. Placed on the LAST day so
    // a check that only looked at the first day would fail here.
    expect(
      classifyCashFlow({
        liquidAccountCount: 1,
        days: [day("2026-06-20", []), day("2026-09-18", AN_EVENT)],
      }),
    ).toBe("ready");
  });

  it("treats an anchored but empty horizon as no-schedule, not ready", () => {
    expect(classifyCashFlow({ liquidAccountCount: 2, days: [] })).toBe(
      "no-schedule",
    );
  });
});
