import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateManualFutureEntryInput,
  type IpcError,
  type UpdateManualFutureEntryInput,
} from "@/bindings";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the manual future entries (TanStack Query, ADR 0020) and exposes add /
/// edit / delete mutations. Each invalidates the entries + forecast caches on
/// success — manual entries feed the forecast (personal-cfo-q6gh), so the chart and
/// ledger refresh whenever one changes. IPC flows through the generated `commands`
/// only; Rust stays authoritative for validation (ADR 0003).
export function useManualEntries() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.manualEntries,
    queryFn: () =>
      ipcQuery(commands.manualFutureEntryList(), "Could not load your entries."),
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.manualEntries });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
  }, [queryClient]);

  const addMutation = useMutation({
    mutationFn: (input: CreateManualFutureEntryInput) =>
      commands.createManualFutureEntry(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const updateMutation = useMutation({
    mutationFn: (input: UpdateManualFutureEntryInput) =>
      commands.updateManualFutureEntry(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) => commands.deleteManualFutureEntry(id),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  const addEntry = useCallback(
    async (input: CreateManualFutureEntryInput): Promise<IpcError | null> => {
      const result = await addMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [addMutation],
  );
  const updateEntry = useCallback(
    async (input: UpdateManualFutureEntryInput): Promise<IpcError | null> => {
      const result = await updateMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [updateMutation],
  );
  const deleteEntry = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await deleteMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [deleteMutation],
  );

  return {
    entries: query.data ?? null,
    error: query.error?.message ?? null,
    addEntry,
    updateEntry,
    deleteEntry,
  };
}
