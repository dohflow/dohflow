import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// The horizons the Future Cash view offers, in days (1M / 3M / 6M / 1Y).
export const FUTURE_CASH_HORIZONS = [
  { days: 30, label: "1M" },
  { days: 90, label: "3M" },
  { days: 180, label: "6M" },
  { days: 365, label: "1Y" },
] as const;

/// The horizon the view opens on (3 months — far enough to see the shape, near
/// enough to be legible).
export const DEFAULT_FUTURE_CASH_HORIZON = 90;

/// Loads the deterministic Future Cash forecast for a selectable horizon and
/// scenario (TanStack Query, ADR 0020). `scenarioId` is `null` for the base
/// forecast; a scenario id layers that scenario's overlays (personal-cfo-6zep) —
/// base and scenario cache independently, so the compare-vs-base view holds both.
/// Every (horizon, scenario) is invalidated together by the account/income/bill
/// and scenario mutations. IPC flows through the generated `commands` only.
export function useFutureCash(
  horizonDays: number,
  /// The ORDERED scenario selection; empty is the base forecast. Order is the precedence
  /// (ADR 0059 §1), so `[a, b]` and `[b, a]` are different forecasts and must cache apart.
  scenarioIds: string[] = [],
) {
  // Keyed on the ids' VALUE and their ORDER, not the array's identity — callers rebuild
  // the array each render, and order is meaningful so it belongs in the key.
  const scopeKey = scenarioIds.join(",");
  const query = useQuery({
    queryKey: queryKeys.forecast(horizonDays, scopeKey),
    queryFn: () =>
      ipcQuery(
        commands.futureCashForecast(
          horizonDays,
          scopeKey === "" ? [] : scopeKey.split(","),
        ),
        "Could not load your forecast.",
      ),
  });
  return {
    forecast: query.data ?? null,
    error: query.error?.message ?? null,
  };
}

/// Loads the per-account/per-group Future Cash projection (personal-cfo-l8oh,
/// ADR 0026 §12) for the multi-series chart + spreadsheet table — one running
/// series per liquid account (plus Unallocated) and the cash-tier rollups. Same
/// `(horizon, scenario)` parameters and invalidation as [`useFutureCash`].
export function useFutureCashByAccount(
  horizonDays: number,
  /// The ORDERED scenario selection; empty is the base projection (ADR 0059 §1).
  scenarioIds: string[] = [],
) {
  const scopeKey = scenarioIds.join(",");
  const query = useQuery({
    queryKey: queryKeys.forecastByAccount(horizonDays, scopeKey),
    queryFn: () =>
      ipcQuery(
        commands.futureCashByAccount(
          horizonDays,
          scopeKey === "" ? [] : scopeKey.split(","),
        ),
        "Could not load your forecast breakdown.",
      ),
  });
  return {
    projection: query.data ?? null,
    error: query.error?.message ?? null,
  };
}
