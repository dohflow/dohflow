import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateRecurringBillInput,
  type IpcError,
  type UpdateRecurringBillInput,
} from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the recurring-bill list (TanStack Query, ADR 0020) and exposes add /
/// edit / delete mutations. Each mutation invalidates the bills + forecast caches
/// on success (bills feed the forecast). IPC flows through the generated
/// `commands` only; Rust stays authoritative for validation (ADR 0003).
export function useBills() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.bills,
    queryFn: () =>
      ipcQuery(commands.recurringBillList(), "Could not load bills."),
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.bills });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
    void queryClient.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
  }, [queryClient]);

  const addMutation = useMutation({
    mutationFn: (input: CreateRecurringBillInput) =>
      commands.createRecurringBill(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const updateMutation = useMutation({
    mutationFn: (input: UpdateRecurringBillInput) =>
      commands.updateRecurringBill(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (billId: string) =>
      commands.deleteRecurringBill(billId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const archiveMutation = useMutation({
    mutationFn: (billId: string) =>
      commands.archiveRecurringBill(billId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const restoreMutation = useMutation({
    mutationFn: (billId: string) =>
      commands.restoreRecurringBill(billId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  const addBill = useCallback(
    async (
      input: CreateRecurringBillInput,
    ): Promise<{ error: IpcError | null; eventId: string | null }> => {
      const result = await addMutation.mutateAsync(input);
      // The created event id lets callers fetch the bill's retro-attached history
      // right after approval (ADR 0047 §1, personal-cfo-4d8.25.8).
      return result.status === "ok"
        ? { error: null, eventId: result.data.event_id }
        : { error: result.error, eventId: null };
    },
    [addMutation],
  );
  const updateBill = useCallback(
    async (input: UpdateRecurringBillInput): Promise<IpcError | null> => {
      const result = await updateMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [updateMutation],
  );
  const deleteBill = useCallback(
    async (billId: string): Promise<IpcError | null> => {
      const result = await deleteMutation.mutateAsync(billId);
      return result.status === "ok" ? null : result.error;
    },
    [deleteMutation],
  );
  const archiveBill = useCallback(
    async (billId: string): Promise<IpcError | null> => {
      const result = await archiveMutation.mutateAsync(billId);
      return result.status === "ok" ? null : result.error;
    },
    [archiveMutation],
  );
  const restoreBill = useCallback(
    async (billId: string): Promise<IpcError | null> => {
      const result = await restoreMutation.mutateAsync(billId);
      return result.status === "ok" ? null : result.error;
    },
    [restoreMutation],
  );

  return {
    bills: query.data ?? null,
    error: query.error?.message ?? null,
    addBill,
    updateBill,
    deleteBill,
    archiveBill,
    restoreBill,
  };
}
