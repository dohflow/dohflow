import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Reads the vault's base/reporting currency (TanStack Query, ADR 0020) and
/// exposes a setter. The backend defaults to `"USD"` when unset, so consumers
/// always get a usable code. The setter invalidates the base-currency key plus
/// the account/bill/income caches, so their add-forms re-default to the new
/// currency. IPC flows through the generated `commands` only (ADR 0003).
export function useBaseCurrency() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.baseCurrency,
    queryFn: () =>
      ipcQuery(commands.baseCurrency(), "Could not load the base currency."),
  });

  const mutation = useMutation({
    mutationFn: (code: string) => commands.setBaseCurrency(code),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.baseCurrency });
        void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
        void queryClient.invalidateQueries({ queryKey: queryKeys.bills });
        void queryClient.invalidateQueries({ queryKey: queryKeys.income });
      }
    },
  });

  const setBaseCurrency = useCallback(
    async (code: string): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(code);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  return {
    // Fall back to USD while the query is in flight so forms always have a value.
    baseCurrency: query.data ?? "USD",
    error: query.error?.message ?? null,
    setBaseCurrency,
  };
}
