import { useCallback, useEffect, useState } from "react";
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

  // A retry of a read that never produced data puts the query back to pending and clears its error;
  // the failure being retried stays visible meanwhile (the Household read-retry pattern), so the
  // message and its Re-check do not vanish from the slot while the read is in flight.
  const [lastFailure, setLastFailure] = useState<string | null>(null);
  useEffect(() => {
    if (query.status === "error") setLastFailure(query.error.message);
    else if (query.status === "success") setLastFailure(null);
  }, [query.status, query.error]);

  return {
    /// `null` while loading or on error (see `error`).
    health: query.data ?? null,
    /// The failure of the last read that failed, while it is still unresolved (including during a
    /// retry of it), else null.
    error:
      query.status === "error"
        ? query.error.message
        : query.status === "pending"
          ? lastFailure
          : null,
    refresh: () => void query.refetch(),
    checking: query.isFetching,
    /// How many reads have settled as a failure, so a caller can tell that a Re-check failed AGAIN
    /// with the same message (monotonic; `checking` can flip in one batch and miss it).
    failures: query.errorUpdateCount,
    rebuild,
    rebuilding: rebuildMutation.isPending,
  };
}
