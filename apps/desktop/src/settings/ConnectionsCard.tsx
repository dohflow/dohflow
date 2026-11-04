// The Connections card (personal-cfo-ul5d, ADR 0060): link a bank connection
// through the provider picker (personal-cfo-dto2j, ADR 0076 §4), map its
// external accounts onto real accounts, refresh on demand, and read connection
// health at a glance. The health surface the connector engine (gglk) feeds.
// Provider names and credential wording come from the connector registry —
// this card carries no provider-specific copy.
//
// The pasted credential is a one-time secret: ConnectProviderFlow holds it in
// local state only until submit (zeroed on every path), rides a masked input,
// and it goes straight to Rust (FRONTEND.md §1).

import { useQuery } from "@tanstack/react-query";
import { Cable, Loader2, Plug } from "lucide-react";
import { forwardRef, useState, type ComponentProps, type Ref } from "react";

import {
  commands,
  type ConnectorAccountLinkDto,
  type ConnectorConnectionDto,
  type ConnectorFeedDto,
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
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Skeleton } from "@/components/ui/skeleton";
import { formatDateTime, formatIsoDate } from "@/lib/format";
import { ipcQuery, queryKeys } from "@/lib/query";
import { describeIpcError } from "@/vault/useVault";

import { ConnectProviderFlow } from "./connections/ConnectProviderFlow";
import { CurrencyRefusal } from "./connections/CurrencyRefusal";
import { SharedFeedDialog } from "./connections/SharedFeedDialog";
import { useConnectorAdapters } from "./connections/useConnectorAdapters";
import { syncOutcomeCopy, syncOutcomeTone } from "./connectorSync";
import { NewMappedAccountDialog } from "./NewMappedAccountDialog";
import { useAttemptCount, useReannounced } from "./attemptKey";
import { PersistentNotice } from "./PersistentSlot";
import { useKeepFocusAcrossMove } from "./useKeepFocusAcrossMove";
import { useBaseCurrency } from "./useBaseCurrency";
import { useConnections } from "./useConnections";

/// IpcError can be a bare string variant, so success results are discriminated
/// by their own fields behind an object guard.
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/// Beyond this, a "Refreshed" badge stops being reassuring: providers
/// refresh about daily, so a few missed days means refreshes are not
/// happening.
const STALE_AFTER_DAYS = 3;

/// A mapped link whose currency is KNOWN to differ from the base currency:
/// its transactions are held on refresh (personal-cfo-049p6).
function isHeld(link: ConnectorAccountLinkDto): boolean {
  return (
    link.account_id !== null &&
    (link.currency ?? null) !== null &&
    (link.currency_refusal ?? null) !== null
  );
}

/// A connection whose last refresh failed, or that holds a foreign-currency account: it has a
/// message that has to stay on screen with its one recovery action, Refresh now.
function needsRefresh(connection: ConnectorConnectionDto): boolean {
  return Boolean(connection.last_error) || connection.links.some(isHeld);
}

/// The status badge may wrap its complete label when the lane is narrower than the stamp is long.
const STATUS_BADGE = "h-auto min-w-0 max-w-full whitespace-normal text-left";

