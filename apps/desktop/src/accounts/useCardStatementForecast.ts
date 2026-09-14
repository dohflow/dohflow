import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery } from "@/lib/query";

/// Loads the per-card credit-card statement + payment forecast (personal-cfo-4piy,
/// ADR 0039 §2 — built on 4lhm/llx5): each card's upcoming cycles with the projected
/// statement balance, minimum due, full-pay amount, revolving interest, and the payment its
/// repayment philosophy selects. Read-only and recomputed from the ledger; IPC flows through
/// the generated `commands` only. Invalidated with the rest of the forecast on ledger writes.
export function useCardStatementForecast() {
  const query = useQuery({
    // Keyed under the `["forecast"]` prefix so every mutation that invalidates the forecast
    // (bills, balances, debt terms, …) refreshes the card view too — it projects from the
    // same canonical ledger state.
    queryKey: ["forecast", "cards"],
    queryFn: () =>
      ipcQuery(
        commands.cardStatementForecast(),
        "Could not load your credit-card forecast.",
      ),
  });
  return {
    cards: query.data ?? null,
    error: query.error?.message ?? null,
    loading: query.isLoading,
  };
}

/// Loads one card's past billing-cycle windows with derived-from-imports charge totals and
/// any recorded actual statements — the statement-history capture surface (ADR 0039 addendum
/// 2026-07-10 §2, personal-cfo-4d8.25.4). `enabled` gates the fetch to when the section is
/// actually expanded.
export function useCardStatementHistory(accountId: string, enabled: boolean) {
  const query = useQuery({
    queryKey: ["forecast", "cards", "history", accountId],
    enabled,
    queryFn: () =>
      ipcQuery(
        commands.cardStatementHistory(accountId),
        "Could not load this card's statement history.",
      ),
  });
  return {
    history: query.data ?? null,
    error: query.error?.message ?? null,
    loading: query.isLoading,
  };
}

/// Record — or clear (`null`) — a card statement's REAL balance for one cycle
/// (feedback 2026-07-03). The whole forecast family recomputes from it: the card view,
/// the aggregate Future Cash, and the per-account activity all carry the asserted number.
export function useSetStatementBalance() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (input: {
      accountId: string;
      cycleClose: string;
      statementBalanceMinor: number | null;
    }) =>
      commands.setCardStatementBalance({
        account_id: input.accountId,
        cycle_close: input.cycleClose,
        statement_balance_minor: input.statementBalanceMinor,
        idempotency_key: mintIdempotencyKey(),
      }),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });
  return useCallback(
    async (
      accountId: string,
      cycleClose: string,
      statementBalanceMinor: number | null,
    ): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync({
        accountId,
        cycleClose,
        statementBalanceMinor,
      });
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );
}
