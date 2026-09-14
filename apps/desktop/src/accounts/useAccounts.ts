import { useCallback } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  commands,
  type AssertBalanceInput,
  type AssertBalanceResult,
  type CreateAccountInput,
  type IpcError,
  type UpdateAccountInput,
} from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

/// Loads the account list (TanStack Query, ADR 0020) and exposes mutations to
/// add, rename, and archive/reinstate accounts. Each mutation invalidates the
/// accounts + forecast caches on success (an account change can move the
/// forecast's liquid starting balance). IPC flows through the generated
/// `commands` only; Rust stays authoritative for validation (ADR 0003).
export function useAccounts() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.accounts,
    queryFn: () => ipcQuery(commands.accountList(), "Could not load accounts."),
  });

  const invalidate = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.accounts });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashTiers });
    void queryClient.invalidateQueries({ queryKey: ["forecast"] });
    void queryClient.invalidateQueries({ queryKey: queryKeys.cashAvailability });
    void queryClient.invalidateQueries({ queryKey: queryKeys.forecastReadiness });
  }, [queryClient]);

  const addMutation = useMutation({
    mutationFn: (input: CreateAccountInput) => commands.createAccount(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const updateMutation = useMutation({
    mutationFn: (input: UpdateAccountInput) => commands.updateAccount(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const archiveMutation = useMutation({
    mutationFn: (accountId: string) =>
      commands.archiveAccount(accountId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const reinstateMutation = useMutation({
    mutationFn: (accountId: string) =>
      commands.reinstateAccount(accountId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const assertMutation = useMutation({
    mutationFn: (input: AssertBalanceInput) => commands.assertBalance(input),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  const subtypeMutation = useMutation({
    mutationFn: ({
      accountId,
      subtype,
    }: {
      accountId: string;
      subtype: string | null;
    }) => commands.setAccountSubtype(accountId, subtype, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  // Convert the unexplained adjustment into a real transaction (ADR 0027 §8,
  // personal-cfo-dyy4). It writes a posting, so refresh the transactions list too.
  const convertMutation = useMutation({
    mutationFn: (accountId: string) =>
      commands.convertUnexplainedToTransaction(accountId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") {
        invalidate();
        void queryClient.invalidateQueries({ queryKey: queryKeys.transactions });
      }
    },
  });

  const addAccount = useCallback(
    async (input: CreateAccountInput): Promise<IpcError | null> => {
      const result = await addMutation.mutateAsync(input);
      return result.status === "ok" ? null : result.error;
    },
    [addMutation],
  );
  const renameAccount = useCallback(
    async (accountId: string, name: string): Promise<IpcError | null> => {
      const result = await updateMutation.mutateAsync({
        account_id: accountId,
        name,
        idempotency_key: mintIdempotencyKey(),
      });
      return result.status === "ok" ? null : result.error;
    },
    [updateMutation],
  );
  const archiveAccount = useCallback(
    async (accountId: string): Promise<IpcError | null> => {
      const result = await archiveMutation.mutateAsync(accountId);
      return result.status === "ok" ? null : result.error;
    },
    [archiveMutation],
  );
  const reinstateAccount = useCallback(
    async (accountId: string): Promise<IpcError | null> => {
      const result = await reinstateMutation.mutateAsync(accountId);
      return result.status === "ok" ? null : result.error;
    },
    [reinstateMutation],
  );
  // Set a balance directly (the additive model, ADR 0027): returns the new
  // assertion-anchored balance and the still-unexplained adjustment so the caller
  // can surface it. Unlike the other mutations, success carries data, so this
  // returns the result rather than just an error sentinel.
  const assertBalance = useCallback(
    async (
      input: AssertBalanceInput,
    ): Promise<{ result: AssertBalanceResult | null; error: IpcError | null }> => {
      const outcome = await assertMutation.mutateAsync(input);
      return outcome.status === "ok"
        ? { result: outcome.data, error: null }
        : { result: null, error: outcome.error };
    },
    [assertMutation],
  );
  // Set (or clear, with `null`) an account's subtype (ADR 0028). The kernel
  // rejects a subtype that does not belong to the account's role.
  const setSubtype = useCallback(
    async (accountId: string, subtype: string | null): Promise<IpcError | null> => {
      const result = await subtypeMutation.mutateAsync({ accountId, subtype });
      return result.status === "ok" ? null : result.error;
    },
    [subtypeMutation],
  );
  const convertUnexplained = useCallback(
    async (accountId: string): Promise<IpcError | null> => {
      const result = await convertMutation.mutateAsync(accountId);
      return result.status === "ok" ? null : result.error;
    },
    [convertMutation],
  );
  // Create an account and return its new id (the unified editor needs it to set the
  // account's subtype / debt terms / note in the same save, ADR 0044).
  const createAccount = useCallback(
    async (
      input: CreateAccountInput,
    ): Promise<{ id: string | null; error: IpcError | null }> => {
      const result = await addMutation.mutateAsync(input);
      return result.status === "ok"
        ? { id: result.data.account_id, error: null }
        : { id: null, error: result.error };
    },
    [addMutation],
  );
  const noteMutation = useMutation({
    mutationFn: ({
      accountId,
      note,
    }: {
      accountId: string;
      note: string | null;
    }) => commands.setAccountNote(accountId, note, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  // Set (or clear, with `null`) an account's free-text note (ADR 0044).
  const setAccountNote = useCallback(
    async (accountId: string, note: string | null): Promise<IpcError | null> => {
      const result = await noteMutation.mutateAsync({ accountId, note });
      return result.status === "ok" ? null : result.error;
    },
    [noteMutation],
  );
  const linkMutation = useMutation({
    mutationFn: ({
      assetId,
      liabilityId,
    }: {
      assetId: string;
      liabilityId: string | null;
    }) => commands.setAccountLink(assetId, liabilityId, mintIdempotencyKey()),
    onSuccess: (result) => {
      if (result.status === "ok") invalidate();
    },
  });
  // Link a real asset to its financing liability, or clear the link (`null`) — ADR 0044 §5.
  // The worker rejects an invalid pairing (non-real-asset source, non-liability target).
  const setAccountLink = useCallback(
    async (assetId: string, liabilityId: string | null): Promise<IpcError | null> => {
      const result = await linkMutation.mutateAsync({ assetId, liabilityId });
      return result.status === "ok" ? null : result.error;
    },
    [linkMutation],
  );

  return {
    accounts: query.data ?? null,
    error: query.error?.message ?? null,
    addAccount,
    createAccount,
    renameAccount,
    archiveAccount,
    reinstateAccount,
    assertBalance,
    setSubtype,
    setAccountNote,
    setAccountLink,
    convertUnexplained,
  };
}

/// Loads the type-based cash-tier rollups (ADR 0028): spendable / reserve / net
/// cash. Shares the `["cash-tiers"]` key that the account mutations invalidate, so
/// it refreshes whenever an account, subtype, or balance changes.
export function useCashTiers() {
  const query = useQuery({
    queryKey: queryKeys.cashTiers,
    queryFn: () => ipcQuery(commands.cashTiers(), "Could not load cash tiers."),
  });
  return {
    tiers: query.data ?? null,
    error: query.error?.message ?? null,
  };
}
