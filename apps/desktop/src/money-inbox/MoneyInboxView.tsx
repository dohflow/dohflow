import { useEffect, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  ArrowRight,
  Check,
  ChevronDown,
  Clock,
  Inbox,
  Layers,
  Loader2,
  MoonStar,
  Sparkles,
  Unplug,
  Upload,
  X,
} from "lucide-react";

import type {
  AccountViewDto,
  CategoryDto,
  ConnectorSyncResultDto,
  IpcError,
  MoneyInboxItemDto,
  TransactionRowDto,
} from "@/bindings";
import { commands } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableRow,
} from "@/components/ui/table";
import { syncOutcomeCopy } from "@/settings/connectorSync";
import { describeIpcError } from "@/vault/useVault";
import { ipcQuery } from "@/lib/query";
import { formatIsoDate, formatMoney, signedAmountClass } from "@/lib/format";
import { cn } from "@/lib/utils";
import { useAccounts } from "@/accounts/useAccounts";
import { useCategories } from "@/categories/useCategories";
import { categoryLabels } from "@/categories/labels";
import { CategoryCombobox } from "@/categories/CategoryCombobox";
import { SetBalanceModal } from "@/accounts/SetBalanceModal";
import { useMoneyInbox } from "./useMoneyInbox";
import { ImportFileDialog } from "./ImportFileDialog";
import { CardReviewModal } from "./CardReviewModal";
import { DuplicateReviewPanel } from "./DuplicateReviewPanel";
import { useTransactions } from "@/transactions/useTransactions";
import { useTags } from "@/tags/useTags";
import { BulkTransactionActions } from "@/transactions/BulkTransactionActions";
import type { TransactionSelection } from "@/transactions/useTransactionSelection";
import { TransactionDetailDrawer } from "@/transactions/TransactionDetailDrawer";
import { PaginationControls } from "@/components/ui/pagination";
import { PAGE_SIZE_OPTIONS, usePagination } from "@/lib/usePagination";

/// The display detail an `imported_waiting_commit` item carries in `payload_json`
/// (built by the db-worker generator, ADR 0014 §7). Parsed leniently — a missing
/// or malformed field degrades to a sensible default rather than throwing.
type ImportedWaitingCommitPayload = {
  merchant: string | null;
  description: string | null;
  amount_minor: number;
  currency: string;
  posted_at: string;
  account_name: string | null;
  source_name: string | null;
  dedupe_reason: string | null;
  suspected_committed_txn_id: string | null;
};

function parsePayload(json: string): ImportedWaitingCommitPayload | null {
  try {
    const raw = JSON.parse(json) as Record<string, unknown>;
    if (typeof raw.amount_minor !== "number" || typeof raw.currency !== "string") {
      return null;
    }
    const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
    return {
      merchant: str(raw.merchant),
      description: str(raw.description),
      amount_minor: raw.amount_minor,
      currency: raw.currency,
      posted_at: str(raw.posted_at) ?? "",
      account_name: str(raw.account_name),
      source_name: str(raw.source_name),
      dedupe_reason: str(raw.dedupe_reason),
      suspected_committed_txn_id: str(raw.suspected_committed_txn_id),
    };
  } catch {
    return null;
  }
}

/// The detail a `stale_balance` item carries (ADR 0014 §7 addendum, personal-cfo-r52x).
type StaleBalancePayload = {
  account_id: string;
  account_name: string;
  last_observed: string;
  days_stale: number;
};

interface ConnectorErrorPayload {
  connection_id: string;
  display_hint: string | null;
  last_error: string;
  last_synced_at: string | null;
}

function parseConnectorErrorPayload(json: string): ConnectorErrorPayload | null {
  try {
    const raw = JSON.parse(json) as Record<string, unknown>;
    if (
      typeof raw.connection_id !== "string" ||
      typeof raw.last_error !== "string"
    ) {
      return null;
    }
    return {
      connection_id: raw.connection_id,
      display_hint:
        typeof raw.display_hint === "string" ? raw.display_hint : null,
      last_error: raw.last_error,
      last_synced_at:
        typeof raw.last_synced_at === "string" ? raw.last_synced_at : null,
    };
  } catch {
    return null;
  }
}

