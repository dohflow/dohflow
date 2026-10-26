import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type ConfirmObligationEarlyInput,
  type UnconfirmObligationInput,
} from "@/bindings";
import { queryKeys } from "@/lib/query";

/// Confirm / unconfirm a bill occurrence as paid early (personal-cfo-5ie.9). Both post (or void)
/// a real ledger transaction and change what the forecast projects, so on success they invalidate
/// the forecast + the balance-derived caches.
///
/// `unconfirm` is a plain function, not a mutation owned by the calling component: the Undo of a
/// mark-paid lives in a shell notice (docs/agent/FRONTEND.md section 6.2), and the row that marked it
/// leaves the forecast, so the Undo must work after that component is gone (personal-cfo-4d8.32).
export function useMarkObligation() {
  const qc = useQueryClient();
  const invalidate = useCallback(() => {
    void qc.invalidateQueries({ queryKey: ["forecast"] });
    void qc.invalidateQueries({ queryKey: queryKeys.accounts });
    void qc.invalidateQueries({ queryKey: queryKeys.cashTiers });
    void qc.invalidateQueries({ queryKey: queryKeys.cashAvailability });
    void qc.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
    void qc.invalidateQueries({ queryKey: queryKeys.transactions });
  }, [qc]);

  const confirm = useMutation({
    mutationFn: (input: ConfirmObligationEarlyInput) => commands.confirmObligationEarly(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const unconfirm = useCallback(
    async (input: UnconfirmObligationInput): Promise<boolean> => {
      const result = await commands.unconfirmObligation(input);
      if (result.status !== "ok") return false;
      invalidate();
      return true;
    },
    [invalidate],
  );

  return { confirm, unconfirm };
}
