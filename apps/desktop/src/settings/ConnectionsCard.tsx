// The Connections card (personal-cfo-ul5d, ADR 0060): link a SimpleFIN Bridge
// connection from a pasted setup token, map its external accounts onto real
// accounts, refresh on demand, and read connection health at a glance. The
// health surface the connector engine (gglk) feeds.
//
// The setup token is a one-time secret: it lives in local state only until
// submit (zeroed in a finally, even on a thrown rejection), rides a masked
// input, and goes straight to Rust (FRONTEND.md §1).

import { useQuery } from "@tanstack/react-query";
import { Cable, Loader2, Plug } from "lucide-react";
import { useState } from "react";

import {
  commands,
  type ConnectorAccountLinkDto,
  type ConnectorConnectionDto,
} from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Skeleton } from "@/components/ui/skeleton";
import { formatDateTime, formatIsoDate } from "@/lib/format";
import { ipcQuery, queryKeys } from "@/lib/query";
import { describeIpcError } from "@/vault/useVault";

import { syncOutcomeCopy, syncOutcomeTone } from "./connectorSync";
import { NewMappedAccountDialog } from "./NewMappedAccountDialog";
import { useBaseCurrency } from "./useBaseCurrency";
import { useConnections } from "./useConnections";

/// IpcError can be a bare string variant, so success results are discriminated
/// by their own fields behind an object guard.
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/// Beyond this, a "Refreshed" badge stops being reassuring: the Bridge
/// refreshes about daily, so a few missed days means refreshes are not
/// happening.
const STALE_AFTER_DAYS = 3;

function HealthBadge({ connection }: { connection: ConnectorConnectionDto }) {
  if (connection.last_error) {
    return <Badge variant="warning">Needs attention</Badge>;
  }
  const stamp = formatDateTime(connection.last_synced_at);
  if (!stamp) {
    return <Badge variant="outline">Never refreshed</Badge>;
  }
  const ageDays = Math.floor(
    (Date.now() - new Date(connection.last_synced_at ?? "").getTime()) /
      86_400_000,
  );
  if (ageDays >= STALE_AFTER_DAYS) {
    return <Badge variant="warning">Refreshed {stamp}</Badge>;
  }
  return <Badge variant="gain">Refreshed {stamp}</Badge>;
}

/// Sentinel select value that opens the create-new-account dialog instead
/// of mapping (personal-cfo-07bn). Never a real account id.
const CREATE_NEW = "__create_new__";

function AccountLinkRow({
  connectionId,
  link,
  accounts,
  onMap,
  onCreateNew,
}: {
  connectionId: string;
  link: ConnectorAccountLinkDto;
  accounts: { id: string; name: string }[];
  onMap: (externalId: string, accountId: string | null) => void;
  onCreateNew: (externalId: string, externalName: string) => void;
}) {
  // Namespaced by connection: external ids are only connection-scoped.
  const selectId = `map-${connectionId}-${link.external_id}`;
  return (
    <div className="flex items-center justify-between gap-3">
      <div className="min-w-0">
        <p className="truncate text-sm">
          {link.external_name ?? link.external_id}
        </p>
        <p className="text-xs text-muted-foreground">
          {link.account_id
            ? link.last_synced_on
              ? `Refreshed through ${formatIsoDate(link.last_synced_on)}`
              : "Mapped — next refresh fetches full history"
            : "Not mapped — transactions from this account are not imported"}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Label htmlFor={selectId} className="sr-only">
          Account for {link.external_name ?? link.external_id}
        </Label>
        <NativeSelect
          id={selectId}
          size="sm"
          value={link.account_id ?? ""}
          onChange={(event) => {
            const value = event.target.value;
            if (value === CREATE_NEW) {
              // The select is controlled by the stored mapping, so it snaps
              // back on re-render; the dialog maps on success.
              onCreateNew(link.external_id, link.external_name ?? link.external_id);
              return;
            }
            onMap(link.external_id, value || null);
          }}
        >
          <option value="">Not mapped</option>
          {accounts.map((account) => (
            <option key={account.id} value={account.id}>
              {account.name}
            </option>
          ))}
          <option value={CREATE_NEW}>Create new account…</option>
        </NativeSelect>
      </div>
    </div>
  );
}

