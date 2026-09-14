import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Reads + sets the "auto-categorize imported transactions" preference (ADR 0030
/// addendum, personal-cfo-5n4.2). The backend defaults to `true` when the setting
/// has never been set, so consumers always get a usable value. IPC flows through the
/// generated `commands` only (ADR 0003).
export function useAutoCategorizeOnImport() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.autoCategorizeOnImport,
    queryFn: () =>
      ipcQuery(
        commands.autoCategorizeOnImport(),
        "Could not load the auto-categorize setting.",
      ),
  });

  const mutation = useMutation({
    mutationFn: (enabled: boolean) =>
      commands.setAutoCategorizeOnImport(enabled),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({
          queryKey: queryKeys.autoCategorizeOnImport,
        });
      }
    },
  });

  const setEnabled = useCallback(
    async (enabled: boolean): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(enabled);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  return {
    // Default on while the query is in flight, matching the backend default.
    enabled: query.data ?? true,
    error: query.error?.message ?? null,
    setEnabled,
  };
}
