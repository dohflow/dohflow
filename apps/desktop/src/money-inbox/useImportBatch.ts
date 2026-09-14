import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type BatchResultDto,
  type ImportBatchInput,
  type IpcError,
} from "@/bindings";
import { queryKeys } from "@/lib/query";

/// Drives a file import through the pipeline (personal-cfo-cu8/-zl8f): hands the
/// raw bytes + target account to `import_batch`, which auto-commits the clean rows
/// and flags the exceptions for the Money Inbox (ADR 0014). On success it
/// invalidates the inbox + every ledger-derived cache (balances, tiers,
/// availability, forecast, transactions), since a batch can both commit rows and
/// raise flags. Rust stays authoritative (ADR 0003); the bytes are never persisted
/// unencrypted (ADR 0014 shred-after-parse).
export function useImportBatch() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (input: ImportBatchInput) => commands.importBatch(input),
    onSuccess: (result) => {
      if (result.status !== "ok") return;
      void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
      void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
      void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
      void queryClient.invalidateQueries({ queryKey: queryKeys.cashTiers });
      void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
      void queryClient.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
      void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    },
  });

  const importFile = useCallback(
    async (
      input: ImportBatchInput,
    ): Promise<{ batch: BatchResultDto } | { error: IpcError }> => {
      const result = await mutation.mutateAsync(input);
      return result.status === "ok"
        ? { batch: result.data }
        : { error: result.error };
    },
    [mutation],
  );

  /// The source column headers of a file, to seed the column-mapping UI
  /// (personal-cfo-4d8.24.1.2). Empty for formats without mappable columns (OFX) or
  /// an unrecognized file; the caller then just imports with auto-detect.
  const previewColumns = useCallback(
    async (data: number[], filename: string | null): Promise<string[]> => {
      const result = await commands.importPreviewColumns(data, filename, null);
      return result.status === "ok" ? result.data : [];
    },
    [],
  );

  return { importFile, previewColumns };
}
