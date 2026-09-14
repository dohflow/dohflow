import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type IpcError, type SplitLineInputDto } from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// One transaction's split lines (ADR 0034, personal-cfo-kr9). Disabled until a
/// transaction id is supplied, so a collapsed list row can pass `null` and not fetch.
export function useTransactionSplits(transactionId: string | null) {
  return useQuery({
    queryKey: queryKeys.transactionSplits(transactionId ?? ""),
    queryFn: () =>
      ipcQuery(
        commands.transactionSplits(transactionId as string),
        "Could not load the splits.",
      ),
    enabled: transactionId !== null,
  });
}

/// The SetSplits mutation (ADR 0034): replace a transaction's split set. Invalidates the
/// transactions list (`split_count` changes) and the transaction's own lines.
export function useSetSplits() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: ({
      transactionId,
      lines,
    }: {
      transactionId: string;
      lines: SplitLineInputDto[];
    }) => commands.setSplits(transactionId, lines, mintIdempotencyKey()),
    onSuccess: (result, variables) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.transactionSplits(variables.transactionId),
        });
      }
    },
  });
  return useCallback(
    async (
      transactionId: string,
      lines: SplitLineInputDto[],
    ): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync({ transactionId, lines });
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );
}
