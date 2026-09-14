import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Loads suspected loan double-counts (personal-cfo-6wk.11): loans tracked as both a loan account
/// with payment terms AND an active recurring `loan_payment` bill, which double-counts the payment
/// in the liquid forecast. Read-only + heuristic (name/amount); descriptive only (ADR 0018).
/// Keyed under `["forecast"]` so ledger mutations (accounts, debt terms, bills) refresh it.
export function useLoanDoubleCountWarnings() {
  const query = useQuery({
    queryKey: ["forecast", "loanDoubleCounts"],
    queryFn: () =>
      ipcQuery(
        commands.loanDoubleCountWarnings(),
        "Could not check for duplicated loans.",
      ),
  });
  return {
    warnings: query.data ?? null,
    error: query.error?.message ?? null,
    loading: query.isLoading,
  };
}
