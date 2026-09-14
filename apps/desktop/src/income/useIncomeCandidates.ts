// Income-source candidates from recurring inbound deposits (personal-cfo-gmnk):
// the bill detector pointed at the inflow side, sharing its suppression store —
// a dismissal here is the same durable (merchant, currency) dismissal.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";

import {
  commands,
  type DismissRecurringSuggestionInput,
  type IpcError,
} from "@/bindings";
import { ipcQuery } from "@/lib/query";
import { BILL_CANDIDATES_KEY } from "@/bills/useRecurringCandidates";

export const INCOME_CANDIDATES_KEY = ["income", "candidates"] as const;

export function useIncomeCandidates() {
  const query = useQuery({
    queryKey: INCOME_CANDIDATES_KEY,
    queryFn: () =>
      ipcQuery(commands.incomeCandidates(), "Could not load income suggestions."),
  });
  return {
    candidates: query.data ?? null,
    error: query.error?.message ?? null,
    loading: query.isLoading,
  };
}

export function useDismissIncomeCandidate() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (input: DismissRecurringSuggestionInput) =>
      commands.dismissRecurringSuggestion(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        // One suppression store serves both detectors.
        void queryClient.invalidateQueries({ queryKey: INCOME_CANDIDATES_KEY });
        void queryClient.invalidateQueries({ queryKey: BILL_CANDIDATES_KEY });
      }
    },
  });
  const dismiss = useCallback(
    async (input: DismissRecurringSuggestionInput): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );
  return { dismiss };
}
