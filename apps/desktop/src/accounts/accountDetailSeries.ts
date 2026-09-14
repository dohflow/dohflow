/// Pure data assembly for the Account Detail chart (personal-cfo-4d8.27.5.7.4, ADR 0050).
///
/// Merges an account's REALIZED history (cash_flow_history, stored-signed) with its FORWARD
/// projection into one row set the chart plots: `hist` (solid realized line), `p50` (dashed
/// median), `band` ([low, high] uncertainty ribbon). All values are converted to the SHOWN
/// sign (positive amount-owed for a card) via the role's balance-sign convention, so the
/// chart itself stays sign-agnostic.

import type {
  AccountHistoryDto,
  AccountSeriesDto,
  CardStatementForecastDto,
} from "@/bindings";

import { storedToShownMinor } from "./balanceSign";

/// One chart row. `hist` is set on realized days, `p50`/`band` on projected days; the
/// today row carries both so the lines meet.
export type DetailChartRow = {
  date: string;
  hist?: number;
  p50?: number;
  band?: [number, number];
};

/// `z_0.90 · sqrt(π/2)` — converts the estimator's MAPE (a mean-absolute relative error)
/// into an 80% half-width, exactly like the backend's payment-date lump sizing
/// (`card_lump_half_width`, ADR 0050).
const MAPE_TO_HALF_WIDTH = 1.2816 * 1.2533;

/// The 80% half-width (minor units) of one open cycle's statement estimate.
export function cardCycleHalfWidthMinor(
  mapeBps: number,
  knownMinor: number,
  variableMinor: number,
): number {
  const scale = knownMinor + variableMinor;
  if (mapeBps <= 0 || scale <= 0) return 0;
  return Math.round(MAPE_TO_HALF_WIDTH * (mapeBps / 10_000) * scale);
}

/// A liquid account: realized days + the per-account forward cone (already banded by the
/// cash-cone/card-lump engine). The forward series' first day IS today, so the today row
/// gets both `hist` and `p50` and the lines connect.
export function buildLiquidChartRows(
  role: string,
  history: AccountHistoryDto | undefined,
  forward: AccountSeriesDto | undefined,
): DetailChartRow[] {
  const rows = new Map<string, DetailChartRow>();
  for (const day of history?.days ?? []) {
    rows.set(day.date, {
      date: day.date,
      hist: storedToShownMinor(role, day.closing.minor_units),
    });
  }
  for (const day of forward?.days ?? []) {
    const row = rows.get(day.date) ?? { date: day.date };
    row.p50 = storedToShownMinor(role, day.closing.p50.minor_units);
    const lo = storedToShownMinor(role, day.closing.p10.minor_units);
    const hi = storedToShownMinor(role, day.closing.p90.minor_units);
    row.band = [Math.min(lo, hi), Math.max(lo, hi)];
    rows.set(day.date, row);
  }
  return [...rows.values()].sort((a, b) => a.date.localeCompare(b.date));
}

/// The next calendar day (`YYYY-MM-DD`, UTC-safe).
function nextIsoDay(iso: string): string {
  return new Date(Date.parse(iso) + 86_400_000).toISOString().slice(0, 10);
}

/// Days from `a` to `b` (ISO dates, positive when `b` is later).
function isoDaysBetween(a: string, b: string): number {
  return Math.round((Date.parse(b) - Date.parse(a)) / 86_400_000);
}

/// A credit card: realized owed history + a piecewise forward path from the projected
/// cycles — owed today → statement at each close → post-payment balance at each due,
/// capped at `horizonDays`. Open (estimated) statements carry an uncertainty band sized
/// from the estimator's walk-forward MAPE (`estimate_mape_bps`); a recorded/closed
/// statement is a known amount (no width), and a full payoff collapses the band to zero
/// at the due date. The path is densified to DAILY rows (linear interpolation between
/// the anchor points): the chart's category X-axis gives every row equal width, so
/// sparse forward points would squeeze months of projection into a few pixels while
/// each history day gets a full slot.
export function buildCardChartRows(
  role: string,
  history: AccountHistoryDto | undefined,
  card: CardStatementForecastDto | undefined,
  todayIso: string,
  horizonDays: number,
): DetailChartRow[] {
  const rows = new Map<string, DetailChartRow>();
  let todayShown: number | undefined;
  for (const day of history?.days ?? []) {
    const shown = storedToShownMinor(role, day.closing.minor_units);
    rows.set(day.date, { date: day.date, hist: shown });
    if (day.date === todayIso) todayShown = shown;
  }

  if (card && todayShown !== undefined && todayIso) {
    // Anchor points along the forward path: (date, owed, half-width).
    const anchors: Array<[string, number, number]> = [[todayIso, todayShown, 0]];
    const horizonEnd = new Date(Date.parse(todayIso) + horizonDays * 86_400_000)
      .toISOString()
      .slice(0, 10);
    for (const cycle of card.cycles) {
      const estimated = !cycle.statement_is_actual && !cycle.is_closed;
      const halfWidth = estimated
        ? cardCycleHalfWidthMinor(
            card.estimate_mape_bps,
            cycle.known_charges_minor,
            cycle.projected_variable_minor,
          )
        : 0;
      if (cycle.close_date > todayIso && cycle.close_date <= horizonEnd) {
        anchors.push([cycle.close_date, cycle.statement_balance_minor, halfWidth]);
      }
      if (cycle.due_date > todayIso && cycle.due_date <= horizonEnd) {
        const afterPay =
          cycle.statement_balance_minor - cycle.forecast_payment_minor;
        // A full payoff clears the statement whatever it turns out to be, so no
        // width survives the payment; a partial payment carries the estimate's
        // uncertainty forward in the owed balance.
        const paidInFull = afterPay <= 0;
        anchors.push([cycle.due_date, Math.max(0, afterPay), paidInFull ? 0 : halfWidth]);
      }
    }
    anchors.sort((a, b) => a[0].localeCompare(b[0]));

    const setForward = (date: string, p50: number, halfWidth: number) => {
      const row = rows.get(date) ?? { date };
      row.p50 = Math.round(p50);
      row.band = [Math.round(p50 - halfWidth), Math.round(p50 + halfWidth)];
      rows.set(date, row);
    };
    // Walk anchor to anchor, filling every day in between by linear interpolation so
    // forward days occupy the same per-day width as history days.
    for (let i = 0; i < anchors.length; i++) {
      const [date, p50, hw] = anchors[i]!;
      setForward(date, p50, hw);
      const next = anchors[i + 1];
      if (!next) continue;
      const span = isoDaysBetween(date, next[0]);
      let day = nextIsoDay(date);
      let step = 1;
      while (day < next[0] && step < span) {
        const t = step / span;
        setForward(day, p50 + (next[1] - p50) * t, hw + (next[2] - hw) * t);
        day = nextIsoDay(day);
        step++;
      }
    }
  }
  return [...rows.values()].sort((a, b) => a.date.localeCompare(b.date));
}
