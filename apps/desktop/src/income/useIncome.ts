import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateIncomeSourceInput,
  type IpcError,
  type UpdateIncomeSourceInput,
} from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the recurring income-source list (TanStack Query, ADR 0020) and exposes
/// add / edit / delete / archive / restore mutations. Each invalidates the income +
/// forecast caches on success (income feeds the forecast). IPC flows through the
/// generated `commands` only; Rust stays authoritative for validation (ADR 0003).
export function useIncome() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.income,
    queryFn: () =>
      ipcQuery(commands.incomeSourceList(), "Could not load income sources."),
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.income });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
    void queryClient.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
  }, [queryClient]);

  const addMutation = useMutation({
    mutationFn: (input: CreateIncomeSourceInput) =>
      commands.createIncomeSource(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const updateMutation = useMutation({
    mutationFn: (input: UpdateIncomeSourceInput) =>
      commands.updateIncomeSource(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      commands.deleteIncomeSource(id, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const archiveMutation = useMutation({
    mutationFn: (id: string) =>
      commands.archiveIncomeSource(id, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const restoreMutation = useMutation({
    mutationFn: (id: string) =>
      commands.restoreIncomeSource(id, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  const addIncomeSource = useCallback(
    async (input: CreateIncomeSourceInput): Promise<IpcError | null> => {
      const result = await addMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [addMutation],
  );
  const updateIncomeSource = useCallback(
    async (input: UpdateIncomeSourceInput): Promise<IpcError | null> => {
      const result = await updateMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [updateMutation],
  );
  const deleteIncomeSource = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await deleteMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [deleteMutation],
  );
  const archiveIncomeSource = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await archiveMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [archiveMutation],
  );
  const restoreIncomeSource = useCallback(
    async (id: string): Promise<IpcError | null> => {
      const result = await restoreMutation.mutateAsync(id);
      return result.status === "ok" ? null : result.error;
    },
    [restoreMutation],
  );

  return {
    sources: query.data ?? null,
    error: query.error?.message ?? null,
    addIncomeSource,
    updateIncomeSource,
    deleteIncomeSource,
    archiveIncomeSource,
    restoreIncomeSource,
  };
}
