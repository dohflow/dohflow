import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// The dashboard's Future Cash chart horizon: a 90-day window, matching the
/// Future Cash tab's default so the dashboard previews the same projection
/// (personal-cfo-d5qy).
export const DASHBOARD_HORIZON_DAYS = 90;

/// The "upcoming income / bills" widgets stay a near-term 30-day window even though
/// the chart looks 90 days ahead — those lists are a glanceable next-month view
/// (plan §18.2).
export const DASHBOARD_UPCOMING_DAYS = 30;

/// Loads the deterministic Future Cash forecast (TanStack Query, ADR 0020) that
/// powers every dashboard widget — liquid cash today (its starting balance), the
/// 90-day balance line, and the upcoming income/bills (its per-day events). The
/// `["forecast", …]` key is invalidated by the account/income/bill mutations, so
/// the dashboard refreshes when any of its inputs change. IPC flows through the
/// generated `commands` only.
export function useDashboard() {
  const query = useQuery({
    queryKey: queryKeys.forecast(DASHBOARD_HORIZON_DAYS),
    queryFn: () =>
      ipcQuery(
        // `null` = the base forecast (the dashboard always shows the base).
        commands.futureCashForecast(DASHBOARD_HORIZON_DAYS, []),
        "Could not load your forecast.",
      ),
  });
  return {
    forecast: query.data ?? null,
    error: query.error?.message ?? null,
  };
}