function ConnectionRow({
  connection,
  accounts,
  accountsPending,
  onMap,
  onCreateNew,
  onSync,
  syncPending,
  onForget,
}: {
  connection: ConnectorConnectionDto;
  accounts: { id: string; name: string }[];
  accountsPending: boolean;
  onMap: (externalId: string, accountId: string | null) => void;
  onCreateNew: (externalId: string, externalName: string) => void;
  onSync: () => void;
  syncPending: boolean;
  onForget: () => void;
}) {
  const [confirmingForget, setConfirmingForget] = useState(false);
  return (
    <div className="rounded-md border p-3">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium">
            {connection.display_hint ?? "SimpleFIN connection"}
          </p>
          <p className="text-xs text-muted-foreground">SimpleFIN Bridge</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <HealthBadge connection={connection} />
          <Button
            size="sm"
            variant="secondary"
            disabled={syncPending}
            onClick={onSync}
          >
            {syncPending ? <Loader2 className="animate-spin" /> : null}
            Refresh now
          </Button>
        </div>
      </div>
      {connection.last_error ? (
        <p role="alert" className="mt-2 text-sm text-loss">
          Last refresh failed: {connection.last_error}
        </p>
      ) : null}
      <div className="mt-3 flex flex-col gap-2">
        {connection.links.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No accounts discovered yet — refresh to discover them.
          </p>
        ) : accountsPending ? (
          // A mapped link must never masquerade as "Not mapped" while the
          // accounts list is still loading (the controlled select would fall
          // back to its first option) — hold the rows behind a skeleton.
          <Skeleton className="h-10 w-full" />
        ) : (
          connection.links.map((link) => (
            <AccountLinkRow
              key={link.external_id}
              connectionId={connection.id}
              link={link}
              accounts={accounts}
              onMap={onMap}
              onCreateNew={onCreateNew}
            />
          ))
        )}
      </div>
      <div className="mt-3">
        {confirmingForget ? (
          <div className="flex items-center gap-2 rounded-md border border-loss/40 bg-loss/5 p-2">
            <p className="flex-1 text-xs text-muted-foreground">
              Forgetting removes the stored credential. Transactions already
              refreshed stay in the ledger. Re-linking needs a fresh setup token.
            </p>
            <Button size="sm" variant="destructive" onClick={onForget}>
              Forget
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setConfirmingForget(false)}
            >
              Cancel
            </Button>
          </div>
        ) : (
          <Button
            size="sm"
            variant="outline"
            className="border-loss/50 text-loss hover:bg-loss/10"
            onClick={() => setConfirmingForget(true)}
          >
            Forget connection…
          </Button>
        )}
      </div>
    </div>
  );
}

const VAULT_UNREACHABLE = "Could not reach the vault service.";

