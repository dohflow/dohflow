import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type DismissRecurringSuggestionInput,
  type IpcError,
} from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Loads detected recurring-bill candidates (personal-cfo-98ql): merchants that recur at a
/// consistent cadence + amount in the realized history, not already tracked as a bill. Read-only
/// suggestions — the user confirms before any bill is created (ADR 0018). Keyed under the
/// `["bills"]` prefix so creating/removing a bill (which can add or clear a suggestion) refreshes
/// it.
export const BILL_CANDIDATES_KEY = ["bills", "candidates"] as const;

export function useRecurringCandidates() {
  const query = useQuery({
    queryKey: BILL_CANDIDATES_KEY,
    queryFn: () =>
      ipcQuery(
        commands.recurringCandidates(),
        "Could not load recurring suggestions.",
      ),
  });
  return {
    candidates: query.data ?? null,
    error: query.error?.message ?? null,
    loading: query.isLoading,
  };
}

/// Dismiss a recurring suggestion (ADR 0046, personal-cfo-4d8.24.6): records a
/// `(merchant_key, currency)` suppression so detection stops offering it until the pattern
/// materially changes. On success the candidate list refreshes and the row drops out.
export function useDismissRecurringSuggestion() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (input: DismissRecurringSuggestionInput) =>
      commands.dismissRecurringSuggestion(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
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
