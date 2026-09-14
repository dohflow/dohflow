import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type DebtTermsDto,
  type IpcError,
  type SetDebtTermsInput,
} from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Loads + upserts a liability account's debt terms (ADR 0035 §5, personal-cfo-6wk.2).
/// IPC flows through the generated `commands` only; the kernel stays authoritative for
/// validation (target must be a liability, paying source must be liquid — ADR 0035 §1/§2).
/// Saving invalidates the terms + the forecast (debt payments feed the projection).
export function useDebtTerms(accountId: string) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["debt-terms", accountId],
    // Skip the fetch when there is no account yet (the unified editor mounts this in
    // create mode before an id exists, ADR 0044); `save` still posts by input.account_id.
    enabled: accountId.trim() !== "",
    queryFn: () =>
      ipcQuery(commands.debtTerms(accountId), "Could not load debt terms."),
  });

  const mutation = useMutation({
    mutationFn: (input: SetDebtTermsInput) => commands.setDebtTerms(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: ["debt-terms", accountId] });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });

  const save = useCallback(
    async (input: SetDebtTermsInput): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  return {
    terms: (query.data ?? null) as DebtTermsDto | null,
    loading: query.isLoading,
    error: query.error?.message ?? null,
    save,
  };
}