/// A bank connection whose last sync recorded an error (personal-cfo-zfyo,
/// ADR 0060 §5). Derived on read from the connection row; resolves
/// intrinsically when a sync succeeds (or the connection is forgotten in
/// Settings). Rate-limited syncs are healthy and never surface here.
function ConnectorErrorRow({
  item,
  selectCell,
  isSelected,
  expanded,
  onToggleExpand,
  onRetrySync,
}: RowCommon & {
  onRetrySync: (
    connectionId: string,
  ) => Promise<ConnectorSyncResultDto | IpcError>;
}) {
  const [retrying, setRetrying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const payload = parseConnectorErrorPayload(item.payload_json);
  if (payload === null) return null;

  async function retry() {
    if (payload === null) return;
    setRetrying(true);
    setError(null);
    try {
      const result = await onRetrySync(payload.connection_id);
      // On a clean sync the inbox re-fetches and this row unmounts; every
      // other outcome (still expired, throttled, in flight…) gets the same
      // copy the Settings card shows — never a silent shrug.
      if (
        typeof result === "object" &&
        result !== null &&
        "connection_id" in result
      ) {
        if (
          result.status !== "synced" &&
          result.status !== "partially_committed"
        ) {
          setError(syncOutcomeCopy(result.status, result.message));
        }
      } else {
        setError(describeIpcError(result));
      }
    } catch {
      setError("Could not reach the vault service.");
    } finally {
      setRetrying(false);
    }
  }

  return (
    <>
      <TableRow data-state={isSelected ? "selected" : undefined}>
        {selectCell}
        <TableCell className="min-w-0">
          <div className="truncate font-medium">
            {payload.display_hint ?? "Bank connection"} could not sync
          </div>
        </TableCell>
        {/* No amount for a connection problem — keep the cell so columns align. */}
        <TableCell className="w-0 text-right" />
        <TableCell className="w-0">
          <Button size="sm" disabled={retrying} onClick={retry}>
            {retrying ? "Syncing…" : "Retry sync"}
          </Button>
        </TableCell>
        <ExpandCell item={item} expanded={expanded} onToggleExpand={onToggleExpand} />
      </TableRow>
      {expanded && (
        <TableRow>
          <TableCell
            id={`inbox-x-${item.item_id}`}
            colSpan={INBOX_COLUMN_COUNT}
            className="bg-muted/30"
          >
            <div className="flex flex-col gap-3 py-2">
              <div className="flex items-start gap-2 rounded-md bg-warning/10 px-3 py-2 text-sm text-warning">
                <Unplug className="mt-0.5 size-4 shrink-0" aria-hidden />
                <span>
                  <span className="font-medium">{payload.last_error}</span>{" "}
                  {payload.last_synced_at
                    ? "New transactions stop arriving until the connection syncs again."
                    : "This connection has not completed a sync yet."}
                </span>
              </div>
              <p className="text-sm text-muted-foreground">
                An expired connection is re-linked from Settings → Connections
                with a fresh setup token.
              </p>
              {error && (
                <p role="alert" className="text-sm text-loss">
                  {error}
                </p>
              )}
            </div>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

function parseStalePayload(json: string): StaleBalancePayload | null {
  try {
    const raw = JSON.parse(json) as Record<string, unknown>;
    if (
      typeof raw.account_id !== "string" ||
      typeof raw.account_name !== "string"
    ) {
      return null;
    }
    return {
      account_id: raw.account_id,
      account_name: raw.account_name,
      last_observed:
        typeof raw.last_observed === "string" ? raw.last_observed : "",
      days_stale: typeof raw.days_stale === "number" ? raw.days_stale : 0,
    };
  } catch {
    return null;
  }
}

/// The detail an `unreviewed_transaction` item carries (ADR 0032, personal-cfo-4d8.7).
type UnreviewedPayload = {
  title: string;
  amount_minor: number;
  currency: string;
  occurred_at: string;
  account_name: string | null;
};

function parseUnreviewedPayload(json: string): UnreviewedPayload | null {
  try {
    const raw = JSON.parse(json) as Record<string, unknown>;
    if (typeof raw.amount_minor !== "number" || typeof raw.currency !== "string") {
      return null;
    }
    const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
    return {
      title:
        str(raw.memo) ??
        str(raw.counterparty) ??
        str(raw.account_name) ??
        "Transaction",
      amount_minor: raw.amount_minor,
      currency: raw.currency,
      occurred_at: str(raw.occurred_at) ?? "",
      account_name: str(raw.account_name),
    };
  } catch {
    return null;
  }
}

/// The detail a `low_confidence_category` item carries (ADR 0030 addendum, personal-cfo-j5ij):
/// the transaction plus the auto-applied category id + its confidence so the card can show
/// what the user is confirming.
type LowConfidencePayload = {
  title: string;
  amount_minor: number;
  currency: string;
  occurred_at: string;
  account_name: string | null;
  category_id: string | null;
  confidence_bps: number;
};

function parseLowConfidencePayload(json: string): LowConfidencePayload | null {
  try {
    const raw = JSON.parse(json) as Record<string, unknown>;
    if (
      typeof raw.amount_minor !== "number" ||
      typeof raw.currency !== "string"
    ) {
      return null;
    }
    const str = (v: unknown): string | null => (typeof v === "string" ? v : null);
    return {
      title:
        str(raw.memo) ??
        str(raw.counterparty) ??
        str(raw.account_name) ??
        "Transaction",
      amount_minor: raw.amount_minor,
      currency: raw.currency,
      occurred_at: str(raw.occurred_at) ?? "",
      account_name: str(raw.account_name),
      category_id: str(raw.category_id),
      confidence_bps:
        typeof raw.confidence_bps === "number" ? raw.confidence_bps : 0,
    };
  } catch {
    return null;
  }
}

/// `YYYY-MM-DD` `days` from now, in the user's locale — the snooze-until date.
function plusDays(days: number): string {
  const date = new Date();
  date.setDate(date.getDate() + days);
  return date.toLocaleDateString("en-CA");
}

/// The dismiss reasons accepted by the kernel (`INBOX_DISMISS_REASONS`, ci71).
const DISMISS_REASONS: { value: string; label: string }[] = [
  { value: "not_relevant", label: "Not relevant" },
  { value: "already_handled", label: "Already handled" },
  { value: "incorrect", label: "Incorrect" },
  { value: "other", label: "Other" },
];

/// The Money Inbox: the single triage surface for the data-quality exceptions the
/// import pipeline could not auto-resolve (ADR 0014 §7, personal-cfo-tknc). Items
/// are resolved in place — import anyway / skip (personal-cfo-asqy) — or deferred:
/// snooze for a week, or dismiss with a reason (personal-cfo-ci71).
/// `selection`, when passed by the hub (personal-cfo-4d8.24.8), makes the inbox's
/// transaction-backed items participate in the unified cross-list selection (the hub renders
/// the single bulk bar). Standalone (no `selection`) keeps its own selection + bar.
export function MoneyInboxView({
  selection,
}: {
  selection?: TransactionSelection;
} = {}) {
  const {
    items,
    error,
    importAnyway,
    syncConnection,
    skip,
    snooze,
    dismiss,
    markReviewed,
    markReviewedBulk,
    recategorize,
    acceptAllLowConfidence,
  } = useMoneyInbox();
  const { accounts, archiveAccount } = useAccounts();
  const { categories } = useCategories();
  const [importing, setImporting] = useState(false);
  const [settingBalance, setSettingBalance] = useState<AccountViewDto | null>(
    null,
  );
  // The duplicate inbox item currently open in the side-by-side Review panel.
  const [reviewing, setReviewing] = useState<MoneyInboxItemDto | null>(null);
  // Reviewing means LOOKING at the details — category, tags, notes, splits — then
  // marking reviewed (feedback 2026-07-03). Transaction-backed items open the same
  // detail drawer the Transactions tab uses, and support multi-select bulk actions.
  const {
    transactions,
    recategorize: recategorizeTxn,
    deleteTransaction,
    setReviewed,
  } = useTransactions();
  const { tags, createTag, setTags, setNote } = useTags();
  const [detailId, setDetailId] = useState<string | null>(null);
  // The whole section collapses to a single header row (personal-cfo-4d8.24.12). Not
  // persisted — kept dep-free/minimal (a localStorage mirror is a possible follow-up).
  const [sectionOpen, setSectionOpen] = useState(true);
  // Which rows have their inline detail expander open. Reset on page change (below).
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  function toggleExpand(itemId: string) {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(itemId)) next.delete(itemId);
      else next.add(itemId);
      return next;
    });
  }
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  function toggleSelect(transactionId: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(transactionId)) next.delete(transactionId);
      else next.add(transactionId);
      return next;
    });
  }

  const hasAccounts = accounts !== null && accounts.length > 0;
  const lowConfidenceCount =
    items?.filter((item) => item.item_kind === "low_confidence_category")
      .length ?? 0;
  // Items whose target is a ledger transaction are the multi-selectable "review" set.
  const isTransactionItem = (item: MoneyInboxItemDto) =>
    item.item_kind === "unreviewed_transaction" ||
    item.item_kind === "low_confidence_category";
  const selectableIds = (items ?? [])
    .filter(isTransactionItem)
    .map((item) => item.target_id);
  // A big import can flood the inbox — paginate it like the transactions list
  // (feedback 2026-07-03), sticky page size.
  const pages = usePagination(items ?? [], "pcfo.pageSize.moneyInbox");
  // Resolve the CURRENT PAGE's rows by id (personal-cfo-4d8.25.15): the recent-window
  // transaction list caps at 200, so a large inbox left rows beyond it without checkbox
  // or bulk data — the by-ids read covers every page.
  const selectableSet = new Set(selectableIds);
  const pageTxnIds = pages.pageItems
    .filter((item) => selectableSet.has(item.target_id))
    .map((item) => item.target_id);
  const pageRowsKey = [...pageTxnIds].sort().join(",");
  const pageRows = useQuery({
    queryKey: ["transactions", "rowsByIds", pageRowsKey],
    enabled: pageTxnIds.length > 0,
    queryFn: () =>
      ipcQuery(
        commands.transactionRowsByIds(pageTxnIds),
        "Could not load transaction details.",
      ),
  });
  // The row DTOs behind the selectable items, for the shared selection + bulk bar:
  // the recent-window list as the warm base, overridden by the page's by-ids rows.
  const inboxRowById = new Map([
    ...(transactions ?? [])
      .filter((t) => selectableSet.has(t.transaction_id))
      .map((t) => [t.transaction_id, t] as const),
    ...(pageRows.data ?? []).map((t) => [t.transaction_id, t] as const),
  ]);
  const detailTxn =
    detailId === null
      ? null
      : (inboxRowById.get(detailId) ??
        transactions?.find((t) => t.transaction_id === detailId) ??
        null);

  // Selected rows leave the view on a page change; acting on off-screen rows would be
  // surprising, so the standalone selection resets (mirrors TransactionsView, j0cg.4). The
  // hub's unified selection persists by id across changes (personal-cfo-4d8.24.8, AC-6).
  const currentPage = pages.page;
  useEffect(() => {
    if (!selection) setSelectedIds(new Set());
    // Expanders are per-page UI state; a page change hides the rows they belong to.
    setExpandedIds(new Set());
  }, [currentPage, selection]);

  // When controlled by the hub, publish the inbox's selectable rows so the shared selection
  // can resolve + dedupe them against the Activity list (personal-cfo-4d8.24.8). Keyed on
  // id + tag_ids so a refetch that changes a selected row's tags re-registers the FRESH DTO
  // (the bulk-tag merge must not read stale tag_ids).
  const registerVisible = selection?.setVisibleRows;
  const inboxRowKey = Array.from(inboxRowById.values())
    .map((t) => `${t.transaction_id}:${t.tag_ids.join("|")}`)
    .join(",");
  useEffect(() => {
    registerVisible?.("inbox", Array.from(inboxRowById.values()));
    // eslint-disable-next-line react-hooks/exhaustive-deps -- inboxRowKey captures the row set + tags
  }, [registerVisible, inboxRowKey]);

  // ----- Card Review + whole-inbox selection (personal-cfo-4d8.25.16/.17) -----
  const [cardReview, setCardReview] = useState<{
    queueIds: string[];
    rowsById: Map<string, TransactionRowDto>;
  } | null>(null);
  const [inboxFetchBusy, setInboxFetchBusy] = useState(false);
  const [inboxFetchError, setInboxFetchError] = useState<string | null>(null);

  /// Fetch full rows for EVERY selectable inbox item in one round-trip — the shared
  /// resolver behind "Select all in inbox" and Card Review. Failures surface as an
  /// alert and never strand the busy flag (adversarial review of 4d8.25.16).
  async function fetchAllInboxRows(): Promise<Map<string, TransactionRowDto> | null> {
    setInboxFetchBusy(true);
    setInboxFetchError(null);
    try {
      const result = await commands.transactionRowsByIds(selectableIds);
      if (result.status !== "ok") {
        setInboxFetchError(describeIpcError(result.error));
        return null;
      }
      return new Map(result.data.map((row) => [row.transaction_id, row] as const));
    } catch {
      setInboxFetchError("Could not load the inbox's transactions.");
      return null;
    } finally {
      setInboxFetchBusy(false);
    }
  }

  async function selectAllInbox() {
    if (!selection) return;
    const rows = await fetchAllInboxRows();
    if (rows) selection.selectRows([...rows.values()]);
  }

  async function openCardReview() {
    const rows = await fetchAllInboxRows();
    if (!rows) return;
    // Queue in inbox order (ADR 0014 default sort), limited to items whose row resolved.
    setCardReview({
      queueIds: selectableIds.filter((id) => rows.has(id)),
      rowsById: rows,
    });
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        {/* The heading doubles as the collapse toggle (personal-cfo-4d8.24.12). Kept as an
            <h2> so it stays a landmark heading (the hub asserts on it); the interactive
            control is the nested <button>. The count badge lives inside the button so it
            stays visible even when the section is collapsed. */}
        <h2 className="text-base font-semibold tracking-tight">
          <button
            type="button"
            onClick={() => setSectionOpen((open) => !open)}
            aria-expanded={sectionOpen}
            className="flex items-center gap-2 hover:text-foreground/80"
          >
            <ChevronDown
              className={cn(
                "size-4 transition-transform",
                !sectionOpen && "-rotate-90",
              )}
              aria-hidden
            />
            Money Inbox
            {items !== null && items.length > 0 && (
              <span className="rounded-full bg-warning/15 px-2 py-0.5 text-xs font-medium tabular-nums text-warning">
                {items.length}
              </span>
            )}
          </button>
        </h2>
        <div className="flex items-center gap-2">
          {selection && selectableIds.length > 0 && (
            <Button
              size="sm"
              variant="ghost"
              disabled={inboxFetchBusy}
              onClick={() => void selectAllInbox()}
            >
              Select all {selectableIds.length} in inbox
            </Button>
          )}
          {selectableIds.length > 0 && (
            <Button
              size="sm"
              variant="outline"
              disabled={inboxFetchBusy}
              onClick={() => void openCardReview()}
            >
              <Layers aria-hidden />
              Card Review
              <span className="rounded-full bg-warning/15 px-1.5 py-0.5 text-xs font-medium tabular-nums text-warning">
                {selectableIds.length}
              </span>
            </Button>
          )}
          {hasAccounts && (
            <Button size="sm" onClick={() => setImporting(true)}>
              <Upload aria-hidden />
              Import file
            </Button>
          )}
        </div>
      </div>

      {importing && accounts && (
        <ImportFileDialog
          accounts={accounts}
          onClose={() => setImporting(false)}
        />
      )}

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}
      {inboxFetchError && (
        <p role="alert" className="text-sm text-loss">
          {inboxFetchError}
        </p>
      )}

      {sectionOpen && (
        <>
          {lowConfidenceCount >= 2 && (
            <LowConfidenceBanner
              count={lowConfidenceCount}
              onAcceptAll={acceptAllLowConfidence}
            />
          )}

          {items === null ? (
            <div className="flex items-center gap-2 py-3 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden />
              Loading your Money Inbox…
            </div>
          ) : items.length === 0 ? (
            <div className="flex items-center gap-2 rounded-md border bg-muted/30 px-4 py-2.5 text-sm text-muted-foreground">
              <Inbox className="size-4 shrink-0" aria-hidden />
              You’re all caught up — items that need a look show up here.
            </div>
          ) : (
            <>
              {!selection && selectedIds.size > 0 && (
                <BulkTransactionActions
                  selected={(transactions ?? []).filter((t) =>
                    selectedIds.has(t.transaction_id),
                  )}
                  total={selectableIds.length}
                  categories={categories}
                  tags={tags}
                  recategorize={recategorizeTxn}
                  setReviewed={setReviewed}
                  deleteTransaction={deleteTransaction}
                  setTags={setTags}
                  createTag={createTag}
                  bulkMarkReviewed={markReviewedBulk}
                  onSelectAll={() => setSelectedIds(new Set(selectableIds))}
                  onSelectionChange={(ids) => setSelectedIds(new Set(ids))}
                />
              )}
              {/* Compact table (personal-cfo-4d8.24.12), mirroring TransactionsView (H4a).
                  Headerless: every accessible name comes from the per-row controls. Five
                  columns — [select, summary, amount, primary action, expand] — are ALWAYS
                  emitted (empty when N/A) so every full-width row spans INBOX_COLUMN_COUNT. Kind-specific
                  secondary actions/detail live in the inline colSpan expander row. */}
              <Card>
                <CardContent className="p-0">
                  <Table className="min-w-[560px]">
                    <TableBody>
                      {pages.pageItems.flatMap((item) => {
                        const expanded = expandedIds.has(item.item_id);
                        const selectable =
                          isTransactionItem(item) &&
                          (!selection || inboxRowById.has(item.target_id));
                        const isSelected =
                          isTransactionItem(item) &&
                          (selection
                            ? selection.isSelected(item.target_id)
                            : selectedIds.has(item.target_id));
                        // The gated H3 checkbox (personal-cfo-4d8.24.8) — verbatim, only the
                        // layout-coupled `mt-6` dropped (the table cell centers it). The empty
                        // cell renders for non-selectable kinds so every row keeps 5 columns.
                        const selectCell = (
                          <TableCell className="w-0">
                            {selectable && (
                              <input
                                type="checkbox"
                                className="size-4 shrink-0 accent-primary"
                                checked={
                                  selection
                                    ? selection.isSelected(item.target_id)
                                    : selectedIds.has(item.target_id)
                                }
                                onChange={() => {
                                  if (selection) {
                                    const row = inboxRowById.get(item.target_id);
                                    if (row) selection.toggle(row);
                                  } else {
                                    toggleSelect(item.target_id);
                                  }
                                }}
                                aria-label="Select for bulk actions"
                              />
                            )}
                          </TableCell>
                        );
                        const common = {
                          item,
                          selectCell,
                          isSelected,
                          expanded,
                          onToggleExpand: () => toggleExpand(item.item_id),
                        };
                        if (item.item_kind === "connector_error")
                          return [
                            <ConnectorErrorRow
                              key={item.item_id}
                              {...common}
                              onRetrySync={syncConnection}
                            />,
                          ];
                        if (item.item_kind === "stale_balance")
                          return [
                            <StaleBalanceRow
                              key={item.item_id}
                              {...common}
                              onUpdateBalance={(accountId) => {
                                const account =
                                  accounts?.find(
                                    (candidate) => candidate.id === accountId,
                                  ) ?? null;
                                if (account) setSettingBalance(account);
                              }}
                              onArchive={archiveAccount}
                            />,
                          ];
                        if (item.item_kind === "low_confidence_category")
                          return [
                            <LowConfidenceCategoryRow
                              key={item.item_id}
                              {...common}
                              categories={categories}
                              onAccept={() => markReviewed(item.target_id)}
                              onRecategorize={(categoryId) =>
                                recategorize(item.target_id, categoryId)
                              }
                              onOpenDetails={() => setDetailId(item.target_id)}
                            />,
                          ];
                        if (item.item_kind === "unreviewed_transaction")
                          return [
                            <UnreviewedTransactionRow
                              key={item.item_id}
                              {...common}
                              onReviewed={() => markReviewed(item.target_id)}
                              onOpenDetails={() => setDetailId(item.target_id)}
                            />,
                          ];
                        return [
                          <ImportedWaitingCommitRow
                            key={item.item_id}
                            {...common}
                            onReview={() => setReviewing(item)}
                            onImportAnyway={() => importAnyway(item.target_id)}
                            onSkip={() => skip(item.target_id)}
                            onSnooze={() => snooze(item.item_id, plusDays(7))}
                            onDismiss={(reason) => dismiss(item.item_id, reason)}
                          />,
                        ];
                      })}
                    </TableBody>
                  </Table>
                </CardContent>
              </Card>
              {pages.total > PAGE_SIZE_OPTIONS[0] && (
                <PaginationControls pagination={pages} noun="items" />
              )}
            </>
          )}
        </>
      )}

      {detailTxn && (
        <TransactionDetailDrawer
          transaction={detailTxn}
          categories={categories}
          onRecategorize={recategorizeTxn}
          onDelete={deleteTransaction}
          onSetReviewed={setReviewed}
          tags={tags}
          onCreateTag={createTag}
          onSetTags={setTags}
          onSetNote={setNote}
          onClose={() => setDetailId(null)}
        />
      )}

      {cardReview && (
        <CardReviewModal
          queueIds={cardReview.queueIds}
          rowsById={cardReview.rowsById}
          categories={categories}
          tags={tags}
          onMarkReviewed={markReviewed}
          onRecategorize={recategorizeTxn}
          onCreateTag={createTag}
          onSetTags={setTags}
          onSetNote={setNote}
          onClose={() => setCardReview(null)}
        />
      )}

      {settingBalance && (
        <SetBalanceModal
          account={settingBalance}
          onClose={() => setSettingBalance(null)}
          // The transaction-reconcile handoff lives on the Accounts tab; from the
          // inbox a plain balance update is enough to clear the stale item.
          onExplainWithTransactions={() => setSettingBalance(null)}
        />
      )}

      {reviewing &&
        (() => {
          const payload = parsePayload(reviewing.payload_json);
          if (payload === null) return null;
          return (
            <DuplicateReviewPanel
              stagedTxnId={reviewing.target_id}
              incoming={{
                merchant:
                  payload.merchant ??
                  payload.description ??
                  "Imported transaction",
                amount: {
                  minor_units: payload.amount_minor,
                  currency: payload.currency,
                },
                date: payload.posted_at,
                account: payload.account_name,
                source: payload.source_name,
              }}
              reason={payload.dedupe_reason}
              onSkip={() => skip(reviewing.target_id)}
              onImportAnyway={() => importAnyway(reviewing.target_id)}
              onClose={() => setReviewing(null)}
            />
          );
        })()}
    </div>
  );
}

