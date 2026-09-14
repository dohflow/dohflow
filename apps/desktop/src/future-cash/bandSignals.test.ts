import type { ForecastDayDto } from "@/bindings";
import { comfortBandSignals } from "./bandSignals";

function day(date: string, netMinor: number): ForecastDayDto {
  const m = { minor_units: netMinor, currency: "USD" };
  return { date, closing: { p10: m, p50: m, p90: m }, events: [] };
}

test("flags the first below-band crossing with the lowest point", () => {
  const days = [
    day("2026-07-01", 600_000),
    day("2026-07-10", 400_000),
    day("2026-07-20", 300_000),
  ];
  expect(comfortBandSignals(days, { lower: 500_000, upper: null })).toEqual([
    { kind: "below", date: "2026-07-10", lowestMinor: 300_000 },
  ]);
});

test("flags ending above the upper edge as excess", () => {
  const days = [day("2026-07-01", 600_000), day("2026-07-20", 1_300_000)];
  expect(comfortBandSignals(days, { lower: 500_000, upper: 1_000_000 })).toEqual([
    { kind: "above", excessMinor: 300_000 },
  ]);
});

test("emits both a below and an above signal when the path dips then ends high", () => {
  const days = [
    day("2026-07-05", 300_000), // below the lower
    day("2026-07-20", 1_400_000), // ends above the upper
  ];
  const signals = comfortBandSignals(days, { lower: 500_000, upper: 1_000_000 });
  expect(signals.map((s) => s.kind)).toEqual(["below", "above"]);
});

test("no signal while the projection stays within the band", () => {
  const days = [day("2026-07-01", 700_000), day("2026-07-20", 800_000)];
  expect(comfortBandSignals(days, { lower: 500_000, upper: 1_000_000 })).toEqual([]);
});

test("no signal without a band or without days", () => {
  expect(comfortBandSignals([day("2026-07-01", 100_000)], null)).toEqual([]);
  expect(comfortBandSignals([], { lower: 500_000, upper: null })).toEqual([]);
});
