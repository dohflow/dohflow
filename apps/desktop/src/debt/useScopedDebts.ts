import { useQuery } from "@tanstack/react-query";

import { commands, type AccountViewDto } from "@/bindings";
import { ipcQuery } from "@/lib/query";

import type { ScopedDebt } from "./debtStats";

/// Pair each debt in scope with its recorded terms (personal-cfo-g43x).
///
/// One `debt_terms_list` call rather than one per debt (personal-cfo-4d17): N round-trips
/// would mean N loading states, and a stat row that totals a half-loaded set reads as final
/// while being wrong.
///
/// A debt with **no terms on record** is still returned, carrying `terms: null`. That is the
/// distinction the whole derivation rests on — "no rate recorded" is not "0%", and the stat
/// row has to be able to say a total is incomplete rather than quietly omit a debt from it.
export function useScopedDebts(accounts: AccountViewDto[]) {
  // Keyed on the ids' VALUE: callers rebuild the array each render.
  const scopeKey = accounts.map((a) => a.id).join(",");
  const query = useQuery({
    queryKey: ["debt-terms-list", scopeKey],
    enabled: accounts.length > 0,
    queryFn: () =>
      ipcQuery(
        commands.debtTermsList(scopeKey === "" ? [] : scopeKey.split(",")),
        "Could not load debt terms.",
      ),
  });

  const byAccount = new Map((query.data ?? []).map((t) => [t.account_id, t]));
  const debts: ScopedDebt[] | null =
    accounts.length === 0
      ? []
      : query.data === undefined
        ? null
        : accounts.map((account) => ({
            account,
            terms: byAccount.get(account.id) ?? null,
            // Liabilities store a negative signed balance; the page speaks in amount owed.
            owedMinor: Math.max(-account.balance.minor_units, 0),
          }));

  return { debts, error: query.error?.message ?? null };
}
