import { useCallback } from "react";
import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import {
  commands,
  type IpcError,
  type RecordTransactionInput,
  type RecordTransferInput,
  type TransactionPageInput,
} from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";
import { attach as attachDocumentFile } from "./useTransactionAttachments";

/// One server-side page of the filtered transaction list (personal-cfo-3fdd.1).
/// Keyed under the `transactions` prefix so every mutation that invalidates the
/// plain list refetches open pages too; `keepPreviousData` holds the previous
/// page on screen while the next loads (no flash on page/filter changes).
export function useTransactionPage(
  input: TransactionPageInput,
  options: { enabled?: boolean } = {},
) {
  return useQuery({
    queryKey: [...queryKeys.transactions, "page", input] as const,
    queryFn: () =>
      ipcQuery(commands.transactionPage(input), "Could not load transactions."),
    placeholderData: keepPreviousData,
    enabled: options.enabled ?? true,
  });
}

/// Loads the recent-transactions list (TanStack Query, ADR 0020) and exposes an
/// `addTransaction` mutation. Recording a posting invalidates transactions,
/// accounts (a balance moved), and the forecast (balances feed it). IPC flows
/// through the generated `commands` only.
///
/// `list: false` skips the plain recent-list fetch for callers that only need the
/// mutations (the server-paged views, personal-cfo-3fdd.1); the Money Inbox keeps
/// the default list for its drawer-from-inbox row lookups.
export function useTransactions({ list = true }: { list?: boolean } = {}) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.transactions,
    queryFn: () =>
      ipcQuery(commands.transactionList(), "Could not load transactions."),
    enabled: list,
  });
  const mutation = useMutation({
    mutationFn: (input: RecordTransactionInput) =>
      commands.recordTransaction(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.cashAvailability,
        });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.forecastReadiness,
        });
      }
    },
  });
  const addTransaction = useCallback(
    async (input: RecordTransactionInput): Promise<IpcError | null> => {
      const result = await mutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [mutation],
  );

  // Set or clear a transaction's category (ADR 0030, personal-cfo-bac). The
  // assignment is metadata — no ledger/forecast change — so only the transactions
  // list is invalidated.
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
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
      }
    },
  });
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

  // Record a manual transaction and attach category/tags/note to the returned id in
  // ONE flow (personal-cfo-4d8.24.2). record_transaction returns the new id
  // (4d8.24.2.1); we thread it into the metadata follow-ups. setTags/setNote live in
  // useTags, so the caller injects them — this keeps useTransactions free of a useTags
  // dependency (no double tag query). Each follow-up runs only when its field is set.
  //
  // Partial-failure: the row is durably recorded the moment record_transaction returns
  // ok, so we never roll it back (the money entry must not vanish because a tag write
  // failed; category/tags/note stay editable from the drawer). We report the FIRST
  // follow-up error as a "partial" outcome so the form can surface a non-blocking
  // warning while still closing + refreshing.
  const addTransactionWithMeta = useCallback(
    async (
      input: RecordTransactionInput,
      meta: { categoryId: string | null; tagIds: string[]; note: string | null },
      followUps: {
        setTags: (
          transactionId: string,
          tagIds: string[],
        ) => Promise<IpcError | null>;
        setNote: (
          transactionId: string,
          note: string | null,
        ) => Promise<IpcError | null>;
      },
      files: File[],
    ): Promise<
      | { status: "ok" }
      | { status: "record-failed"; error: IpcError }
      | { status: "partial"; error: IpcError }
    > => {
      const result = await mutation.mutateAsync(input);
      if (result.status !== "ok") {
        return { status: "record-failed", error: result.error };
      }
      const id = result.data.transaction_id;

      // Sequential so an early failure short-circuits later calls; the returned error is
      // the first meaningful one. Only non-empty metadata triggers a write.
      let firstError: IpcError | null = null;
      if (meta.categoryId !== null) {
        const failure = await recategorize(id, meta.categoryId);
        if (failure && !firstError) firstError = failure;
      }
      if (meta.tagIds.length > 0) {
        const failure = await followUps.setTags(id, meta.tagIds);
        if (failure && !firstError) firstError = failure;
      }
      if (meta.note !== null && meta.note.trim() !== "") {
        const failure = await followUps.setNote(id, meta.note.trim());
        if (failure && !firstError) firstError = failure;
      }
      // Attachments last (personal-cfo-4d8.24.2.3): the row is already durably recorded, so
      // an attach failure degrades to "partial" exactly like tags/note — never a rollback.
      // Sequential; the first failure short-circuits the rest and becomes the reported error.
      for (const file of files) {
        const result = await attachDocumentFile(id, file);
        if (result.status !== "ok") {
          if (!firstError) firstError = result.error;
          break;
        }
      }
      return firstError
        ? { status: "partial", error: firstError }
        : { status: "ok" };
    },
    [mutation, recategorize],
  );

  // Auto-categorize uncategorized transactions from merchant memory (ADR 0030 addendum,
  // personal-cfo-5n4.1): learn merchant→category from the user's manual categorizations and
  // fill same-merchant uncategorized rows as source=rule. User-triggered (never silent), so
  // the result count is surfaced to the user. Filling categories changes coverage, which feeds
  // the forecast readiness factors, so both caches are invalidated.
  const autoCategorizeMutation = useMutation({
    mutationFn: () => commands.applyMerchantMemory(),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.forecastReadiness,
        });
      }
    },
  });
  const autoCategorize = useCallback(async (): Promise<
    { count: number } | { error: IpcError }
  > => {
    const result = await autoCategorizeMutation.mutateAsync();
    return result.status === "ok"
      ? { count: result.data }
      : { error: result.error };
  }, [autoCategorizeMutation]);

  // Record a transfer between two accounts (personal-cfo-npoe). It moves both
  // balances and shifts the spendable/reserve split, so it invalidates the same
  // ledger-derived caches as a transaction plus the cash tiers.
  const transferMutation = useMutation({
    mutationFn: (input: RecordTransferInput) => commands.recordTransfer(input),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
        void queryClient.invalidateQueries({ queryKey: queryKeys.cashTiers });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.cashAvailability,
        });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.forecastReadiness,
        });
      }
    },
  });
  const transfer = useCallback(
    async (input: RecordTransferInput): Promise<IpcError | null> => {
      const result = await transferMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [transferMutation],
  );

  // Delete a transaction by voiding it (personal-cfo-4d8.11, ADR 0007 §9). A reversing
  // entry moves balances like a transaction does, so it invalidates the same
  // ledger-derived caches.
  const deleteMutation = useMutation({
    mutationFn: (transactionId: string) =>
      commands.voidTransaction(transactionId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
        void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
        void queryClient.invalidateQueries({ queryKey: queryKeys.cashTiers });
        void queryClient.invalidateQueries({ queryKey: ["forecast"] });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.cashAvailability,
        });
        void queryClient.invalidateQueries({
          queryKey: queryKeys.forecastReadiness,
        });
      }
    },
  });
  const deleteTransaction = useCallback(
    async (transactionId: string): Promise<IpcError | null> => {
      const result = await deleteMutation.mutateAsync(transactionId);
      return result.status === "ok" ? null : result.error;
    },
    [deleteMutation],
  );

  // Mark a transaction reviewed/unreviewed (personal-cfo-4d8.7, ADR 0032). Reviewed-state
  // is metadata — no balance change — so it refreshes the transactions list and the Money
  // Inbox (which surfaces the unreviewed ones).
  const setReviewedMutation = useMutation({
    mutationFn: ({
      transactionId,
      reviewed,
    }: {
      transactionId: string;
      reviewed: boolean;
    }) => commands.markReviewed(transactionId, reviewed, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") {
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
        void queryClient.invalidateQueries({ queryKey: queryKeys.moneyInbox });
      }
    },
  });
  const setReviewed = useCallback(
    async (transactionId: string, reviewed: boolean): Promise<IpcError | null> => {
      const result = await setReviewedMutation.mutateAsync({
        transactionId,
        reviewed,
      });
      return result.status === "ok" ? null : result.error;
    },
    [setReviewedMutation],
  );

  return {
    transactions: query.data ?? null,
    error: query.error?.message ?? null,
    addTransaction,
    addTransactionWithMeta,
    recategorize,
    autoCategorize,
    transfer,
    deleteTransaction,
    setReviewed,
  };
}
