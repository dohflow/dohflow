import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type CreateCategoryInput,
  type IpcError,
  type MoveCategoryInput,
  type UpdateCategoryInput,
} from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the category taxonomy (TanStack Query, ADR 0020) and exposes the CRUD
/// mutations from `bac` PRs 1–3: create / update (rename+recolor) / move
/// (reparent) / archive / reinstate. Categories are reference data, not part of
/// the ledger or forecast, so each mutation invalidates only the category cache.
/// IPC flows through the generated `commands`; Rust stays authoritative for
/// validation — a system category's identity (name/parent) is fixed while its
/// appearance (color + icon) is editable, and cycles are rejected (ADR 0030, ADR 0003).
export function useCategories() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.categories,
    queryFn: () =>
      ipcQuery(commands.categoryList(), "Could not load categories."),
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.categories });
  }, [queryClient]);

  const addMutation = useMutation({
    mutationFn: (input: CreateCategoryInput) => commands.createCategory(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const updateMutation = useMutation({
    mutationFn: (input: UpdateCategoryInput) => commands.updateCategory(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const moveMutation = useMutation({
    mutationFn: (input: MoveCategoryInput) => commands.moveCategory(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const archiveMutation = useMutation({
    mutationFn: (categoryId: string) =>
      commands.archiveCategory(categoryId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const reinstateMutation = useMutation({
    mutationFn: (categoryId: string) =>
      commands.reinstateCategory(categoryId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });

  const addCategory = useCallback(
    // Returns the created id so create-from-picker can autofill the selection
    // (personal-cfo-4d8.25.19) — the command always carried it; the hook used to
    // drop it.
    async (
      input: CreateCategoryInput,
    ): Promise<{ error: IpcError | null; categoryId: string | null }> => {
      const result = await addMutation.mutateAsync(input);
      return result.status === "ok"
        ? { error: null, categoryId: result.data.category_id }
        : { error: result.error, categoryId: null };
    },
    [addMutation],
  );
  const updateCategory = useCallback(
    async (input: UpdateCategoryInput): Promise<IpcError | null> => {
      const result = await updateMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [updateMutation],
  );
  const moveCategory = useCallback(
    async (input: MoveCategoryInput): Promise<IpcError | null> => {
      const result = await moveMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [moveMutation],
  );
  const archiveCategory = useCallback(
    async (categoryId: string): Promise<IpcError | null> => {
      const result = await archiveMutation.mutateAsync(categoryId);
      return result.status === "ok" ? null : result.error;
    },
    [archiveMutation],
  );
  const reinstateCategory = useCallback(
    async (categoryId: string): Promise<IpcError | null> => {
      const result = await reinstateMutation.mutateAsync(categoryId);
      return result.status === "ok" ? null : result.error;
    },
    [reinstateMutation],
  );

  return {
    categories: query.data ?? null,
    error: query.error?.message ?? null,
    addCategory,
    updateCategory,
    moveCategory,
    archiveCategory,
    reinstateCategory,
  };
}
