import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Loads the realized cash-flow history (personal-cfo-4d8.27.5.2): each spending
/// account's actual daily closing balance over the trailing lookback, folded backward
/// from today over the ledger and clamped to the account's earliest real data. Keyed
/// under the `["forecast"]` prefix (like the card view) so every ledger/balance
/// mutation that invalidates the forecast refreshes the realized line too.
export function useCashFlowHistory(lookbackDays: number) {
  const query = useQuery({
    queryKey: ["forecast", "history", lookbackDays],
    queryFn: () =>
      ipcQuery(
        commands.cashFlowHistory(lookbackDays),
        "Could not load your balance history.",
      ),
  });
  return {
    history: query.data ?? null,
    error: query.error?.message ?? null,
    loading: query.isLoading,
  };
}
