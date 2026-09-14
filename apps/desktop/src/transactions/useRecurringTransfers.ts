import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateRecurringTransferInput,
  type IpcError,
} from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the recurring-transfer list (TanStack Query, ADR 0020) and exposes create
/// + delete mutations (ADR 0026 §14, personal-cfo-npoe). A recurring transfer
/// projects in the per-account forecast, so each mutation invalidates the forecast
/// caches alongside its own list. IPC flows through the generated `commands`; Rust
/// stays authoritative for validation (ADR 0003).
export function useRecurringTransfers() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.recurringTransfers,
    queryFn: () =>
      ipcQuery(
        commands.recurringTransferList(),
        "Could not load recurring transfers.",
      ),
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: queryKeys.recurringTransfers,
    });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
  }, [queryClient]);

  const addMutation = useMutation({
    mutationFn: (input: CreateRecurringTransferInput) =>
      commands.createRecurringTransfer(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      commands.deleteRecurringTransfer(id, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  const addRecurringTransfer = useCallback(
    async (input: CreateRecurringTransferInput): Promise<IpcError | null> => {
      const result = await addMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [addMutation],
  );
  const deleteRecurringTransfer = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await deleteMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [deleteMutation],
  );

  return {
    recurringTransfers: query.data ?? null,
    error: query.error?.message ?? null,
    addRecurringTransfer,
    deleteRecurringTransfer,
  };
}
