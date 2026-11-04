import { useCallback, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError } from "@/bindings";
import { ipcQuery } from "@/lib/query";

/// Query key for a transaction's attachments (ADR 0023). Scoped by id so each
/// transaction's list caches independently and invalidates on attach/remove.
function attachmentsKey(transactionId: string) {
  return ["attachments", "transaction", transactionId] as const;
}

/// Read a picked `File` into the `number[]` the `attach_document` IPC expects,
/// then attach it to the transaction. The bytes are encrypted in the worker and
/// never written outside the vault (ADR 0023). Exported so the add-transaction
/// one-flow can attach staged files after the record (personal-cfo-4d8.24.2.3).
export async function attach(transactionId: string, file: File) {
  return sendAttachment(transactionId, file, await readFileBytes(file));
}

/// Reading the picked file. A failure here happens before anything is sent, so the file is known
/// not to be attached (personal-cfo-g3m.35).
export async function readFileBytes(file: File): Promise<number[]> {
  return Array.from(new Uint8Array(await file.arrayBuffer()));
}

/// The call itself. If it throws rather than returns, whether the host stored the file is unknown.
export function sendAttachment(transactionId: string, file: File, bytes: number[]) {
  return commands.attachDocument(
    transactionId,
    file.name || null,
    file.type || null,
    bytes,
  );
}

/// Loads a transaction's attachments and exposes `attachFile` / `removeAttachment`
/// (TanStack Query, ADR 0020). Mutations invalidate the list so it refreshes.
export function useTransactionAttachments(transactionId: string) {
  const queryClient = useQueryClient();
  const key = attachmentsKey(transactionId);

  const query = useQuery({
    queryKey: key,
    queryFn: () =>
      ipcQuery(
        commands.transactionAttachments(transactionId),
        "Could not load attachments.",
      ),
  });

  // A retry of a read that never produced data puts the query back to pending, which clears its error
  // and would flip the list to "Loading…"; the failure being retried stays on screen meanwhile.
  const [retriedFailure, setRetriedFailure] = useState<string | null>(null);
  const [retrying, setRetrying] = useState(false);
  const failure = query.error?.message ?? (retrying ? retriedFailure : null);
  /// Read the list again after a failed read.
  const retry = useCallback(async () => {
    setRetriedFailure(query.error?.message ?? null);
    setRetrying(true);
    try {
      await query.refetch();
    } finally {
      setRetrying(false);
    }
  }, [query]);
  /// Read the list again so it shows what is stored, e.g. after a call whose outcome is unknown.
  /// Settles once the read has answered, whether or not it worked.
  const refresh = useCallback(
    () => queryClient.refetchQueries({ queryKey: key }),
    [queryClient, key],
  );

  const attachMutation = useMutation({
    mutationFn: (file: File) => attach(transactionId, file),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: key });
      }
    },
  });

  const removeMutation = useMutation({
    mutationFn: (attachmentId: string) =>
      commands.removeAttachment(attachmentId, transactionId),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: key });
      }
    },
  });

  const attachFile = useCallback(
    async (file: File): Promise<IpcError | null> => {
      const result = await attachMutation.mutateAsync(file);
      return result.status === "ok" ? null : result.error;
    },
    [attachMutation],
  );

  const removeAttachment = useCallback(
    async (attachmentId: string): Promise<IpcError | null> => {
      const result = await removeMutation.mutateAsync(attachmentId);
      return result.status === "ok" ? null : result.error;
    },
    [removeMutation],
  );

  return {
    attachments: query.data ?? null,
    error: failure,
    retry,
    retrying,
    refresh,
    attachFile,
    removeAttachment,
    isAttaching: attachMutation.isPending,
  };
}
