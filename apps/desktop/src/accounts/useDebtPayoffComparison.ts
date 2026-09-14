import { useQuery } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Loads the debt-payoff strategy comparison (personal-cfo-od07, ADR 0036 debt_payoff): for the
/// household's current carry-debts, each of minimum-only / snowball / avalanche projected at
/// `extraBudgetMinor` extra per month — the debt-free month + total interest per strategy.
/// Read-only and recomputed from the ledger. Keyed under the `["forecast"]` prefix so ledger
/// mutations (balances, debt terms, …) refresh it; the extra budget is part of the key so the
/// comparison refetches when it changes.
export function useDebtPayoffComparison(
  extraBudgetMinor: number,
  /// Restrict the comparison to these debt accounts; **empty means every debt**
  /// (personal-cfo-4d8.27.9.7). The scope reaches the SIMULATION rather than its output —
  /// snowball and avalanche order the debts and route the extra budget among them, so
  /// filtering the result afterwards would report a payoff order the selection does not have.
  accountIds: string[] = [],
) {
  // Keyed on the ids' VALUE, not the array's identity: callers rebuild the array each
  // render, and an identity-keyed query key would refetch forever.
  const scopeKey = accountIds.join(",");
  const query = useQuery({
    queryKey: ["forecast", "payoff", extraBudgetMinor, scopeKey],
    queryFn: () =>
      ipcQuery(
        commands.debtPayoffComparison(
          extraBudgetMinor,
          scopeKey === "" ? [] : scopeKey.split(","),
        ),
        "Could not load your debt-payoff comparison.",
      ),
  });
  return {
    plans: query.data ?? null,
    error: query.error?.message ?? null,
    loading: query.isLoading,
  };
}
