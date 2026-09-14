import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands, type ConnectorSyncResultDto,
  type IpcError } from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the Money Inbox triage items (TanStack Query, ADR 0020) and exposes the
/// two resolutions for the imported-waiting-commit kind: **import anyway**
/// (force-commit the flagged duplicate) and **skip** (personal-cfo-asqy). Both
/// invalidate the inbox so the resolved item drops; import-anyway also writes the
/// ledger, so it invalidates the financial caches (balances, tiers, availability,
/// forecast, transactions). IPC flows through the generated `commands` only; Rust
/// stays authoritative (ADR 0003).
export function useMoneyInbox() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.moneyInbox,
    queryFn: () =>
      ipcQuery(commands.moneyInboxList(), "Could not load your Money Inbox."),
  });

  const invalidateInbox = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
  }, [queryClient]);

  // Committing a flagged row to the ledger shifts balances downstream, so refresh
  // every cache derived from the ledger (mirrors the transaction mutations).
  const invalidateFinancial = useCallback(() => {
    invalidateInbox();
    void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
    void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashTiers });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
    void queryClient.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
  }, [invalidateInbox, queryClient]);

  const importAnywayMutation = useMutation({
    mutationFn: (stagedTransactionId: string) =>
      commands.importStagedAnyway(stagedTransactionId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidateFinancial();
    },
  });
  const skipMutation = useMutation({
    mutationFn: (stagedTransactionId: string) =>
      commands.skipStagedTransaction(stagedTransactionId, mintIdempotencyKey()),
    onSuccess: (result) => {
      // Skip changes no ledger state — only the inbox needs refreshing.
      if (result.status === "ok") invalidateInbox();
    },
  });

  // Snooze (hide until a date) and dismiss (with a typed reason) are soft actions
  // (ADR 0014 §7, personal-cfo-ci71): they change no ledger state, so only the
  // inbox is refreshed. Keyed by the inbox *item id*, not the staged id.
  const snoozeMutation = useMutation({
    mutationFn: ({ itemId, until }: { itemId: string; until: string }) =>
      commands.snoozeInboxItem(itemId, until, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidateInbox();
    },
  });
  const dismissMutation = useMutation({
    mutationFn: ({ itemId, reason }: { itemId: string; reason: string }) =>
      commands.dismissInboxItem(itemId, reason, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidateInbox();
    },
  });

  // Mark an unreviewed-transaction item reviewed (ADR 0032, personal-cfo-4d8.7): clears
  // the review item and flips the transaction's reviewed flag — no balance change, so
  // only the inbox + transactions list refresh. Keyed by the transaction id (the item's
  // `target_id`).
  const markReviewedMutation = useMutation({
    mutationFn: (transactionId: string) =>
      commands.markReviewed(transactionId, true, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") {
        invalidateInbox();
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
      }
    },
  });

  // Recategorize a low-confidence item's transaction (ADR 0030, personal-cfo-j5ij): picking a
  // category rewrites it as source=user, which drops it from the review queue. No balance
  // change, so only the inbox + transactions list refresh.
  const recategorizeMutation = useMutation({
    mutationFn: ({
      transactionId,
      categoryId,
    }: {
      transactionId: string;
      categoryId: string | null;
    }) =>
      commands.recategorizeTransaction(
        transactionId,
        categoryId,
        mintIdempotencyKey(),
      ),
    onSuccess: (result) => {
      if (result.status === "ok") {
        invalidateInbox();
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
      }
    },
  });

  // Bulk-accept the whole low-confidence-category queue (ADR 0030 addendum, personal-cfo-j5ij):
  // marks every queued transaction reviewed (keeping its rule category). Returns the count so
  // the UI can confirm "Accepted N".
  const acceptAllLowConfidenceMutation = useMutation({
    mutationFn: () => commands.acceptLowConfidenceCategories(mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") {
        invalidateInbox();
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
      }
    },
  });

  const syncConnectionMutation = useMutation({
    mutationFn: (connectionId: string) =>
      commands.connectorSync({
        connection_id: connectionId,
        idempotency_key: mintIdempotencyKey(),
      }),
    onSuccess: (result) => {
      // A retry that succeeds clears last_error and may commit ledger rows;
      // the Settings card reads the same connection rows, so its cache must
      // refresh too (lib/query contract on the connections key).
      if (result.status === "ok") {
        invalidateFinancial();
        void queryClient.invalidateQueries({ queryKey: queryKeys.connections });
      }
    },
  });

  const importAnyway = useCallback(
    async (stagedTransactionId: string): Promise<IpcError | null> => {
      const result = await importAnywayMutation.mutateAsync(stagedTransactionId);
      return result.status === "ok" ? null : result.error;
    },
    [importAnywayMutation],
  );
  const syncConnection = useCallback(
    async (connectionId: string): Promise<ConnectorSyncResultDto | IpcError> => {
      const result = await syncConnectionMutation.mutateAsync(connectionId);
      return result.status === "ok" ? result.data : result.error;
    },
    [syncConnectionMutation],
  );
  const skip = useCallback(
    async (stagedTransactionId: string): Promise<IpcError | null> => {
      const result = await skipMutation.mutateAsync(stagedTransactionId);
      return result.status === "ok" ? null : result.error;
    },
    [skipMutation],
  );
  const snooze = useCallback(
    async (itemId: string, until: string): Promise<IpcError | null> => {
      const result = await snoozeMutation.mutateAsync({ itemId, until });
      return result.status === "ok" ? null : result.error;
    },
    [snoozeMutation],
  );
  const dismiss = useCallback(
    async (itemId: string, reason: string): Promise<IpcError | null> => {
      const result = await dismissMutation.mutateAsync({ itemId, reason });
      return result.status === "ok" ? null : result.error;
    },
    [dismissMutation],
  );
  const markReviewed = useCallback(
    async (transactionId: string): Promise<IpcError | null> => {
      const result = await markReviewedMutation.mutateAsync(transactionId);
      return result.status === "ok" ? null : result.error;
    },
    [markReviewedMutation],
  );
  const recategorize = useCallback(
    async (
      transactionId: string,
      categoryId: string | null,
    ): Promise<IpcError | null> => {
      const result = await recategorizeMutation.mutateAsync({
        transactionId,
        categoryId,
      });
      return result.status === "ok" ? null : result.error;
    },
    [recategorizeMutation],
  );
  const markReviewedBulk = useMarkReviewedBulk();

  const acceptAllLowConfidence = useCallback(async (): Promise<
    { count: number } | { error: IpcError }
  > => {
    const result = await acceptAllLowConfidenceMutation.mutateAsync();
    return result.status === "ok"
      ? { count: result.data }
      : { error: result.error };
  }, [acceptAllLowConfidenceMutation]);

  return {
    items: query.data ?? null,
    error: query.error?.message ?? null,
    importAnyway,
    syncConnection,
    skip,
    snooze,
    dismiss,
    markReviewed,
    recategorize,
    acceptAllLowConfidence,
    markReviewedBulk,
  };
}

/// Bulk mark an explicit id set reviewed in ONE round-trip (personal-cfo-4d8.25.16):
/// the kernel dispatches one audited MarkReviewed per id — no frontend fan-out. A
/// standalone hook so the TransactionsHub's bulk bar can use it without pulling the
/// whole inbox query.
export function useMarkReviewedBulk() {
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (transactionIds: string[]) =>
      commands.markInboxReviewedBulk(mintIdempotencyKey(), transactionIds),
    // The kernel applies marks one-by-one and aborts on the first failure, so a
    // PARTIAL failure has still reviewed earlier ids — invalidate unconditionally
    // or the list keeps showing reviewed rows as unreviewed (adversarial review
    // of 4d8.25.16).
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
      void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
    },
  });
  return useCallback(
    async (
      transactionIds: string[],
    ): Promise<{ count: number } | { error: IpcError }> => {
      const result = await mutation.mutateAsync(transactionIds);
      return result.status === "ok"
        ? { count: result.data }
        : { error: result.error };
    },
    [mutation],
  );
}
