import { useCallback } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type BatchResultDto,
  type ColumnMappingDto,
  type CreateAccountInput,
  type ImportBatchInput,
  type IpcError,
  type SourcePresetDto,
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
    async (
      data: number[],
      filename: string | null,
      pluginId: string | null = null,
    ): Promise<string[]> => {
      const result = await commands.importPreviewColumns(data, filename, pluginId);
      return result.status === "ok" ? result.data : [];
    },
    [],
  );

  /// The distinct source accounts in a file (personal-cfo-tulv) — read the way
  /// the import will read it (the preset's importer and hints, the user's
  /// mapping on top). Empty for a file with no account column; the dialog then
  /// imports everything into the one chosen account, as before.
  const previewAccounts = useCallback(
    async (
      data: number[],
      filename: string | null,
      presetId: string | null,
      columnMapping: ColumnMappingDto | null,
    ): Promise<string[]> => {
      const result = await commands.importPreviewAccounts(
        data,
        filename,
        null,
        presetId,
        columnMapping,
      );
      return result.status === "ok" ? result.data : [];
    },
    [],
  );

  /// Create an account from the import's account-mapping step
  /// (personal-cfo-tulv), refreshing the account list so it can be picked.
  const createAccount = useCallback(
    async (
      input: CreateAccountInput,
    ): Promise<{ id: string | null; error: IpcError | null }> => {
      const result = await commands.createAccount(input);
      if (result.status !== "ok") return { id: null, error: result.error };
      void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
      return { id: result.data.account_id, error: null };
    },
    [queryClient],
  );

  /// Every registered source-app preset (personal-cfo-gvidg), for the
  /// "Import from <app>" picker. The registry is compile-time and doesn't
  /// change during a session, so this is a plain callback (like
  /// `previewColumns`) rather than a cached query — nothing ever
  /// invalidates it.
  const listPresets = useCallback(async (): Promise<SourcePresetDto[]> => {
    return commands.listSourcePresets();
  }, []);

  return { importFile, previewColumns, previewAccounts, listPresets, createAccount };
}
