import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError, type TagViewDto } from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the tag vocabulary (ADR 0033, personal-cfo-2ryf) and exposes the tag/note
/// mutations: create a tag, set a transaction's tag set, set a transaction's note.
/// Creating a tag invalidates the tag list; tag/note changes invalidate the
/// transactions list (the row carries `tag_ids` + `note`). No balance moves, so the
/// financial caches are left alone. IPC flows through the generated `commands` only
/// (ADR 0003).
export function useTags() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.tags,
    queryFn: () => ipcQuery(commands.tagList(), "Could not load your tags."),
  });

  const invalidateTags = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.tags });
  }, [queryClient]);
  const invalidateTransactions = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
  }, [queryClient]);

  const createTagMutation = useMutation({
    mutationFn: ({ name, color }: { name: string; color: string | null }) =>
      commands.createTag(name, color, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidateTags();
    },
  });
  const setTagsMutation = useMutation({
    mutationFn: ({
      transactionId,
      tagIds,
    }: {
      transactionId: string;
      tagIds: string[];
    }) => commands.setTags(transactionId, tagIds, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidateTransactions();
    },
  });
  const setNoteMutation = useMutation({
    mutationFn: ({
      transactionId,
      note,
    }: {
      transactionId: string;
      note: string | null;
    }) => commands.setNote(transactionId, note, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidateTransactions();
    },
  });

  /// Create a tag and return its new id (or an `IpcError`); the caller assigns it.
  const createTag = useCallback(
    async (
      name: string,
      color: string | null = null,
    ): Promise<string | IpcError> => {
      const result = await createTagMutation.mutateAsync({ name, color });
      return result.status === "ok" ? result.data.tag_id : result.error;
    },
    [createTagMutation],
  );
  const setTags = useCallback(
    async (transactionId: string, tagIds: string[]): Promise<IpcError | null> => {
      const result = await setTagsMutation.mutateAsync({ transactionId, tagIds });
      return result.status === "ok" ? null : result.error;
    },
    [setTagsMutation],
  );
  const setNote = useCallback(
    async (
      transactionId: string,
      note: string | null,
    ): Promise<IpcError | null> => {
      const result = await setNoteMutation.mutateAsync({ transactionId, note });
      return result.status === "ok" ? null : result.error;
    },
    [setNoteMutation],
  );

  return {
    tags: (query.data ?? null) as TagViewDto[] | null,
    error: query.error?.message ?? null,
    createTag,
    setTags,
    setNote,
  };
}
