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

/// What the user chose about a loan while adding a bill (personal-cfo-6wk.22): link the new bill to
/// it, or remember that it is NOT that loan's payment.
export type BillFollowUp = {
  linkLoanAccountId?: string;
  notPaymentOfLoanAccountId?: string;
};

/// The bill-to-loan actions (personal-cfo-6wk.22) without the bill list query, so a surface that only
/// links (the overlap warning) does not start a list load. Each refreshes the bills, the forecast
/// (which includes the overlap warning) and the cash views.
export function useBillLoanActions() {
  const queryClient = useQueryClient();
  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.bills });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
    void queryClient.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
  }, [queryClient]);
  /// Link a bill to the loan it pays, so the forecast counts the obligation once.
  const linkBill = useCallback(
    async (billId: string, loanAccountId: string): Promise<IpcError | null> => {
      const result = await commands.linkBillToLoan(billId, loanAccountId, mintIdempotencyKey());
      invalidate();
      return result.status === "ok" ? null : result.error;
    },
    [invalidate],
  );
  /// Remove a bill's link: the loan's own payment counts again.
  const unlinkBill = useCallback(
    async (billId: string): Promise<IpcError | null> => {
      const result = await commands.unlinkBillFromLoan(billId, mintIdempotencyKey());
      invalidate();
      return result.status === "ok" ? null : result.error;
    },
    [invalidate],
  );
  /// Forget the answer "this isn't that loan's payment" (the overlap note may return).
  const forgetNotLoanPayment = useCallback(
    async (billId: string): Promise<IpcError | null> => {
      const result = await commands.setBillNotLoanPayment(billId, null, mintIdempotencyKey());
      invalidate();
      return result.status === "ok" ? null : result.error;
    },
    [invalidate],
  );
  return { linkBill, unlinkBill, forgetNotLoanPayment };
}

/// Loads the recurring-bill list (TanStack Query, ADR 0020) and exposes add /
/// edit / delete mutations. Each mutation invalidates the bills + forecast caches
/// on success (bills feed the forecast). IPC flows through the generated
/// `commands` only; Rust stays authoritative for validation (ADR 0003).
export function useBills() {
  const queryClient = useQueryClient();
  const { linkBill, unlinkBill, forgetNotLoanPayment } = useBillLoanActions();
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
      follow?: BillFollowUp,
    ): Promise<{ error: IpcError | null; eventId: string | null; followError: IpcError | null }> => {
      const result = await addMutation.mutateAsync(input);
      if (result.status !== "ok") return { error: result.error, eventId: null, followError: null };
      // The created event id lets callers fetch the bill's retro-attached history
      // right after approval (ADR 0047 §1, personal-cfo-4d8.25.8).
      const eventId = result.data.event_id;
      // What the user chose about the loan while adding (personal-cfo-6wk.22): link the new bill to
      // it ("Add and link"), or remember that it is NOT that loan's payment ("No, it's a different
      // bill"). The bill already exists, so a failure here is returned, not thrown: the bill stays,
      // unlinked, and the overlap warning still offers "Link it" afterwards.
      let followError: IpcError | null = null;
      if (follow?.linkLoanAccountId) {
        const linked = await commands.linkBillToLoan(
          eventId,
          follow.linkLoanAccountId,
          mintIdempotencyKey(),
        );
        if (linked.status !== "ok") followError = linked.error;
      } else if (follow?.notPaymentOfLoanAccountId) {
        const answered = await commands.setBillNotLoanPayment(
          eventId,
          follow.notPaymentOfLoanAccountId,
          mintIdempotencyKey(),
        );
        if (answered.status !== "ok") followError = answered.error;
      }
      if (follow) invalidate();
      return { error: null, eventId, followError };
    },
    [addMutation, invalidate],
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
  /// Bring back a removed bill (the Undo of a remove; personal-cfo-egn67). Calls the command
  /// directly, not through a mutation owned by this component, because the notice that holds the
  /// Undo outlives the screen the removal happened on.
  const reinstateBill = useCallback(
    async (billId: string): Promise<IpcError | null> => {
      const result = await commands.reinstateRecurringBill(billId, mintIdempotencyKey());
      if (result.status === "ok") invalidate();
      return result.status === "ok" ? null : result.error;
    },
    [invalidate],
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
    linkBill,
    unlinkBill,
    forgetNotLoanPayment,
    updateBill,
    deleteBill,
    reinstateBill,
    archiveBill,
    restoreBill,
  };
}