export function ConnectionsCard() {
  const {
    connections,
    error: loadError,
    link,
    linkPending,
    setAccountLink,
    sync,
    syncingId,
    forget,
  } = useConnections();
  const accountsQuery = useQuery({
    queryKey: queryKeys.accounts,
    queryFn: () => ipcQuery(commands.accountList(), "Could not load accounts."),
  });
  const accounts = (accountsQuery.data ?? [])
    .filter((account) => account.active)
    .map((account) => ({ id: account.id, name: account.name }));

  const { baseCurrency } = useBaseCurrency();
  // The mapping row whose "Create new account…" was chosen (07bn).
  const [creatingFor, setCreatingFor] = useState<{
    connectionId: string;
    externalId: string;
    externalName: string;
  } | null>(null);

  const [linking, setLinking] = useState(false);
  const [token, setToken] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const submitLink = async () => {
    setActionError(null);
    setNotice(null);
    try {
      const result = await link(token.trim());
      if (isRecord(result) && "connection_id" in result) {
        setLinking(false);
        setNotice(
          result.fetch_error
            ? "Connected. Account discovery hit a snag — refresh to retry it."
            : `Connected — ${result.accounts.length} account(s) discovered. Map them below.`,
        );
      } else {
        setActionError(describeIpcError(result));
      }
    } catch {
      setActionError(VAULT_UNREACHABLE);
    } finally {
      // Zero the pasted secret from component state on every path.
      setToken("");
    }
  };

  const runSync = async (connectionId: string) => {
    setActionError(null);
    setNotice(null);
    try {
      const result = await sync(connectionId);
      if (isRecord(result) && "connection_id" in result) {
        const copy = syncOutcomeCopy(result.status, result.message);
        if (syncOutcomeTone(result.status) === "notice") setNotice(copy);
        else setActionError(copy);
      } else {
        setActionError(describeIpcError(result));
      }
    } catch {
      setActionError(VAULT_UNREACHABLE);
    }
  };

  const runMap = async (
    connectionId: string,
    externalId: string,
    accountId: string | null,
  ) => {
    setActionError(null);
    try {
      const failure = await setAccountLink(connectionId, externalId, accountId);
      if (failure) setActionError(describeIpcError(failure));
    } catch {
      setActionError(VAULT_UNREACHABLE);
    }
  };

  const runForget = async (connectionId: string) => {
    setActionError(null);
    setNotice(null);
    try {
      const failure = await forget(connectionId);
      if (failure) setActionError(describeIpcError(failure));
    } catch {
      setActionError(VAULT_UNREACHABLE);
    }
  };

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Connections</CardTitle>
        <CardDescription>
          Bank connections via the SimpleFIN Bridge. The Bridge refreshes bank
          data about daily; the app refreshes it on open and on demand, and
          everything stays in this vault.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {loadError ? (
          <p role="alert" className="text-sm text-loss">
            {loadError}
          </p>
        ) : connections === null ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-16 w-full" />
          </div>
        ) : connections.length === 0 ? (
          <EmptyState
            icon={Cable}
            title="No connections yet"
            description="Link a SimpleFIN Bridge connection to refresh transactions automatically."
          />
        ) : accountsQuery.error ? (
          <p role="alert" className="text-sm text-loss">
            {accountsQuery.error.message}
          </p>
        ) : (
          connections.map((connection) => (
            <ConnectionRow
              key={connection.id}
              connection={connection}
              accounts={accounts}
              accountsPending={accountsQuery.isPending}
              onMap={(externalId, accountId) =>
                void runMap(connection.id, externalId, accountId)
              }
              onCreateNew={(externalId, externalName) =>
                setCreatingFor({ connectionId: connection.id, externalId, externalName })
              }
              onSync={() => void runSync(connection.id)}
              syncPending={syncingId === connection.id}
              onForget={() => void runForget(connection.id)}
            />
          ))
        )}

        {notice ? (
          <p role="status" className="text-sm text-gain">
            {notice}
          </p>
        ) : null}
        {actionError ? (
          <p role="alert" className="text-sm text-loss">
            {actionError}
          </p>
        ) : null}

        {linking ? (
          <div className="rounded-md border p-3">
            <Label htmlFor="setup-token" className="text-sm">
              Setup token
            </Label>
            <p className="mt-1 text-xs text-muted-foreground">
              Create the token in the SimpleFIN Bridge under Apps, then paste
              it here. Tokens are single-use.
            </p>
            <div className="mt-2 flex items-center gap-2">
              <Input
                id="setup-token"
                type="password"
                autoComplete="off"
                value={token}
                onChange={(event) => setToken(event.target.value)}
                placeholder="Paste the setup token"
              />
              <Button
                size="sm"
                disabled={token.trim().length === 0 || linkPending}
                onClick={() => void submitLink()}
              >
                {linkPending ? <Loader2 className="animate-spin" /> : null}
                Connect
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  setToken("");
                  setLinking(false);
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <div>
            <Button size="sm" variant="outline" onClick={() => setLinking(true)}>
              <Plug />
              Link a connection…
            </Button>
          </div>
        )}
      </CardContent>
      {creatingFor ? (
        <NewMappedAccountDialog
          // Keyed per row so choosing another row's "Create new account…"
          // remounts with its name rather than mutating a live dialog.
          key={`${creatingFor.connectionId}:${creatingFor.externalId}`}
          externalName={creatingFor.externalName}
          currency={baseCurrency}
          onCreated={(id) =>
            void runMap(creatingFor.connectionId, creatingFor.externalId, id)
          }
          onClose={() => {
            const selectId = `map-${creatingFor.connectionId}-${creatingFor.externalId}`;
            setCreatingFor(null);
            // Hand focus back to the select that opened the dialog.
            document.getElementById(selectId)?.focus();
          }}
        />
      ) : null}
    </Card>
  );
}