function HealthBadge({ connection }: { connection: ConnectorConnectionDto }) {
  if (
    connection.last_error ||
    connection.attention.length > 0 ||
    connection.links.some(isHeld)
  ) {
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
  // The words, not the tint, say it is out of date (a Badge's mark is only a redundant cue;
  // docs/agent/FRONTEND.md 7.3, personal-cfo-4d8.35). Fresh and stale must not read the same.
  if (ageDays >= STALE_AFTER_DAYS) {
    return <Badge variant="warning" className={STATUS_BADGE}>
        Out of date · refreshed {stamp}
      </Badge>;
  }
  return <Badge variant="gain" className={STATUS_BADGE}>
      Refreshed {stamp}
    </Badge>;
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
  onCreateNew: (
    externalId: string,
    externalName: string,
    currencyRefusal: string | null,
  ) => void;
}) {
  // Namespaced by connection: external ids are only connection-scoped.
  const selectId = `map-${connectionId}-${link.external_id}`;
  const refusalId = `${selectId}-currency`;
  const statusId = `${selectId}-status`;
  const refusal = link.currency_refusal ?? null;
  const currency = link.currency ?? null;
  const mapped = link.account_id !== null;
  // The currency guard (personal-cfo-049p6): an unmapped account it refuses
  // can't be mapped; a mapped one it refuses can only be unmapped. Unknown
  // and known-foreign read differently — unknown is never shown as the base
  // currency.
  const held = isHeld(link);
  const unconfirmed = mapped && currency === null;
  const blocked = !mapped && refusal !== null;
  // The select is described by the one line that explains its state, and only by a line that is on
  // screen: the refusal in the Held and Blocked branches, the status line when the currency is
  // not confirmed yet (a mapped link with an unknown currency carries a refusal that is not shown).
  const describedBy = held || blocked ? refusalId : unconfirmed ? statusId : undefined;
  const mappedAccount = accounts.find((account) => account.id === link.account_id);
  return (
    <div className="flex items-center justify-between gap-3">
      <div className="min-w-0">
        <p className="truncate text-sm">
          {link.external_name ?? link.external_id}
          {currency ? (
            <span className="text-muted-foreground"> · {currency}</span>
          ) : null}
        </p>
        {held ? (
          <>
            <p className="text-xs text-warning">
              Held — this account&rsquo;s transactions are not imported.
            </p>
            <CurrencyRefusal id={refusalId} message={refusal ?? ""} />
          </>
        ) : blocked ? (
          <CurrencyRefusal id={refusalId} message={refusal ?? ""} />
        ) : (
          <p id={statusId} className="text-xs text-muted-foreground">
            {mapped
              ? link.last_synced_on
                ? `Refreshed through ${formatIsoDate(link.last_synced_on)}`
                : "Mapped — next refresh fetches full history"
              : "Not mapped — transactions from this account are not imported"}
            {unconfirmed ? " · Currency not confirmed yet — the next refresh records it" : ""}
          </p>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Label htmlFor={selectId} className="sr-only">
          Account for {link.external_name ?? link.external_id}
        </Label>
        <NativeSelect
          id={selectId}
          size="sm"
          value={link.account_id ?? ""}
          disabled={blocked}
          aria-describedby={describedBy}
          onChange={(event) => {
            const value = event.target.value;
            if (value === CREATE_NEW) {
              // The select is controlled by the stored mapping, so it snaps
              // back on re-render; the dialog maps on success.
              onCreateNew(
                link.external_id,
                link.external_name ?? link.external_id,
                refusal,
              );
              return;
            }
            onMap(link.external_id, value || null);
          }}
        >
          <option value="">Not mapped</option>
          {refusal !== null ? (
            // Refused: only the current mapping (to unmap from), no new ones.
            mappedAccount ? (
              <option value={mappedAccount.id}>{mappedAccount.name}</option>
            ) : null
          ) : (
            <>
              {accounts.map((account) => (
                <option key={account.id} value={account.id}>
                  {account.name}
                </option>
              ))}
              <option value={CREATE_NEW}>Create new account…</option>
            </>
          )}
        </NativeSelect>
      </div>
    </div>
  );
}

/// How often a connection refreshes on its own (personal-cfo-lqk; ADR 0060
/// addendum 2026-10-04). Refresh only ever runs after the vault is unlocked.
const CADENCE_OPTIONS: { value: string; label: string }[] = [
  { value: "every_open", label: "Every time you open the vault" },
  { value: "daily", label: "Daily" },
  { value: "weekly", label: "Weekly" },
  { value: "manual", label: "Only when I press Refresh now" },
];

const RefreshNowButton = forwardRef<
  HTMLButtonElement,
  { syncPending: boolean; onSync: () => void }
>(function RefreshNowButton({ syncPending, onSync }, ref) {
  return (
    <Button
      ref={ref}
      size="sm"
      variant="secondary"
      className="shrink-0"
      disabled={syncPending}
      onClick={onSync}
    >
      {syncPending ? <Loader2 className="animate-spin" /> : null}
      Refresh now
    </Button>
  );
});

/// A connection's refresh failure and its held-account message, in full.
function ConnectionMessages({
  connection,
  className,
}: {
  connection: ConnectorConnectionDto;
  className?: string;
}) {
  return (
    <>
      {connection.last_error ? (
        <p role="alert" className={`${className ?? ""} text-sm text-loss`}>
          Last refresh failed: {connection.last_error}
        </p>
      ) : null}
      {connection.links.some(isHeld) ? (
        <p role="status" className={`${className ?? ""} text-sm text-warning`}>
          An account on this connection is in a currency other than your base
          currency. Its transactions are held and not imported.
        </p>
      ) : null}
    </>
  );
}

export function ConnectionRow({
  connection,
  providerName,
  credentialNoun,
  accounts,
  accountsPending,
  onMap,
  onCreateNew,
  onSetCadence,
  onSync,
  syncPending,
  onForget,
  messagesInSlot = false,
  refreshRef,
}: {
  connection: ConnectorConnectionDto;
  /// The provider's display name, from the registry.
  providerName: string;
  /// What re-linking asks for ("setup token", "API key").
  credentialNoun: string;
  accounts: { id: string; name: string }[];
  accountsPending: boolean;
  onMap: (externalId: string, accountId: string | null) => void;
  onCreateNew: (
    externalId: string,
    externalName: string,
    currencyRefusal: string | null,
  ) => void;
  onSetCadence: (cadence: string) => void;
  onSync: () => void;
  syncPending: boolean;
  onForget: () => void;
  /// The card shows this connection's refresh failure, held-account message and Refresh now in the
  /// section's persistent slot (`ConnectionAttention`); the row then omits all three, so each is
  /// rendered once.
  messagesInSlot?: boolean;
  /// For the card to hand focus back to Refresh now when it moves into the slot and out again.
  refreshRef?: Ref<HTMLButtonElement>;
}) {
  const [confirmingForget, setConfirmingForget] = useState(false);
  const cadenceId = `refresh-cadence-${connection.id}`;
  return (
    <div className="@container rounded-md border p-3">
      {/* The status words and the action must both stay whole (the owner's status-fit condition,
          personal-cfo-4d8.35), in the reviewed composition (Paper SF-1r2 SF03/SF04): at a narrow
          component width the identity sits above ONE status lane, in which the badge wraps its
          complete label beside a right-aligned, non-shrinking 36px Refresh now; at a wide width the
          identity and the lane share a row. The switch follows the row's own width (a container
          query), not the window's. */}
      <div className="flex flex-col gap-2 @lg:flex-row @lg:items-start @lg:justify-between @lg:gap-3">
        <div data-part="identity" className="min-w-0 @lg:flex-1 @lg:basis-40">
          <p className="break-words text-sm font-medium">
            {connection.display_hint ?? `${providerName} connection`}
          </p>
          <p className="text-xs text-muted-foreground">{providerName}</p>
        </div>
        <div
          data-part="status-lane"
          className="flex min-w-0 items-center justify-between gap-3 @lg:justify-end"
        >
          <HealthBadge connection={connection} />
          {messagesInSlot ? null : (
            <RefreshNowButton ref={refreshRef} syncPending={syncPending} onSync={onSync} />
          )}
        </div>
      </div>
      <div className="mt-2 flex items-center gap-2">
        <Label htmlFor={cadenceId} className="text-xs text-muted-foreground">
          Refresh automatically
        </Label>
        <NativeSelect
          id={cadenceId}
          size="sm"
          value={connection.refresh_cadence}
          onChange={(event) => onSetCadence(event.target.value)}
        >
          {CADENCE_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </NativeSelect>
      </div>
      {messagesInSlot ? null : (
        <ConnectionMessages connection={connection} className="mt-2" />
      )}
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
              refreshed stay in the ledger. Re-linking needs a fresh{" "}
              {credentialNoun}.
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

/// One connection in the list, with its message and recovery action in the persistent slot. A
/// refresh failure, a held-account message, or a failed Refresh now stays on screen, open or closed,
/// with the one Refresh now that recovers from it; the row then leaves all of those out, so each is
/// rendered once. The connection's name leads the entry so it is clear which one it is for.
function ConnectionItem({
  syncError,
  actionAttempt,
  ...row
}: ComponentProps<typeof ConnectionRow> & { syncError: string | null; actionAttempt: number }) {
  const syncAlert = useReannounced(syncError, actionAttempt);
  const refresh = useKeepFocusAcrossMove({ settled: !row.syncPending });
  const inSlot = needsRefresh(row.connection) || syncError !== null;
  const onSync = () => {
    refresh.remember();
    row.onSync();
  };
  return (
    <>
      <ConnectionRow {...row} onSync={onSync} messagesInSlot={inSlot} refreshRef={refresh.ref} />
      <PersistentNotice>
        {inSlot && (
          <div className="flex flex-col gap-2 rounded-md border p-3">
            <p className="break-words text-sm font-medium">
              {row.connection.display_hint ?? `${row.providerName} connection`}
            </p>
            <ConnectionMessages connection={row.connection} />
            {syncError !== null && (
              <p role="alert" className="text-sm text-loss">
                {syncAlert}
              </p>
            )}
            <div>
              <RefreshNowButton ref={refresh.ref} syncPending={row.syncPending} onSync={onSync} />
            </div>
          </div>
        )}
      </PersistentNotice>
    </>
  );
}

const VAULT_UNREACHABLE = "Could not reach the vault service.";

export function ConnectionsCard({
  startLinking = false,
}: {
  /// Open the connect flow on mount — onboarding's connected branch shows the
  /// provider disclosure up front, as its dedicated panel used to.
  startLinking?: boolean;
} = {}) {
  const providers = useConnectorAdapters();
  const {
    connections,
    error: loadError,
    link,
    linkPending,
    setAccountLink,
    createMappedAccount,
    setRefreshCadence,
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
    currencyRefusal: string | null;
  } | null>(null);

  // A mapping refused because another connector link already feeds the
  // account (personal-cfo-6evt): the user decides in SharedFeedDialog.
  const [sharedFeed, setSharedFeed] = useState<{
    connectionId: string;
    externalId: string;
    accountId: string;
    existingFeeds: ConnectorFeedDto[];
  } | null>(null);
  const [sharedFeedBusy, setSharedFeedBusy] = useState(false);

  const [linking, setLinking] = useState(startLinking);
  const [actionError, setActionError] = useState<string | null>(null);
  // The connection whose Refresh now produced `actionError`, so the message sits with that
  // connection's Refresh now in the persistent slot; any other action's error is shown on its own.
  const [syncFailedId, setSyncFailedId] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  // Each settled attempt that fails clears and restores its alert text, so an identical repeat is
  // announced again (the error is kept on screen through the retry).
  const [actionAttempt, nextActionAttempt] = useAttemptCount();
  const actionAlert = useReannounced(actionError, actionAttempt);
  const setOtherError = (message: string | null) => {
    setSyncFailedId(null);
    setActionError(message);
    if (message !== null) nextActionAttempt();
  };

  const submitLink = async (adapterId: string, credential: string) => {
    setOtherError(null);
    setNotice(null);
    try {
      const result = await link(adapterId, credential);
      if (isRecord(result) && "connection_id" in result) {
        setLinking(false);
        setNotice(
          result.fetch_error
            ? "Connected. Account discovery hit a snag — refresh to retry it."
            : `Connected — ${result.accounts.length} account(s) discovered. Map them below.`,
        );
      } else {
        setOtherError(describeIpcError(result));
      }
    } catch {
      setOtherError(VAULT_UNREACHABLE);
    }
  };

  const runSync = async (connectionId: string) => {
    setNotice(null);
    // An earlier failure stays, with its Refresh now, until this try settles: clearing it now would
    // move the Refresh now into the body, which is hidden when the section is collapsed.
    const failSync = (message: string) => {
      setSyncFailedId(connectionId);
      setActionError(message);
      nextActionAttempt();
    };
    try {
      const result = await sync(connectionId);
      if (isRecord(result) && "connection_id" in result) {
        const copy = syncOutcomeCopy(result.status, result.message);
        if (syncOutcomeTone(result.status) === "notice") {
          setOtherError(null);
          setNotice(copy);
        } else {
          failSync(copy);
        }
      } else {
        failSync(describeIpcError(result));
      }
    } catch {
      failSync(VAULT_UNREACHABLE);
    }
  };

  const runMap = async (
    connectionId: string,
    externalId: string,
    accountId: string | null,
    allowSharedFeed = false,
  ): Promise<boolean> => {
    setOtherError(null);
    try {
      const outcome = await setAccountLink(
        connectionId,
        externalId,
        accountId,
        allowSharedFeed,
      );
      if (outcome.kind === "error") {
        setOtherError(describeIpcError(outcome.error));
        return false;
      }
      if (outcome.kind === "already_fed" && accountId !== null) {
        setSharedFeed({
          connectionId,
          externalId,
          accountId,
          existingFeeds: outcome.existingFeeds,
        });
        return false;
      }
      return true;
    } catch {
      setOtherError(VAULT_UNREACHABLE);
      return false;
    }
  };

  const linkFor = (connectionId: string, externalId: string) =>
    connections
      ?.find((connection) => connection.id === connectionId)
      ?.links.find((link) => link.external_id === externalId) ?? null;

  const closeSharedFeed = () => {
    if (sharedFeed) {
      const selectId = `map-${sharedFeed.connectionId}-${sharedFeed.externalId}`;
      document.getElementById(selectId)?.focus();
    }
    setSharedFeed(null);
  };

  const chooseSharedFeed = async (choice: "dont_import" | "import_both") => {
    if (!sharedFeed) return;
    const { connectionId, externalId, accountId } = sharedFeed;
    const link = linkFor(connectionId, externalId);
    const externalName = link?.external_name ?? externalId;
    const accountName =
      accounts.find((account) => account.id === accountId)?.name ?? "That account";
    setSharedFeedBusy(true);
    let saved = true;
    if (choice === "import_both") {
      saved = await runMap(connectionId, externalId, accountId, true);
      if (saved) {
        setNotice(
          `“${externalName}” now also updates ${accountName}. A transaction both connections report with the same date and amount waits in the Money Inbox; one reported on different dates is imported twice.`,
        );
      }
    } else {
      // Leave it unmapped: listed under its connection, feeding nothing.
      if (link?.account_id) saved = await runMap(connectionId, externalId, null);
      if (saved) {
        setNotice(
          `“${externalName}” is not imported. ${accountName} keeps its existing connection.`,
        );
      }
    }
    setSharedFeedBusy(false);
    closeSharedFeed();
  };

  const runSetCadence = async (connectionId: string, cadence: string) => {
    setOtherError(null);
    try {
      const failure = await setRefreshCadence(connectionId, cadence);
      if (failure) setOtherError(describeIpcError(failure));
    } catch {
      setOtherError(VAULT_UNREACHABLE);
    }
  };

  const runForget = async (connectionId: string) => {
    setOtherError(null);
    setNotice(null);
    try {
      const failure = await forget(connectionId);
      if (failure) setOtherError(describeIpcError(failure));
    } catch {
      setOtherError(VAULT_UNREACHABLE);
    }
  };

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm">Connections</CardTitle>
        <CardDescription>
          Bank connections through an independent provider you choose. The app
          refreshes them on open and on demand, and everything stays in this
          vault.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {loadError ? null : connections === null ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-16 w-full" />
          </div>
        ) : connections.length === 0 ? (
          <EmptyState
            icon={Cable}
            title="No connections yet"
            description="Link a bank connection to refresh transactions automatically."
          />
        ) : accountsQuery.error ? null : (
          connections.map((connection) => (
            <ConnectionItem
              key={connection.id}
              syncError={syncFailedId === connection.id ? actionError : null}
              actionAttempt={actionAttempt}
              connection={connection}
              providerName={providers.displayName(connection.adapter_id)}
              credentialNoun={
                providers.adapter(connection.adapter_id)?.link_guide.credential_noun ??
                "credential"
              }
              accounts={accounts}
              accountsPending={accountsQuery.isPending}
              onMap={(externalId, accountId) =>
                void runMap(connection.id, externalId, accountId)
              }
              onCreateNew={(externalId, externalName, currencyRefusal) =>
                setCreatingFor({
                  connectionId: connection.id,
                  externalId,
                  externalName,
                  currencyRefusal,
                })
              }
              onSetCadence={(cadence) => void runSetCadence(connection.id, cadence)}
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
        {/* Every error and its message stay in the section's persistent slot, open or closed. A
            failed read, and a failed map, cadence, forget or link, show their message there; a failed
            Refresh now shows it with that connection's Refresh now. */}
        <PersistentNotice>
          {loadError ? (
            <p role="alert" className="text-sm text-loss">
              {loadError}
            </p>
          ) : accountsQuery.error ? (
            <p role="alert" className="text-sm text-loss">
              {accountsQuery.error.message}
            </p>
          ) : null}
          {actionError && syncFailedId === null ? (
            <p role="alert" className="text-sm text-loss">
              {actionAlert}
            </p>
          ) : null}
        </PersistentNotice>

        {linking ? (
          <ConnectProviderFlow
            linkPending={linkPending}
            onLink={submitLink}
            onCancel={() => setLinking(false)}
          />
        ) : (
          <div>
            <Button size="sm" variant="outline" onClick={() => setLinking(true)}>
              <Plug />
              Link a connection…
            </Button>
          </div>
        )}
      </CardContent>
      {sharedFeed ? (
        <SharedFeedDialog
          accountName={
            accounts.find((account) => account.id === sharedFeed.accountId)?.name ??
            "This account"
          }
          externalName={
            linkFor(sharedFeed.connectionId, sharedFeed.externalId)?.external_name ??
            sharedFeed.externalId
          }
          existingFeeds={sharedFeed.existingFeeds.map((feed) => ({
            connection:
              feed.display_hint ?? `${providers.displayName(feed.adapter_id)} connection`,
            account: feed.external_name ?? null,
          }))}
          busy={sharedFeedBusy}
          onDontImport={() => void chooseSharedFeed("dont_import")}
          onChooseOther={closeSharedFeed}
          onImportBoth={() => void chooseSharedFeed("import_both")}
        />
      ) : null}
      {creatingFor ? (
        <NewMappedAccountDialog
          // Keyed per row so choosing another row's "Create new account…"
          // remounts with its name rather than mutating a live dialog.
          key={`${creatingFor.connectionId}:${creatingFor.externalId}`}
          externalName={creatingFor.externalName}
          currency={baseCurrency}
          currencyRefusal={creatingFor.currencyRefusal}
          // One call: guard, then create, then map (personal-cfo-pxi.8), so a
          // stale refusal state can never leave an empty account behind.
          onCreate={(account) =>
            createMappedAccount(creatingFor.connectionId, creatingFor.externalId, account)
          }
          onCreated={() => setOtherError(null)}
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