type Pending = "import" | "skip" | "snooze" | "dismiss" | null;

/// The column count every kind row emits: [select, summary, amount, primary action,
/// expand]. Every full-width row below spans it.
///
/// A named constant rather than a `colSpan={5}` literal per site, because there were
/// SIX of those across four components and nothing checked them: add a column and the
/// expanders silently misalign. `DataTable` makes this structural for the surfaces that
/// migrated, but Money Inbox deliberately does not migrate (personal-cfo-9krd — its rows
/// are polymorphic with state shared across cells, which a column-def API cannot host),
/// so it removes the hazard directly instead of inheriting the fix.
const INBOX_COLUMN_COUNT = 5;

/// Props every kind row-fragment shares. The parent owns selection state (H3), so it
/// builds the gated checkbox `selectCell` node and passes it in, keeping the selection
/// JSX out of the per-kind components; `isSelected` drives the row's `data-state`.
type RowCommon = {
  item: MoneyInboxItemDto;
  selectCell: ReactNode;
  isSelected: boolean;
  expanded: boolean;
  onToggleExpand: () => void;
};

/// The always-present expand-toggle cell (personal-cfo-4d8.24.12). Empty for kinds with
/// no expander (unreviewed_transaction) so the 5-column grid stays aligned.
function ExpandCell({
  item,
  expanded,
  onToggleExpand,
}: {
  item: MoneyInboxItemDto;
  expanded: boolean;
  onToggleExpand: () => void;
}) {
  return (
    <TableCell className="w-0">
      <button
        type="button"
        onClick={onToggleExpand}
        aria-label={expanded ? "Hide details" : "Show details"}
        aria-expanded={expanded}
        aria-controls={`inbox-x-${item.item_id}`}
        className="shrink-0 rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
      >
        <ChevronDown
          className={cn("size-4 transition-transform", expanded && "rotate-180")}
          aria-hidden
        />
      </button>
    </TableCell>
  );
}

