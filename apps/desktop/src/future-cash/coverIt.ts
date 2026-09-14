import type { MultiSeriesForecastDto } from "@/bindings";

/// Crossings within this many days are "near" — the user-steered cover-it tool's domain; a further
/// shortfall is carried descriptively by the drift attribution (5ie.8). Mirrors the engine's
/// NEAR_HORIZON_DAYS (ADR 0018 timing addendum).
export const NEAR_HORIZON_DAYS = 45;

/// A near-term per-account cash shortfall — a specific liquid account projected negative.
export type Shortfall = {
  accountId: string;
  accountName: string;
  /// The first day the account's projected balance goes negative, `YYYY-MM-DD`.
  date: string;
  /// The positive amount that would bring its lowest point back to $0.
  shortfallMinor: number;
};

/// `iso` (`YYYY-MM-DD`) advanced by `days`, as `YYYY-MM-DD`.
function addDays(iso: string, days: number): string {
  const d = new Date(`${iso}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

/// The worst near-term per-account shortfall in the projection (a liquid account whose projected
/// balance dips below $0 within `nearHorizonDays`), or `null`. Descriptive detection for the
/// user-steered "cover it" tool (ADR 0018 addendum, personal-cfo-j0cg.1) — timing gates it to NEAR
/// shortfalls; far drifts are the attribution's job. `YYYY-MM-DD` strings compare chronologically.
export function detectShortfall(
  projection: MultiSeriesForecastDto,
  nearHorizonDays: number = NEAR_HORIZON_DAYS,
): Shortfall | null {
  const cutoff = addDays(projection.start_date, nearHorizonDays);
  let worst: Shortfall | null = null;
  for (const series of projection.accounts) {
    if (series.account_id === null) continue; // skip the synthetic Unallocated series
    let firstNegDate: string | null = null;
    let lowest = 0;
    for (const day of series.days) {
      if (day.date > cutoff) break; // days are ascending; only the near window
      const net = day.closing.p50.minor_units;
      if (net < 0) {
        if (firstNegDate === null) firstNegDate = day.date;
        if (net < lowest) lowest = net;
      }
    }
    if (firstNegDate !== null && (worst === null || -lowest > worst.shortfallMinor)) {
      worst = {
        accountId: series.account_id,
        accountName: series.name,
        date: firstNegDate,
        shortfallMinor: -lowest,
      };
    }
  }
  return worst;
}
