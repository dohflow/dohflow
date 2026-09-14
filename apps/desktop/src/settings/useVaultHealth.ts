import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Runs the vault health check (personal-cfo-n9w) and exposes the read-model
/// rebuild repair (personal-cfo-5ivp). The rebuild invalidates the health verdict
/// plus the derived read-model caches so every screen reflects the repaired state.
/// IPC flows through the generated `commands` only (ADR 0003).
export function useVaultHealth() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.vaultHealth,
    queryFn: () =>
      ipcQuery(commands.vaultHealth(), "Could not run the vault health check."),
  });

  const rebuildMutation = useMutation({
    mutationFn: () => commands.rebuildReadModels(),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.vaultHealth });
        // A rebuild re-materializes the read models the rest of the app reads.
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({ queryKey: queryKeys.bills });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
      }
    },
  });

  const rebuild = useCallback(async (): Promise<IpcError | null> => {
    const result = await rebuildMutation.mutateAsync();
    return result.status === "ok" ? null : result.error;
  }, [rebuildMutation]);

  return {
    /// `null` while loading or on error (see `error`).
    health: query.data ?? null,
    error: query.error?.message ?? null,
    refresh: () => void query.refetch(),
    checking: query.isFetching,
    rebuild,
    rebuilding: rebuildMutation.isPending,
  };
}