function ImportedWaitingCommitRow({
  item,
  selectCell,
  isSelected,
  expanded,
  onToggleExpand,
  onReview,
  onImportAnyway,
  onSkip,
  onSnooze,
  onDismiss,
}: RowCommon & {
  onReview: () => void;
  onImportAnyway: () => Promise<IpcError | null>;
  onSkip: () => Promise<IpcError | null>;
  onSnooze: () => Promise<IpcError | null>;
  onDismiss: (reason: string) => Promise<IpcError | null>;
}) {
  const [pending, setPending] = useState<Pending>(null);
  const [error, setError] = useState<string | null>(null);
  const [dismissing, setDismissing] = useState(false);
  const [reason, setReason] = useState(DISMISS_REASONS[0]?.value ?? "other");
  const payload = parsePayload(item.payload_json);

  async function run(action: Pending, fn: () => Promise<IpcError | null>) {
    setPending(action);
    setError(null);
    const failure = await fn();
    // On success the inbox re-fetches and this row unmounts; on failure we keep
    // the row and surface the reason.
    if (failure) {
      setError(describeIpcError(failure));
      setPending(null);
    }
  }

  const title =
    payload?.merchant ?? payload?.description ?? "Imported transaction";
  // A committed counterpart exists → lead with Review (the side-by-side panel is where
  // the decision is made, personal-cfo-4d8.8); otherwise the primary is Import anyway.
  const isReview = Boolean(payload?.suspected_committed_txn_id);

  return (
    <>
      <TableRow data-state={isSelected ? "selected" : undefined}>
        {selectCell}
        <TableCell className="min-w-0">
          <div className="truncate font-medium">{title}</div>
          <div className="truncate text-xs text-muted-foreground">
            {payload?.posted_at ? formatIsoDate(payload.posted_at) : null}
            {payload?.account_name ? ` · ${payload.account_name}` : null}
          </div>
        </TableCell>
        <TableCell
          className={cn(
            "w-0 text-right font-medium tabular-nums",
            payload &&
              signedAmountClass({
                minor_units: payload.amount_minor,
                currency: payload.currency,
              }),
          )}
        >
          {payload &&
            formatMoney({
              minor_units: payload.amount_minor,
              currency: payload.currency,
            })}
        </TableCell>
        <TableCell className="w-0">
          {isReview ? (
            <Button size="sm" onClick={onReview}>
              Review
              <ArrowRight aria-hidden />
            </Button>
          ) : (
            <Button
              size="sm"
              disabled={pending !== null}
              onClick={() => run("import", onImportAnyway)}
            >
              <Check aria-hidden />
              {pending === "import" ? "Importing…" : "Import anyway"}
            </Button>
          )}
        </TableCell>
        <ExpandCell
          item={item}
          expanded={expanded}
          onToggleExpand={onToggleExpand}
        />
      </TableRow>
      {/* Always-visible so a failed PRIMARY action (Import anyway) is surfaced even when
          the expander is collapsed (personal-cfo-4d8.24.12 review). */}
      {error && (
        <TableRow className="hover:bg-transparent">
          <TableCell colSpan={INBOX_COLUMN_COUNT} className="pt-0">
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          </TableCell>
        </TableRow>
      )}
      {expanded && (
        <TableRow className="bg-muted/20 hover:bg-transparent">
          <TableCell colSpan={INBOX_COLUMN_COUNT} className="p-0" id={`inbox-x-${item.item_id}`}>
            <div className="flex flex-col gap-3 p-4">
              <div className="flex items-start gap-2 rounded-md bg-warning/10 px-3 py-2 text-sm text-warning">
                <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden />
                <span>
                  <span className="font-medium">Possible duplicate.</span>{" "}
                  {payload?.dedupe_reason ??
                    "This looks like a transaction you already have."}
                </span>
              </div>

              {payload?.source_name && (
                <div className="text-xs text-muted-foreground">
                  from {payload.source_name}
                </div>
              )}

              <div className="flex flex-wrap items-center gap-2">
                {dismissing ? (
                  <>
                    <select
                      aria-label="Dismiss reason"
                      className="h-8 rounded-md border border-input bg-background px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                      value={reason}
                      onChange={(event) => setReason(event.target.value)}
                    >
                      {DISMISS_REASONS.map((option) => (
                        <option key={option.value} value={option.value}>
                          {option.label}
                        </option>
                      ))}
                    </select>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={pending !== null}
                      onClick={() => run("dismiss", () => onDismiss(reason))}
                    >
                      {pending === "dismiss" ? "Dismissing…" : "Confirm"}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setDismissing(false)}
                    >
                      Cancel
                    </Button>
                  </>
                ) : (
                  <>
                    {!isReview && (
                      <Button
                        variant="ghost"
                        size="sm"
                        disabled={pending !== null}
                        onClick={() => run("skip", onSkip)}
                      >
                        <X aria-hidden />
                        {pending === "skip" ? "Skipping…" : "Skip"}
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={pending !== null}
                      onClick={() => run("snooze", onSnooze)}
                    >
                      <MoonStar aria-hidden />
                      {pending === "snooze" ? "Snoozing…" : "Snooze 1 week"}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={pending !== null}
                      onClick={() => setDismissing(true)}
                    >
                      Dismiss
                    </Button>
                  </>
                )}
              </div>
            </div>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

// A stale-balance nudge (personal-cfo-r52x): an account whose balance hasn't been
// confirmed in a while. Resolve by updating the balance (opens the set-balance
// modal) or archiving the account — either clears the item on the next read.
function StaleBalanceRow({
  item,
  selectCell,
  isSelected,
  expanded,
  onToggleExpand,
  onUpdateBalance,
  onArchive,
}: RowCommon & {
  onUpdateBalance: (accountId: string) => void;
  onArchive: (accountId: string) => Promise<IpcError | null>;
}) {
  const [archiving, setArchiving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const payload = parseStalePayload(item.payload_json);
  if (payload === null) return null;

  async function archive() {
    if (payload === null) return;
    setArchiving(true);
    setError(null);
    const failure = await onArchive(payload.account_id);
    // On success the inbox re-fetches and this row unmounts.
    if (failure) {
      setError(describeIpcError(failure));
      setArchiving(false);
    }
  }

  return (
    <>
      <TableRow data-state={isSelected ? "selected" : undefined}>
        {selectCell}
        <TableCell className="min-w-0">
          <div className="truncate font-medium">{payload.account_name}</div>
        </TableCell>
        {/* No amount for a stale-balance nudge — keep the cell so columns align. */}
        <TableCell className="w-0 text-right" />
        <TableCell className="w-0">
          <Button size="sm" onClick={() => onUpdateBalance(payload.account_id)}>
            Update balance
          </Button>
        </TableCell>
        <ExpandCell
          item={item}
          expanded={expanded}
          onToggleExpand={onToggleExpand}
        />
      </TableRow>
      {expanded && (
        <TableRow className="bg-muted/20 hover:bg-transparent">
          <TableCell colSpan={INBOX_COLUMN_COUNT} className="p-0" id={`inbox-x-${item.item_id}`}>
            <div className="flex flex-col gap-3 p-4">
              <div className="flex items-start gap-2 rounded-md bg-warning/10 px-3 py-2 text-sm text-warning">
                <Clock className="mt-0.5 size-4 shrink-0" aria-hidden />
                <span>
                  <span className="font-medium">
                    Balance may be out of date.
                  </span>{" "}
                  Last set{" "}
                  {payload.last_observed
                    ? formatIsoDate(payload.last_observed)
                    : "a while ago"}{" "}
                  ({payload.days_stale} days ago).
                </span>
              </div>

              {error && (
                <p role="alert" className="text-sm text-loss">
                  {error}
                </p>
              )}

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={archiving}
                  onClick={archive}
                >
                  {archiving ? "Archiving…" : "Archive account"}
                </Button>
              </div>
            </div>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

/// An imported transaction waiting to be reviewed (ADR 0032, personal-cfo-4d8.7).
/// "Mark reviewed" clears it from the inbox (and flips the transaction's reviewed flag).
function UnreviewedTransactionRow({
  item,
  selectCell,
  isSelected,
  onReviewed,
  onOpenDetails,
}: Omit<RowCommon, "expanded" | "onToggleExpand"> & {
  onReviewed: () => Promise<IpcError | null>;
  /// Open the full detail drawer — category, tags, notes, splits — so "reviewed"
  /// means actually reviewed (feedback 2026-07-03).
  onOpenDetails: () => void;
}) {
  const [reviewing, setReviewing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const payload = parseUnreviewedPayload(item.payload_json);
  if (payload === null) return null;

  async function markReviewed() {
    setReviewing(true);
    setError(null);
    const failure = await onReviewed();
    // On success the inbox re-fetches and this row unmounts.
    if (failure) {
      setError(describeIpcError(failure));
      setReviewing(false);
    }
  }

  return (
    <>
      <TableRow data-state={isSelected ? "selected" : undefined}>
        {selectCell}
        <TableCell className="min-w-0">
          <button
            type="button"
            onClick={onOpenDetails}
            className="flex min-w-0 flex-col items-start rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="max-w-full truncate font-medium">
              {payload.title}
            </span>
            <span className="max-w-full truncate text-xs text-muted-foreground">
              Imported · click to review details
              {payload.account_name ? ` · ${payload.account_name}` : ""}
              {payload.occurred_at
                ? ` · ${formatIsoDate(payload.occurred_at)}`
                : ""}
            </span>
          </button>
        </TableCell>
        <TableCell
          className={cn(
            "w-0 text-right font-medium tabular-nums",
            signedAmountClass({
              minor_units: payload.amount_minor,
              currency: payload.currency,
            }),
          )}
        >
          {formatMoney({
            minor_units: payload.amount_minor,
            currency: payload.currency,
          })}
        </TableCell>
        <TableCell className="w-0">
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            disabled={reviewing}
            onClick={markReviewed}
          >
            <Check aria-hidden />
            {reviewing ? "Marking…" : "Mark reviewed"}
          </Button>
        </TableCell>
        {/* No expander for an already-compact unreviewed row — keep the cell empty. */}
        <TableCell className="w-0" />
      </TableRow>
      {error && (
        <TableRow className="hover:bg-transparent">
          <TableCell colSpan={INBOX_COLUMN_COUNT} className="pt-0">
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}

/// A bulk "accept all" banner for the low-confidence-category queue (personal-cfo-j5ij): it
/// confirms in two steps (the first click reveals Confirm/Cancel) and reports the count, so a
/// sweeping mark-reviewed is never a single misclick.
function LowConfidenceBanner({
  count,
  onAcceptAll,
}: {
  count: number;
  onAcceptAll: () => Promise<{ count: number } | { error: IpcError }>;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function acceptAll() {
    setBusy(true);
    setError(null);
    const result = await onAcceptAll();
    setBusy(false);
    setConfirming(false);
    if ("error" in result) setError(describeIpcError(result.error));
  }

  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border bg-info/5 px-4 py-2.5 text-sm">
      <span className="flex items-center gap-2 text-muted-foreground">
        <Sparkles className="size-4 shrink-0 text-info" aria-hidden />
        {count} transactions were auto-categorized with low confidence.
      </span>
      <div className="flex items-center gap-2">
        {error && <span className="text-xs text-loss">{error}</span>}
        {confirming ? (
          <>
            <Button size="sm" disabled={busy} onClick={acceptAll}>
              {busy ? "Accepting…" : `Accept all ${count}`}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => setConfirming(false)}
            >
              Cancel
            </Button>
          </>
        ) : (
          <Button
            variant="outline"
            size="sm"
            onClick={() => setConfirming(true)}
          >
            <Check aria-hidden />
            Accept all
          </Button>
        )}
      </div>
    </div>
  );
}

/// A transaction auto-categorized below the confidence threshold (ADR 0030 addendum,
/// personal-cfo-j5ij/-uc95). The user confirms the suggested category ("Accept" → mark
/// reviewed, keeping `source=rule`) or changes it (a picker → recategorize as `source=user`);
/// either drops it from the queue.
function LowConfidenceCategoryRow({
  item,
  selectCell,
  isSelected,
  expanded,
  onToggleExpand,
  categories,
  onAccept,
  onRecategorize,
  onOpenDetails,
}: RowCommon & {
  categories: CategoryDto[] | null;
  onAccept: () => Promise<IpcError | null>;
  onRecategorize: (categoryId: string | null) => Promise<IpcError | null>;
  /// Open the full detail drawer (tags / notes / splits) before deciding.
  onOpenDetails: () => void;
}) {
  const [pending, setPending] = useState<"accept" | "recategorize" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const payload = parseLowConfidencePayload(item.payload_json);
  const { label } = categoryLabels(categories ?? []);
  if (payload === null) return null;

  const suggested = label(payload.category_id) ?? "a category";
  const percent = Math.round(payload.confidence_bps / 100);

  async function run(
    action: "accept" | "recategorize",
    fn: () => Promise<IpcError | null>,
  ) {
    setPending(action);
    setError(null);
    const failure = await fn();
    // On success the inbox re-fetches and this row unmounts; on failure keep it.
    if (failure) {
      setError(describeIpcError(failure));
      setPending(null);
    }
  }

  return (
    <>
      <TableRow data-state={isSelected ? "selected" : undefined}>
        {selectCell}
        <TableCell className="min-w-0">
          <button
            type="button"
            onClick={onOpenDetails}
            className="flex min-w-0 flex-col items-start rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span className="max-w-full truncate font-medium">
              {payload.title}
            </span>
            <span className="max-w-full truncate text-xs text-muted-foreground">
              {payload.account_name ? `${payload.account_name} · ` : ""}
              {payload.occurred_at ? formatIsoDate(payload.occurred_at) : ""}
            </span>
          </button>
        </TableCell>
        <TableCell
          className={cn(
            "w-0 text-right font-medium tabular-nums",
            signedAmountClass({
              minor_units: payload.amount_minor,
              currency: payload.currency,
            }),
          )}
        >
          {formatMoney({
            minor_units: payload.amount_minor,
            currency: payload.currency,
          })}
        </TableCell>
        <TableCell className="w-0">
          <Button
            size="sm"
            disabled={pending !== null}
            onClick={() => void run("accept", onAccept)}
          >
            <Check aria-hidden />
            {pending === "accept" ? "Accepting…" : "Accept"}
          </Button>
        </TableCell>
        <ExpandCell
          item={item}
          expanded={expanded}
          onToggleExpand={onToggleExpand}
        />
      </TableRow>
      {/* Always-visible so a failed PRIMARY action (Accept) is surfaced even when the
          expander is collapsed (personal-cfo-4d8.24.12 review). */}
      {error && (
        <TableRow className="hover:bg-transparent">
          <TableCell colSpan={INBOX_COLUMN_COUNT} className="pt-0">
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          </TableCell>
        </TableRow>
      )}
      {expanded && (
        <TableRow className="bg-muted/20 hover:bg-transparent">
          <TableCell colSpan={INBOX_COLUMN_COUNT} className="p-0" id={`inbox-x-${item.item_id}`}>
            <div className="flex flex-col gap-3 p-4">
              <div className="flex items-start gap-2 rounded-md bg-info/10 px-3 py-2 text-sm text-info">
                <Sparkles className="mt-0.5 size-4 shrink-0" aria-hidden />
                <span>
                  Auto-categorized as{" "}
                  <span className="font-medium">{suggested}</span> ({percent}%
                  confidence). Confirm it or pick a better fit.
                </span>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <div className="w-44">
                  <CategoryCombobox
                    categories={categories ?? []}
                    aria-label="Change category"
                    value={payload.category_id ?? null}
                    disabled={pending !== null}
                    onSelect={(next) =>
                      void run("recategorize", () => onRecategorize(next))
                    }
                    clearLabel="Uncategorized"
                    buttonClassName="h-8"
                  />
                </div>
              </div>
            </div>
          </TableCell>
        </TableRow>
      )}
    </>
  );
}
