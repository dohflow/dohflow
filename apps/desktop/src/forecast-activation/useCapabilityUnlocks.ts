import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type CapabilityUnlockDto, type IpcError } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads forecast capabilities that have self-activated but whose one-time unlock notice
/// the user hasn't acknowledged yet (ADR 0026 §10, personal-cfo-egon), plus the acknowledge
/// action that dismisses one for good. Acknowledging invalidates the query so the notice
/// drops immediately; otherwise it refreshes on mount / window focus, so a band that
/// activates mid-session surfaces its notice on the next look. IPC flows through the
/// generated `commands` only — Rust stays authoritative (ADR 0003).
export function useCapabilityUnlocks() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.capabilityUnlocks,
    queryFn: () =>
      ipcQuery(
        commands.pendingCapabilityUnlocks(),
        "Could not check for new forecast features.",
      ),
  });

  const acknowledgeMutation = useMutation({
    mutationFn: (key: string) => commands.acknowledgeCapability(key),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({
          queryKey: queryKeys.capabilityUnlocks,
        });
      }
    },
  });

  const acknowledge = useCallback(
    async (key: string): Promise<IpcError | null> => {
      const result = await acknowledgeMutation.mutateAsync(key);
      return result.status === "ok" ? null : result.error;
    },
    [acknowledgeMutation],
  );

  const unlocks: CapabilityUnlockDto[] = query.data ?? [];
  return { unlocks, acknowledge };
}
