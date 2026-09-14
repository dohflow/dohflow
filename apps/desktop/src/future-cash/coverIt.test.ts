import type {
  AccountSeriesDto,
  ForecastDayDto,
  MultiSeriesForecastDto,
} from "@/bindings";
import { detectShortfall } from "./coverIt";

function day(date: string, netMinor: number): ForecastDayDto {
  const m = { minor_units: netMinor, currency: "USD" };
  return { date, closing: { p10: m, p50: m, p90: m }, events: [] };
}

function account(
  id: string | null,
  name: string,
  days: ForecastDayDto[],
): AccountSeriesDto {
  return { account_id: id, name, subtype: null, tier: "spendable", days };
}

function projection(accounts: AccountSeriesDto[]): MultiSeriesForecastDto {
  return {
    currency: "USD",
    start_date: "2026-07-01",
    horizon_days: 90,
    accounts,
    groups: [],
  };
}

test("detects a near-term account dip below $0 with its first date + worst depth", () => {
  const p = projection([
    account("a1", "Checking", [
      day("2026-07-05", 50_000),
      day("2026-07-20", -30_000),
      day("2026-07-25", -45_000),
    ]),
  ]);
  expect(detectShortfall(p)).toEqual({
    accountId: "a1",
    accountName: "Checking",
    date: "2026-07-20",
    shortfallMinor: 45_000,
  });
});

test("no shortfall when the account stays at or above $0", () => {
  const p = projection([
    account("a1", "Checking", [day("2026-07-05", 50_000), day("2026-07-20", 30_000)]),
  ]);
  expect(detectShortfall(p)).toBeNull();
});

test("ignores a dip beyond the near horizon (that's the drift attribution's job)", () => {
  const p = projection([
    account("a1", "Checking", [day("2026-07-05", 50_000), day("2026-10-01", -30_000)]),
  ]);
  expect(detectShortfall(p)).toBeNull();
});

test("picks the deepest shortfall across accounts", () => {
  const p = projection([
    account("a1", "Checking", [day("2026-07-20", -10_000)]),
    account("a2", "Ops", [day("2026-07-18", -50_000)]),
  ]);
  expect(detectShortfall(p)?.accountId).toBe("a2");
});

test("skips the synthetic Unallocated series", () => {
  const p = projection([account(null, "Unallocated cash", [day("2026-07-20", -30_000)])]);
  expect(detectShortfall(p)).toBeNull();
});
