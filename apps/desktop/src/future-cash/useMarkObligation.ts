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
export function useMarkObligation() {
  const qc = useQueryClient();
  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: ["forecast"] });
    void qc.invalidateQueries({ queryKey: queryKeys.accounts });
    void qc.invalidateQueries({ queryKey: queryKeys.cashTiers });
    void qc.invalidateQueries({ queryKey: queryKeys.cashAvailability });
    void qc.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
    void qc.invalidateQueries({ queryKey: queryKeys.transactions });
  };

  const confirm = useMutation({
    mutationFn: (input: ConfirmObligationEarlyInput) => commands.confirmObligationEarly(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const unconfirm = useMutation({
    mutationFn: (input: UnconfirmObligationInput) => commands.unconfirmObligation(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  return { confirm, unconfirm };
}
