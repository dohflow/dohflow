// Bank-connection state for the Settings Connections card (personal-cfo-ul5d,
// ADR 0060): the connections list plus the four lifecycle mutations. A sync
// writes real ledger rows, so its success invalidates the financial caches the
// same way an import does.
//
// LEAK RULE: the setup token never enters a useMutation — TanStack retains
// mutation variables in its cache (and exposes them via devtools), so link()
// is a direct command call with hand-rolled pending state (the
// ChangePasswordCard precedent). The token exists only for the call's
// duration.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";

import {
  commands,
  type ConnectorConnectionDto,
  type ConnectorLinkResultDto,
  type ConnectorSyncResultDto,
  type IpcError,
} from "@/bindings";
import { mintIdempotencyKey } from "@/lib/idempotency";
import { ipcQuery, queryKeys } from "@/lib/query";

interface UseConnections {
  connections: ConnectorConnectionDto[] | null;
  error: string | null;
  refreshing: boolean;
  link: (setupToken: string) => Promise<ConnectorLinkResultDto | IpcError>;
  linkPending: boolean;
  setAccountLink: (
    connectionId: string,
    externalId: string,
    accountId: string | null,
  ) => Promise<IpcError | null>;
  sync: (connectionId: string) => Promise<ConnectorSyncResultDto | IpcError>;
  /// The connection currently syncing, or null — per-row pending state (the
  /// backend's in-flight claim is per-connection, so other rows stay live).
  syncingId: string | null;
  forget: (connectionId: string) => Promise<IpcError | null>;
}

/// Everything a completed sync can have changed, in one place so the Settings
/// card and the Money Inbox retry stay in lockstep (lib/query contract).
export function invalidateAfterConnectorSync(
  queryClient: ReturnType<typeof useQueryClient>,
): void {
  for (const key of [
    queryKeys.connections,
    queryKeys.accounts,
    queryKeys.transactions,
    queryKeys.moneyInbox,
    queryKeys.cashTiers,
    queryKeys.cashAvailability,
    queryKeys.forecastReadiness,
    // Suggestion surfaces derive from the ledger a sync just changed
    // (income/bills candidate keys share these prefixes; nhmsg review).
    queryKeys.bills,
    queryKeys.income,
    ["forecast"] as const,
  ]) {
    void queryClient.invalidateQueries({ queryKey: key });
  }
}

export function useConnections(): UseConnections {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: queryKeys.connections,
    queryFn: () =>
      ipcQuery(commands.connectorConnections(), "Could not load connections."),
  });

  const invalidateConnections = () =>
    queryClient.invalidateQueries({ queryKey: queryKeys.connections });

  const [linkPending, setLinkPending] = useState(false);
  const link = useCallback(
    async (setupToken: string) => {
      setLinkPending(true);
      try {
        const result = await commands.connectorLink({
          adapter_id: "simplefin",
          setup_token: setupToken,
        });
        if (result.status === "ok") {
          void invalidateConnections();
          return result.data;
        }
        return result.error;
      } finally {
        setLinkPending(false);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [queryClient],
  );

  const setLinkMutation = useMutation({
    mutationFn: (input: {
      connectionId: string;
      externalId: string;
      accountId: string | null;
    }) =>
      commands.connectorSetAccountLink({
        connection_id: input.connectionId,
        external_id: input.externalId,
        account_id: input.accountId,
      }),
    onSuccess: invalidateConnections,
  });
  const setAccountLink = useCallback(
    async (connectionId: string, externalId: string, accountId: string | null) => {
      const result = await setLinkMutation.mutateAsync({
        connectionId,
        externalId,
        accountId,
      });
      return result.status === "ok" ? null : result.error;
    },
    [setLinkMutation],
  );

  const syncMutation = useMutation({
    mutationFn: (connectionId: string) =>
      commands.connectorSync({
        connection_id: connectionId,
        // Minted per submit, never at render (lib/idempotency doc): two syncs
        // must never share a key.
        idempotency_key: mintIdempotencyKey(),
      }),
    onSuccess: () => invalidateAfterConnectorSync(queryClient),
  });
  const sync = useCallback(
    async (connectionId: string) => {
      const result = await syncMutation.mutateAsync(connectionId);
      return result.status === "ok" ? result.data : result.error;
    },
    [syncMutation],
  );

  const forgetMutation = useMutation({
    mutationFn: (connectionId: string) =>
      commands.connectorForget({ connection_id: connectionId }),
    onSuccess: invalidateConnections,
  });
  const forget = useCallback(
    async (connectionId: string) => {
      const result = await forgetMutation.mutateAsync(connectionId);
      return result.status === "ok" ? null : result.error;
    },
    [forgetMutation],
  );

  return {
    connections: query.data ?? null,
    error: query.error?.message ?? null,
    refreshing: query.isFetching,
    link,
    linkPending,
    setAccountLink,
    sync,
    syncingId: syncMutation.isPending ? (syncMutation.variables ?? null) : null,
    forget,
  };
}
