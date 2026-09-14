import type { ForecastDayDto } from "@/bindings";

/// What the Cash Flow view actually has to show (personal-cfo-4fbl).
///
/// "Empty" here is not "no data" — it is **"not enough to project"**, and there are two
/// distinct ways to be short. The forecast needs a place to start from and something to
/// project; naming *which* one is missing is the difference between a dead end and a next
/// step, and it is the whole reason this is a classifier rather than a boolean.
///
/// Pure and in its own module so it can be tested directly, and so the view file gains no
/// `react-refresh/only-export-components` warning (the `seriesKeys.ts` pattern).
export type CashFlowState =
  /// Nothing to anchor to: no liquid account, so there is no balance to project FROM.
  | "no-anchor"
  /// Anchored, but nothing to project: no income, bills, or other scheduled events, so
  /// the chart would be a flat line restating today's balance as though it were a forecast.
  | "no-schedule"
  /// Both present — draw the thing.
  | "ready";

export function classifyCashFlow({
  liquidAccountCount,
  days,
}: {
  /// Liquid accounts in the projection. Zero means nothing anchors the forecast.
  liquidAccountCount: number;
  /// The projected days. `null` while the forecast is still loading — callers must handle
  /// loading BEFORE classifying, since "no events yet" and "no events at all" look
  /// identical here and only one of them is an empty state.
  days: ForecastDayDto[];
}): CashFlowState {
  if (liquidAccountCount === 0) return "no-anchor";
  // A horizon with no events is a flat line at today's balance. Drawing it would present
  // a restatement of a known number as if it were a projection — the one thing a forecast
  // must not do.
  if (!days.some((day) => day.events.length > 0)) return "no-schedule";
  return "ready";
}
