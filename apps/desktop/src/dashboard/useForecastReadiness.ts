import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Reads the R1 Forecast Readiness score (ADR 0026 §13, personal-cfo-6vj9): a
/// 0–100 data-maturity indicator (coverage + balance freshness + explained ratio)
/// with a per-factor breakdown. Derived on read; invalidated by the account /
/// income / bill / balance / transaction mutations. IPC flows through the
/// generated `commands` only (ADR 0003).
export function useForecastReadiness() {
  const query = useQuery({
    queryKey: queryKeys.forecastReadiness,
    queryFn: () =>
      ipcQuery(
        commands.forecastReadiness(),
        "Could not load your forecast readiness.",
      ),
  });

  return {
    /// `null` while loading or on error (see `error`).
    readiness: query.data ?? null,
    error: query.error?.message ?? null,
  };
}
