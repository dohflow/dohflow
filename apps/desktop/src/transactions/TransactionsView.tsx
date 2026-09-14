import { useEffect, useMemo, useRef, useState, type ChangeEvent } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { ArrowLeftRight, ArrowRight, ChevronDown, FileText, Loader2, Paperclip, Plus, Search, Sparkles, Split, Trash2 } from "lucide-react";

import type {
  AccountViewDto,
  CategoryDto,
  CreateRecurringBillInput,
  CreateRecurringTransferInput,
  IpcError,
  RecordTransactionInput,
  RecordTransferInput,
  TagViewDto,
  TransactionRowDto,
} from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardContent } from "@/components/ui/card";
import { describeIpcError } from "@/vault/useVault";
import {
  dollarsToMinorUnits,
  formatBytes,
  formatDate,
  formatMoney,
  formatSignedMoney,
  signedAmountClass,
} from "@/lib/format";
import { mintIdempotencyKey } from "@/lib/idempotency";
import {
  CLASSIC_FREQUENCIES,
  INTERVAL_UNITS,
  buildIntervalToken,
  isValidIntervalCount,
  type IntervalUnit,
} from "@/lib/frequency";
import { useAccounts } from "@/accounts/useAccounts";
import { useCategories } from "@/categories/useCategories";
import { categoryLabels, type CategoryMeta } from "@/categories/labels";
import { PaginationControls } from "@/components/ui/pagination";
import {
  DataTable,
  type DataTableColumn,
} from "@/components/ui/data-table";
import { PAGE_SIZE_OPTIONS } from "@/lib/usePagination";
import { cn } from "@/lib/utils";
import { useTags } from "@/tags/useTags";
import { useBills } from "@/bills/useBills";
import { TagChip } from "@/tags/TagChip";
import { BulkTransactionActions } from "./BulkTransactionActions";
import type { TransactionSelection } from "./useTransactionSelection";
import { useTransactionSplits } from "./useSplits";
import { useTransactions } from "./useTransactions";
import { usePagedTransactions } from "./usePagedTransactions";
import { useRecurringTransfers } from "./useRecurringTransfers";
import { TransactionDetailDrawer } from "./TransactionDetailDrawer";
import { TransactionFilterBar } from "./TransactionFilterBar";
import { ExportCsvButton } from "./ExportCsvButton";
import {
  activeFilterCount,
  EMPTY_FILTERS,
  type TransactionFilters,
  type TransactionSort,
} from "./filters";
import { ScopeStrip } from "./ScopeStrip";
import { scopeChips, CLEARED_SCOPE } from "./scopeChips";
import { SpendByCategoryCard } from "./SpendByCategoryCard";

const SELECT_CLASS =
  "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

/// Roles you can move money OUT of: liquid cash, or an investment being liquidated (j0cg.2).
const SOURCE_ROLES = ["liquid_cash", "investment_asset"];
/// Every role a transfer can touch (source or destination) — real assets / virtual accounts aren't
/// transfer legs.
const TRANSFER_ROLES = new Set([
  "liquid_cash",
  "investment_asset",
  "credit_facility",
  "loan_liability",
]);

/// A short parenthetical describing an account's role in a transfer dropdown ("" for plain cash).
function roleHint(role: string): string {
  switch (role) {
    case "investment_asset":
      return " (investment)";
    case "credit_facility":
      return " (card)";
    case "loan_liability":
      return " (loan)";
    default:
      return "";
  }
}

/// A plain-language line describing what a non-plain transfer does, so the user sees they're paying
/// a debt, contributing, or withdrawing — not just moving cash. `null` for a plain cash↔cash move.
function transferKindHint(
  source?: AccountViewDto,
  dest?: AccountViewDto,
  recurring = false,
): string | null {
  if (!source || !dest) return null;
  if (source.cashflow_role === "investment_asset") {
    return "Withdrawing from your investment into cash.";
  }
  switch (dest.cashflow_role) {
    case "investment_asset":
      return recurring
        ? "A recurring contribution into your investment (dollar-cost averaging)."
        : "Contributing to your investment.";
    case "credit_facility":
      return "Paying down this card.";
    case "loan_liability":
      return "Paying down this loan.";
    default:
      return null;
  }
}

/// Today as `YYYY-MM-DD` in the user's locale (the `<input type="date">` value).
function today(): string {
  return new Date().toLocaleDateString("en-CA");
}

/// `embedded` hides the section's own heading when it sits inside the unified
/// Transactions hub (personal-cfo-xdbm), which provides the framing. `selection`, when
/// passed by the hub (personal-cfo-4d8.24.8), makes this list a controlled participant in
/// the unified cross-list selection — its rows check against the shared selection and the
/// hub renders the single bulk bar. Standalone (no `selection`) keeps its own selection +
/// bar (used by tests).
export function TransactionsView({
  embedded = false,
  selection,
  accountScope,
}: {
  embedded?: boolean;
  selection?: TransactionSelection;
  /// Pin the list to these accounts and do not let it be widened past them
  /// (ADR 0057 §3). The Debt page's selector owns this scope; the filter bar therefore
  /// stops offering an account facet, and every other filter stays available — the scope
  /// is a floor on what is shown, not a replacement for filtering within it.
  accountScope?: string[];
} = {}) {
  // Mutations only — the rows come from the server-paged query below
  // (personal-cfo-3fdd.1), so the plain 200-row list fetch is skipped.
  const {
    addTransactionWithMeta,
    recategorize,
    autoCategorize,
    transfer,
    deleteTransaction,
    setReviewed,
  } = useTransactions({ list: false });
  const { tags, createTag, setTags, setNote } = useTags();
  // A record succeeded but a metadata follow-up (category/tags/note) failed — the
  // transaction IS saved, so we close + refresh and surface a non-blocking notice
  // (the details stay editable from the drawer). personal-cfo-4d8.24.2.
  const [addWarning, setAddWarning] = useState<string | null>(null);
  const { addRecurringTransfer } = useRecurringTransfers();
  const { addBill } = useBills();
  const { accounts } = useAccounts();
  const { categories } = useCategories();
  const [adding, setAdding] = useState<"transaction" | "transfer" | null>(null);
  const [selected, setSelected] = useState<TransactionRowDto | null>(null);
  // Whether the row category chip shows its emoji-on-color swatch (personal-cfo-4d8.24.10,
  // AC4). A sticky local view preference (mirrors the page-size localStorage pattern),
  // default on; a restricted webview falls back to on.
  const [showCategoryIcons, setShowCategoryIcons] = useState(() => {
    try {
      return window.localStorage.getItem("pcfo.txnCategoryIcons") !== "0";
    } catch {
      return true;
    }
  });
  const toggleCategoryIcons = () =>
    setShowCategoryIcons((on) => {
      const next = !on;
      try {
        window.localStorage.setItem("pcfo.txnCategoryIcons", next ? "1" : "0");
      } catch {
        // Ignore storage failures (e.g. a locked-down webview) — the toggle still works
        // for the session.
      }
      return next;
    });
  // Multi-select for bulk operations (personal-cfo-j0cg.4): recategorize / tag / review / delete
  // many transactions at once.
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  function toggleSelect(id: string) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }
  // Auto-categorize (personal-cfo-5n4.1): a user-triggered pass that fills uncategorized
  // rows from merchant memory. We surface the count (never a silent re-tag) and a busy state.
  const [autoCatBusy, setAutoCatBusy] = useState(false);
  const [autoCatResult, setAutoCatResult] = useState<string | null>(null);
  async function runAutoCategorize() {
    setAutoCatBusy(true);
    setAutoCatResult(null);
    const result = await autoCategorize();
    setAutoCatBusy(false);
    if ("error" in result) {
      setAutoCatResult(describeIpcError(result.error));
      return;
    }
    setAutoCatResult(
      result.count === 0
        ? "No new transactions to categorize."
        : `Categorized ${result.count} transaction${result.count === 1 ? "" : "s"}.`,
    );
  }
  // Search / facet filters / sort, evaluated server-side with real pagination
  // (personal-cfo-3fdd.1): the query is keyed on (filters, sort, page, pageSize)
  // and spans ALL history, not a 200-row window. Default 10, sticky page size
  // (personal-cfo-4d8.12).
  const [rawFilters, setRawFilters] = useState<TransactionFilters>(() =>
    accountScope === undefined
      ? EMPTY_FILTERS
      : { ...EMPTY_FILTERS, accountIds: accountScope },
  );
  // The scope is re-applied on EVERY read rather than only on change, so no code path —
  // a filter reset, a stale state update, a future control — can widen the list past it.
  // Deriving it beats remembering to preserve it (ADR 0057 §3).
  //
  // MEMOIZED on the scope's VALUE, not its identity, and that is load-bearing rather than
  // an optimization: `filters` is a dependency of the selection-reset effect below. An
  // object rebuilt every render made that effect fire every render, which set state,
  // which re-rendered — an infinite loop that starves the event loop, so even test
  // timeouts stop firing. Keying on the joined ids also absorbs callers that rebuild the
  // array each render, which the Debt page does.
  const scopeKey = accountScope === undefined ? null : accountScope.join(",");
  const filters = useMemo(
    () =>
      scopeKey === null
        ? rawFilters
        : {
            ...rawFilters,
            accountIds: scopeKey === "" ? [] : scopeKey.split(","),
          },
    [rawFilters, scopeKey],
  );
  const setFilters = setRawFilters;
  const [sort, setSort] = useState<TransactionSort>("newest");
  // The spend chart's drill path (ADR 0052 §2). `filters.categoryId` is the single source
  // of truth for WHICH level both views are on; the trail only remembers the labels and
  // the way back, because a level's rows carry their CHILDREN's parent, not their own —
  // going up needs the ancestor the user came through.
  const [spendTrail, setSpendTrail] = useState<{ id: string; name: string }[]>([]);
  const trailTip = spendTrail[spendTrail.length - 1]?.id ?? "";
  // The filter bar can set a category the chart never drilled through. Rebuilding the
  // trail from it — rather than clearing it — is what keeps the two views agreeing: a
  // cleared trail would leave the chart showing every root while the list showed one
  // category, which is precisely the disagreement ADR 0052 §2 exists to prevent.
  useEffect(() => {
    if (filters.categoryId === trailTip) return;
    if (filters.categoryId === "" || filters.categoryId === "uncategorized") {
      setSpendTrail([]);
      return;
    }
    const picked = (categories ?? []).find((c) => c.id === filters.categoryId);
    setSpendTrail(picked ? [{ id: picked.id, name: picked.name }] : []);
  }, [filters.categoryId, trailTip, categories]);
  const {
    rows,
    total,
    error,
    pagination: transactionPages,
  } = usePagedTransactions(filters, sort, "pcfo.pageSize.transactions");
  // Whether anything narrows the list — distinguishes "vault has no transactions"
  // from "the filters matched nothing" now that only the filtered total is known.
  const filtersActive =
    filters.query.trim() !== "" || activeFilterCount(filters) > 0;

  // Clear the bulk selection when the page or the visible set changes — the selected rows may
  // leave the view, so keeping them selected would act on transactions the user can no longer
  // see (personal-cfo-j0cg.4 review).
  const currentPage = transactionPages.page;
  const currentPageSize = transactionPages.pageSize;
  useEffect(() => {
    // Only the standalone (uncontrolled) selection resets on a page/filter change; the
    // hub's unified selection persists by id across changes (personal-cfo-4d8.24.8, AC-6).
    if (!selection) setSelectedIds(new Set());
  }, [currentPage, currentPageSize, filters, sort, selection]);

  // When controlled by the hub, publish this page's rows so the shared selection can
  // resolve + dedupe them against the inbox (personal-cfo-4d8.24.8). Keyed on a string of
  // id + tag_ids (not the array ref): a plain array dep would loop while loading (`rows` is
  // a fresh `[]` each render); the string is stable across renders yet changes when a row's
  // tags change on refetch, so the shared selection re-registers the FRESH DTO (the bulk-tag
  // merge must not read stale tag_ids).
  const registerVisible = selection?.setVisibleRows;
  const activityRowKey = rows
    .map((t) => `${t.transaction_id}:${t.tag_ids.join("|")}`)
    .join(",");
  useEffect(() => {
    registerVisible?.("activity", rows);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- activityRowKey captures the row set + tags
  }, [registerVisible, activityRowKey]);

  const hasAccounts = accounts !== null && accounts.length > 0;
  // A transfer moves money between two accounts across roles (personal-cfo-j0cg.3): cash↔cash, a
  // debt payment, an investment contribution, or a withdrawal. It needs a source to move money out
  // of (cash or investment) plus a second transfer account to move it to.
  const transferAccounts = (accounts ?? []).filter(
    (account) => account.active && TRANSFER_ROLES.has(account.cashflow_role),
  );
  const canTransfer =
    transferAccounts.length >= 2 &&
    transferAccounts.some((a) => SOURCE_ROLES.includes(a.cashflow_role));
  const {
    options: categoryOptions,
    groups: categoryGroups,
    meta: categoryMeta,
    label: categoryPathLabel,
  } = categoryLabels(categories ?? []);

  // The active scope, described once (personal-cfo-3tbn). The drill is one of these chips
  // rather than a separate breadcrumb: the chart's drill and the bar's category facet are
  // the SAME filters.categoryId, so showing them in two places asks the reader to
  // reconcile one piece of state with itself.
  const chips = scopeChips(filters, setFilters, {
    accounts: accounts ?? [],
    tags: tags ?? [],
    // The full path disambiguates two categories that share a leaf name — the strip has to
    // say WHICH "Maintenance" is filtering the list.
    categoryLabel: (id: string) => categoryPathLabel(id) ?? "a category",
    drilled: spendTrail.length > 0,
  });
  // Outflow and inflow for the rows on screen. Off the SAME set the list shows, so the
  // strip's figures and the list are checkable against each other.
  const outflowMinor = rows.reduce(
    (sum, r) => (r.amount.minor_units < 0 ? sum + r.amount.minor_units : sum),
    0,
  );
  const inflowMinor = rows.reduce(
    (sum, r) => (r.amount.minor_units > 0 ? sum + r.amount.minor_units : sum),
    0,
  );
  const scopeCurrency = rows[0]?.amount.currency ?? "USD";

  // The table's column definitions (personal-cfo-4d8.27.4.2). Declaring them here rather
  // than inlining JSX rows means the header, the body cells and the expander's span all
  // derive from ONE list — adding a column cannot leave them out of step.
  const transactionColumns: DataTableColumn<TransactionRowDto>[] = [
    {
      key: "select",
      header: "",
      width: "min",
      cell: (txn) => (
        <input
          type="checkbox"
          className="size-4 shrink-0 accent-primary"
          checked={
            selection
              ? selection.isSelected(txn.transaction_id)
              : selectedIds.has(txn.transaction_id)
          }
          onChange={() =>
            selection ? selection.toggle(txn) : toggleSelect(txn.transaction_id)
          }
          aria-label={`Select ${txn.memo ?? txn.counterparty ?? txn.note ?? txn.account_name}`}
        />
      ),
    },
    {
      key: "activity",
      header: "Activity",
      className: "min-w-0 align-top",
      cell: (txn) => (
        <div className="flex items-center gap-3">
          {/* Unreviewed indicator (personal-cfo-4d8.7): a dot until reviewed; the space
              is reserved either way so rows stay aligned. */}
          <span
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              txn.reviewed ? "bg-transparent" : "bg-info",
            )}
            title={txn.reviewed ? undefined : "Unreviewed"}
            aria-hidden
          />
          <button
            type="button"
            onClick={() => setSelected(txn)}
            className="flex min-w-0 flex-1 flex-col items-start text-left"
          >
            <span className="max-w-full truncate font-medium">
              {/* The account is its OWN column, so falling back to it here would print
                  the same name twice on one row. */}
              {txn.memo ?? txn.counterparty ?? txn.note ?? "—"}
            </span>
            {(txn.split_count > 0 || isAutoCategorized(txn.category_source)) && (
              <span className="mt-1 flex flex-wrap items-center gap-1">
                <CategoryProvenanceBadge
                  source={txn.category_source}
                  confidenceBps={txn.category_confidence_bps}
                />
                {txn.split_count > 0 && (
                  <span className="inline-flex items-center gap-1 rounded-full border bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
                    <Split className="size-3" aria-hidden />
                    Split into {txn.split_count}
                  </span>
                )}
              </span>
            )}
          </button>
        </div>
      ),
    },
    {
      key: "date",
      header: "Date",
      width: "min",
      className: "whitespace-nowrap align-top text-muted-foreground",
      cell: (txn) => formatDate(txn.occurred_at),
    },
    {
      key: "accounts",
      header: "Accounts",
      className: "align-top text-muted-foreground",
      cell: (txn) =>
        txn.counter_account_name ? (
          <span className="flex min-w-0 items-center gap-1 whitespace-nowrap">
            <span className="truncate">
              {txn.amount.minor_units < 0 ? txn.account_name : txn.counter_account_name}
            </span>
            <ArrowRight aria-hidden className="size-3 shrink-0" />
            <span className="truncate">
              {txn.amount.minor_units < 0 ? txn.counter_account_name : txn.account_name}
            </span>
          </span>
        ) : (
          <span className="truncate">{txn.account_name}</span>
        ),
    },
    {
      key: "category",
      header: "Category",
      width: "min",
      className: "align-top",
      cell: (txn) => (
        <CategoryCell
          categoryId={txn.category_id}
          groups={categoryGroups}
          meta={categoryMeta(txn.category_id)}
          showIcons={showCategoryIcons}
          onSelect={(categoryId) => void recategorize(txn.transaction_id, categoryId)}
        />
      ),
    },
    {
      key: "tags",
      header: "Tags",
      className: "align-top",
      cell: (txn) => (
        <span className="flex flex-wrap items-center gap-1">
          {txn.tag_ids.map((id) => {
            const tag = tags?.find((candidate) => candidate.id === id);
            return tag ? <TagChip key={id} tag={tag} /> : null;
          })}
        </span>
      ),
    },
    {
      key: "amount",
      header: "Amount",
      align: "right",
      width: "min",
      className: "align-top font-medium tabular-nums",
      cell: (txn) => (
        <span className={signedAmountClass(txn.amount)}>
          {formatSignedMoney(txn.amount)}
        </span>
      ),
    },
    {
      key: "balance",
      header: "Balance after",
      align: "right",
      width: "min",
      className: "tabular-nums",
      cell: (txn) =>
        txn.balance_after_minor === null ? (
          <span className="text-muted-foreground">—</span>
        ) : (
          formatMoney({
            minor_units: txn.balance_after_minor,
            currency: txn.amount.currency,
          })
        ),
    },
    {
      key: "expand",
      header: "",
      width: "min",
      className: "align-top",
      cell: (txn) =>
        txn.split_count > 0 ? (
          <button
            type="button"
            onClick={() => toggleSplit(txn.transaction_id)}
            aria-label={
              expandedSplits.has(txn.transaction_id)
                ? "Hide split lines"
                : "Show split lines"
            }
            aria-expanded={expandedSplits.has(txn.transaction_id)}
            aria-controls={`txn-splits-${txn.transaction_id}`}
            className="rounded-md p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <ChevronDown
              className={cn(
                "size-4 transition-transform",
                expandedSplits.has(txn.transaction_id) && "rotate-180",
              )}
              aria-hidden
            />
          </button>
        ) : null,
    },
  ];

  // Which split transactions are expanded inline to show their lines (4d8.19).
  const [expandedSplits, setExpandedSplits] = useState<Set<string>>(new Set());
  function toggleSplit(transactionId: string) {
    setExpandedSplits((prev) => {
      const next = new Set(prev);
      if (next.has(transactionId)) next.delete(transactionId);
      else next.add(transactionId);
      return next;
    });
  }

  return (
    // Wider than the app's usual reading column: this surface is a real columnar table
    // now (personal-cfo-4d8.27.8.3), and a 2xl cap would leave it horizontally scrolling
    // at all times — a table you must scroll sideways to read is worse than the stacked
    // rows it replaced.
    <div
      className={
        embedded
          ? "flex flex-col gap-4"
          : "mx-auto flex w-full max-w-6xl flex-col gap-4"
      }
    >
      <div className="flex items-center justify-between">
        {embedded ? (
          <span />
        ) : (
          <h2 className="text-lg font-semibold tracking-tight">Transactions</h2>
        )}
        {!adding && hasAccounts && (
          <div className="flex gap-2">
            {total !== undefined && (total > 0 || filtersActive) && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => void runAutoCategorize()}
                disabled={autoCatBusy}
              >
                {autoCatBusy ? (
                  <Loader2 className="animate-spin" aria-hidden />
                ) : (
                  <Sparkles aria-hidden />
                )}
                Auto-categorize
              </Button>
            )}
            {total !== undefined && total > 0 && <ExportCsvButton />}
            {canTransfer && (
              <Button
                size="sm"
                variant="outline"
                onClick={() => setAdding("transfer")}
              >
                <ArrowLeftRight aria-hidden />
                Add transfer
              </Button>
            )}
            <Button size="sm" onClick={() => setAdding("transaction")}>
              <Plus aria-hidden />
              Add transaction
            </Button>
          </div>
        )}
      </div>

      {adding === "transaction" && accounts && (
        <AddTransactionForm
          accounts={accounts}
          categories={categories}
          tags={tags}
          onCreateTag={createTag}
          onCancel={() => setAdding(null)}
          onCreate={async (input, meta, files) => {
            setAddWarning(null);
            const outcome = await addTransactionWithMeta(
              input,
              meta,
              { setTags, setNote },
              files,
            );
            if (outcome.status === "record-failed") {
              // Nothing persisted — keep the form open with the inline error.
              return describeIpcError(outcome.error);
            }
            // The transaction is saved — close + let the list refetch.
            setAdding(null);
            if (outcome.status === "partial") {
              setAddWarning(
                "Transaction added, but some details (category, tags, note, or attachments) could not be saved. Open it to try again.",
              );
            }
            return null;
          }}
          onCreateRecurring={async (input) => {
            // Instead of a one-off record, promote to a recurring bill via the shared
            // create path (personal-cfo-4d8.24.2.2).
            const { error: failure } = await addBill(input);
            if (!failure) setAdding(null);
            return failure ? describeIpcError(failure) : null;
          }}
        />
      )}

      {adding === "transfer" && (
        <AddTransferForm
          accounts={transferAccounts}
          onCancel={() => setAdding(null)}
          onCreate={async (input) => {
            const failure = await transfer(input);
            if (!failure) setAdding(null);
            return failure ? describeIpcError(failure) : null;
          }}
          onCreateRecurring={async (input) => {
            const failure = await addRecurringTransfer(input);
            if (!failure) setAdding(null);
            return failure ? describeIpcError(failure) : null;
          }}
        />
      )}

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}

      {autoCatResult && (
        <p role="status" className="text-sm text-muted-foreground">
          {autoCatResult}
        </p>
      )}

      {addWarning && (
        <p role="status" className="text-sm text-warning">
          {addWarning}
        </p>
      )}

      {accounts !== null && !hasAccounts ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-2 py-10 text-center">
            <ArrowLeftRight className="size-8 text-muted-foreground" aria-hidden />
            <p className="font-medium">Add an account first</p>
            <p className="text-sm text-muted-foreground">
              Transactions move money against an account. Create one on the
              Accounts tab.
            </p>
          </CardContent>
        </Card>
      ) : total === 0 && !filtersActive ? (
        // No filters and a zero total = the vault truly has no transactions.
        adding === null && (
          <Card>
            <CardContent className="flex flex-col items-center gap-2 py-10 text-center">
              <ArrowLeftRight
                className="size-8 text-muted-foreground"
                aria-hidden
              />
              <p className="font-medium">No transactions yet</p>
              <p className="max-w-md text-sm text-muted-foreground">
                Import a CSV or OFX file from your bank and every row lands here, ready to
                categorize.
              </p>
              {/* The import path is the one that actually fills a vault; adding by hand is
                  the fallback, so it reads as the secondary action rather than the only
                  one. */}
              <Button size="sm" variant="outline" onClick={() => setAdding("transaction")}>
                Add one manually
              </Button>
            </CardContent>
          </Card>
        )
      ) : (
        <>
          <TransactionFilterBar
            filters={filters}
            onFiltersChange={setFilters}
            accountScopeLocked={accountScope !== undefined}
            sort={sort}
            onSortChange={setSort}
            accounts={accounts ?? []}
            categoryOptions={categoryOptions}
            tags={tags ?? []}
          />
          {/* Spend analytics live WITH the transactions they describe (ADR 0052 §1), and
              read the same filter state as the list below (§2) — so a bar the user clicks
              lists exactly the rows it counted. */}
          {/* The one place the scope is stated (ADR 0052 §2, personal-cfo-3tbn). Sticky:
              categorizing is a long scroll, and scope that scrolls away is how a row gets
              mis-filed against a filter the reader forgot was on. */}
          <ScopeStrip
            chips={chips}
            shown={rows.length}
            total={total ?? rows.length}
            outflowMinor={outflowMinor}
            inflowMinor={inflowMinor}
            currency={scopeCurrency}
            onClearAll={() => setFilters(CLEARED_SCOPE)}
          />

          <SpendByCategoryCard
            filters={filters}
            trail={spendTrail}
            onDrill={(id, name) => {
              setSpendTrail((prev) => [...prev, { id, name }]);
              setFilters((prev) => ({ ...prev, categoryId: id }));
            }}
            onBack={() => {
              const next = spendTrail.slice(0, -1);
              setSpendTrail(next);
              setFilters((prev) => ({
                ...prev,
                categoryId: next[next.length - 1]?.id ?? "",
              }));
            }}
          />
          {/* View preference (personal-cfo-4d8.24.10, AC4): show the category icon legend
              (emoji-on-color swatch) on each row, or hide it for a plainer name-only list. */}
          <label className="flex items-center gap-1.5 self-end text-xs text-muted-foreground">
            <input
              type="checkbox"
              className="size-3.5 accent-primary"
              checked={showCategoryIcons}
              onChange={toggleCategoryIcons}
            />
            Show category icons
          </label>
          {/* Standalone bulk bar (uncontrolled). When the hub controls the selection
              (personal-cfo-4d8.24.8) it renders the single unified bar instead. */}
          {!selection && selectedIds.size > 0 && (
            <BulkTransactionActions
              selected={rows.filter((t) => selectedIds.has(t.transaction_id))}
              total={rows.length}
              categories={categories}
              tags={tags}
              recategorize={recategorize}
              setReviewed={setReviewed}
              deleteTransaction={deleteTransaction}
              setTags={setTags}
              createTag={createTag}
              onSelectAll={() =>
                setSelectedIds(new Set(rows.map((t) => t.transaction_id)))
              }
              onSelectionChange={(ids) => setSelectedIds(new Set(ids))}
            />
          )}
          <Card>
            <CardContent className="p-0">
              {/* A real columnar table on the shared DataTable primitive
                  (personal-cfo-4d8.27.4.2): the date, the accounts the money moved
                  between, the category and the tags each get their own column instead of
                  being stacked as subtitles, so the eye can scan down one kind of thing
                  at a time. The primitive owns the header and the split row's colSpan.
                  The header deliberately has no select-all checkbox — bulk select-all
                  lives in the bulk bar, where it can say how many it is acting on.
                  Sorting stays in the filter bar: it is server-side, over the whole
                  result set, not just the page in view. */}
              <DataTable
                columns={transactionColumns}
                rows={rows}
                rowKey={(txn) => txn.transaction_id}
                minWidth="min-w-[900px]"
                // The table owns its own loading and empty treatments (ADR 0053 §1) —
                // this screen used to hand-roll a centred spinner and a bespoke card,
                // which is the state re-invention the primitive exists to end.
                // A failed read used to leave the table skeletonised forever, because
                // `total` stays undefined on error — so the screen showed an error message
                // AND an endless "loading" list, which reads as "still trying".
                status={error ? "error" : total === undefined ? "loading" : "ready"}
                error={
                  error
                    ? "Couldn’t read this page of transactions. Your data is fine — this read failed."
                    : null
                }
                empty={{
                  icon: Search,
                  title: "No transactions match these filters",
                  description:
                    "Widen the range, drop a facet, or clear everything to see them again.",
                }}
                rowProps={(txn) => ({
                  "data-state": (
                    selection
                      ? selection.isSelected(txn.transaction_id)
                      : selectedIds.has(txn.transaction_id)
                  )
                    ? "selected"
                    : undefined,
                })}
                expandedContent={(txn) =>
                  txn.split_count > 0 && expandedSplits.has(txn.transaction_id) ? (
                    <div id={`txn-splits-${txn.transaction_id}`} className="bg-muted/20">
                      <SplitLinesDetail
                        transactionId={txn.transaction_id}
                        categories={categories}
                        tags={tags}
                      />
                    </div>
                  ) : null
                }
              />
            {transactionPages.total > PAGE_SIZE_OPTIONS[0] && (
              <PaginationControls
                pagination={transactionPages}
                noun="transactions"
                className="border-t px-5 py-3"
              />
            )}
            </CardContent>
          </Card>
        </>
      )}

      {selected && (
        <TransactionDetailDrawer
          transaction={selected}
          categories={categories}
          onRecategorize={recategorize}
          onDelete={deleteTransaction}
          onSetReviewed={setReviewed}
          tags={tags}
          onCreateTag={createTag}
          onSetTags={setTags}
          onSetNote={setNote}
          onClose={() => setSelected(null)}
        />
      )}
    </div>
  );
}

/// Whether a category was assigned by something other than the user (ADR 0030,
/// personal-cfo-5n4.1). `user` is the expected default and gets no badge; `null` is
/// uncategorized. Anything else (`rule` / `model` / `import_alias`) is auto-categorized.
function isAutoCategorized(source: string | null): boolean {
  return source !== null && source !== "user";
}

/// The provenance badge on an auto-categorized row (personal-cfo-5n4.1): makes a
/// learned/auto tag visible and trustable by showing it wasn't the user, plus the
/// categorizer's confidence. User-assigned and uncategorized rows render nothing.
function CategoryProvenanceBadge({
  source,
  confidenceBps,
}: {
  source: string | null;
  confidenceBps: number | null;
}) {
  if (!isAutoCategorized(source)) return null;
  const pct = confidenceBps === null ? null : Math.round(confidenceBps / 100);
  const label = source === "import_alias" ? "Import" : "Auto";
  const title =
    pct === null
      ? "Categorized automatically, not by you"
      : `Categorized automatically (${pct}% confidence), not by you`;
  return (
    <span
      title={title}
      className="inline-flex items-center gap-1 rounded-full border border-info/40 bg-info/10 px-2 py-0.5 text-[11px] font-medium text-info"
    >
      <Sparkles className="size-3" aria-hidden />
      {label}
      {pct !== null && ` · ${pct}%`}
    </span>
  );
}

/// Inline category control on a transaction row (personal-cfo-4d8.13): a compact chip
/// reading "Uncategorized" (dashed) until a category is set, then the label. Changing
/// it calls `recategorize` via the existing IPC. A native select keeps it dep-free and
/// accessible; it stops the row's drawer-open click from firing.
function CategoryCell({
  categoryId,
  groups,
  meta,
  showIcons,
  onSelect,
}: {
  categoryId: string | null;
  // Grouped taxonomy (personal-cfo-4d8.24.9): the collapsed chip shows the selected
  // category's OWN name (leaf-only), while the open dropdown groups same-named leaves
  // under their parent header so they stay disambiguated.
  groups: { id: string; name: string; children: { id: string; name: string }[] }[];
  // The selected category's display metadata (icon/color), for the swatch chip
  // (personal-cfo-4d8.24.10). `null` when uncategorized/unknown.
  meta: CategoryMeta | null;
  // Whether to show the emoji-on-color swatch (the AC4 icon-legend toggle).
  showIcons: boolean;
  onSelect: (categoryId: string | null) => void;
}) {
  const uncategorized = categoryId === null;
  // A swatch is worth drawing only when icons are on and the category has an icon or a
  // color to show.
  const swatch = showIcons && !uncategorized && (meta?.icon || meta?.color) ? meta : null;
  return (
    <div className="relative shrink-0">
      {swatch && (
        <span
          aria-hidden
          className="pointer-events-none absolute top-1/2 left-1 z-10 flex size-5 -translate-y-1/2 items-center justify-center rounded-full border text-[11px] leading-none"
          style={{ backgroundColor: swatch.color ?? "transparent" }}
        >
          {swatch.icon ?? ""}
        </span>
      )}
      <select
        aria-label="Category"
        value={categoryId ?? ""}
        onChange={(event) => onSelect(event.target.value || null)}
        className={cn(
          "max-w-[9rem] cursor-pointer appearance-none truncate rounded-full border py-0.5 pr-6 text-[11px] font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring",
          swatch ? "pl-7" : "pl-2.5",
          uncategorized
            ? "border-dashed border-warning/50 bg-warning/5 text-warning"
            : "border-transparent bg-secondary text-secondary-foreground",
        )}
      >
        <option value="">Uncategorized</option>
        {groups.map((group) =>
          group.children.length > 0 ? (
            <optgroup key={group.id} label={group.name}>
              {/* The parent group stays selectable (a transaction assigned to it must
                  still render); its leaves nest below, shown leaf-only. */}
              <option value={group.id}>{group.name}</option>
              {group.children.map((child) => (
                <option key={child.id} value={child.id}>
                  {child.name}
                </option>
              ))}
            </optgroup>
          ) : (
            <option key={group.id} value={group.id}>
              {group.name}
            </option>
          ),
        )}
      </select>
      <ChevronDown
        className="pointer-events-none absolute top-1/2 right-1.5 size-3 -translate-y-1/2 text-muted-foreground"
        aria-hidden
      />
    </div>
  );
}

/// The split lines shown inline when a split row is expanded (personal-cfo-4d8.19):
/// per line, the category + amount, plus its tags + note. Lazily fetched (the hook only
/// runs once this is mounted on expand).
function SplitLinesDetail({
  transactionId,
  categories,
  tags,
}: {
  transactionId: string;
  categories: CategoryDto[] | null;
  tags: TagViewDto[] | null;
}) {
  const splitsQuery = useTransactionSplits(transactionId);
  // Leaf-only here (personal-cfo-4d8.24.9): the split sits under its transaction row, so
  // the parent group is redundant context.
  const { leafLabel } = categoryLabels(categories ?? []);
  const lines = splitsQuery.data ?? [];

  if (splitsQuery.isLoading) {
    return (
      <div className="py-2 pr-5 pl-14 text-sm text-muted-foreground">
        Loading split…
      </div>
    );
  }
  return (
    <ul className="flex flex-col gap-2 py-2 pr-5 pl-14">
      {lines.map((line) => (
        <li key={line.id} className="flex flex-col gap-0.5">
          <div className="flex items-center justify-between gap-3 text-sm">
            <span className="truncate text-muted-foreground">
              {leafLabel(line.category_id) ?? "Uncategorized"}
            </span>
            <span
              className={`shrink-0 tabular-nums ${signedAmountClass(line.amount)}`}
            >
              {formatSignedMoney(line.amount)}
            </span>
          </div>
          {line.tag_ids.length > 0 && (
            <span className="flex flex-wrap gap-1">
              {line.tag_ids.map((id) => {
                const tag = tags?.find((candidate) => candidate.id === id);
                return tag ? <TagChip key={id} tag={tag} /> : null;
              })}
            </span>
          )}
          {line.note && (
            <span className="text-xs text-muted-foreground">{line.note}</span>
          )}
        </li>
      ))}
    </ul>
  );
}

// Rust remains authoritative for validation (ADR 0003); this schema only gates
// the form UI and shapes display copy.
const transactionFormSchema = z.object({
  account_id: z.string().min(1, "Choose an account."),
  kind: z.enum(["expense", "income"]),
  amount: z.string().refine((value) => {
    const minor = dollarsToMinorUnits(value);
    return minor !== null && minor > 0;
  }, "Enter an amount above zero."),
  date: z.string().min(1, "Pick a date."),
  // Optional metadata set inline at add time (personal-cfo-4d8.24.2). `categoryId`
  // is "" for uncategorized; note "" clears; tags empty means none.
  categoryId: z.string(),
  tagIds: z.array(z.string()),
  note: z.string(),
  // Recurring-bill promotion (personal-cfo-4d8.24.2.2): only read when the "recurring"
  // toggle is on. `recurringName` is required in that mode (validated on submit); the
  // date doubles as the bill's anchor and the amount as its per-occurrence outflow.
  recurringName: z.string(),
  frequency: z.string(),
  // Parts of a custom interval (ADR 0048); only read when frequency === "custom".
  interval_count: z.string(),
  interval_unit: z.string(),
});
type TransactionFormValues = z.infer<typeof transactionFormSchema>;

/// The metadata the form assembles for the record→attach one-flow (personal-cfo-4d8.24.2).
type TransactionMeta = {
  categoryId: string | null;
  tagIds: string[];
  note: string | null;
};

function AddTransactionForm({
  accounts,
  categories,
  tags,
  onCreateTag,
  onCancel,
  onCreate,
  onCreateRecurring,
}: {
  accounts: AccountViewDto[];
  categories: CategoryDto[] | null;
  tags: TagViewDto[] | null;
  onCreateTag: (name: string) => Promise<string | IpcError>;
  onCancel: () => void;
  onCreate: (
    input: RecordTransactionInput,
    meta: TransactionMeta,
    files: File[],
  ) => Promise<string | null>;
  onCreateRecurring: (input: CreateRecurringBillInput) => Promise<string | null>;
}) {
  const {
    register,
    handleSubmit,
    watch,
    setValue,
    formState: { errors, isSubmitting },
  } = useForm<TransactionFormValues>({
    resolver: zodResolver(transactionFormSchema),
    defaultValues: {
      account_id: accounts[0]?.id ?? "",
      kind: "expense",
      amount: "",
      date: today(),
      categoryId: "",
      tagIds: [],
      note: "",
      interval_count: "",
      interval_unit: "months",
      recurringName: "",
      frequency: "monthly",
    },
  });
  const kind = watch("kind");
  const tagIds = watch("tagIds");
  const [error, setError] = useState<string | null>(null);
  // When on, submit promotes to a recurring bill (name + cadence) via the shared bill
  // create path instead of recording a one-off transaction (personal-cfo-4d8.24.2.2).
  const [recurring, setRecurring] = useState(false);
  // Documents picked at add time (personal-cfo-4d8.24.2.3): staged locally (no txn id
  // yet) and attached after the record. Only for a one-off txn — a bill has no id.
  const [stagedFiles, setStagedFiles] = useState<File[]>([]);
  const filePicker = useRef<HTMLInputElement>(null);
  function onPickFiles(event: ChangeEvent<HTMLInputElement>) {
    const picked = Array.from(event.target.files ?? []);
    event.target.value = ""; // let the user re-pick the same file later (mirrors the drawer)
    if (picked.length > 0) setStagedFiles((prev) => [...prev, ...picked]);
  }
  function removeStaged(index: number) {
    setStagedFiles((prev) => prev.filter((_, i) => i !== index));
  }
  // The metadata (category/tags/note) sits behind a disclosure so the common fast path
  // stays amount + date + account. Values entered then collapsed are still submitted
  // (RHF keeps them in form state).
  const [showDetails, setShowDetails] = useState(false);
  const [tagInput, setTagInput] = useState("");
  const [tagError, setTagError] = useState<string | null>(null);
  const { options: categoryOptions } = categoryLabels(categories ?? []);

  // Add the typed tag: match an existing one by name (case-insensitive), else mint it
  // (mirrors the drawer). The tag isn't transaction-scoped, so it resolves before the
  // transaction exists; the id is staged in form state and attached after the record.
  async function addTagFromInput() {
    const name = tagInput.trim();
    if (!name) return;
    setTagInput("");
    setTagError(null);
    const existing = (tags ?? []).find(
      (tag) => tag.name.toLowerCase() === name.toLowerCase(),
    );
    let tagId = existing?.id ?? null;
    if (tagId === null) {
      const created = await onCreateTag(name);
      if (typeof created !== "string") {
        setTagError(describeIpcError(created));
        return;
      }
      tagId = created;
    }
    if (tagIds.includes(tagId)) return; // already staged — no-op
    setValue("tagIds", [...tagIds, tagId]);
  }

  function removeTag(tagId: string) {
    setValue(
      "tagIds",
      tagIds.filter((id) => id !== tagId),
    );
  }

  const submit = handleSubmit(async (values) => {
    setError(null);
    const selectedAccount =
      accounts.find((account) => account.id === values.account_id) ??
      accounts[0];
    const magnitude = dollarsToMinorUnits(values.amount);
    if (selectedAccount === undefined || magnitude === null) return;

    if (recurring) {
      const name = values.recurringName.trim();
      if (name === "") {
        setError("Enter a name for the recurring bill.");
        return;
      }
      // Resolve the form-only "custom" selection into the wire token (ADR 0048).
      let frequency = values.frequency;
      if (frequency === "custom") {
        const n = Number(values.interval_count);
        const unit = values.interval_unit as IntervalUnit;
        if (!isValidIntervalCount(n, unit)) {
          const max = INTERVAL_UNITS.find((u) => u.value === unit)?.max;
          setError(`Enter a whole number of ${unit} between 1 and ${max}.`);
          return;
        }
        frequency = buildIntervalToken(n, unit);
      }
      const note = values.note.trim();
      // A bill amount is a positive per-occurrence magnitude; the date is its anchor and
      // the account its pay-from. Category/tags/note carry over to the bill.
      const failure = await onCreateRecurring({
        name,
        amount: {
          minor_units: magnitude,
          currency: selectedAccount.balance.currency,
        },
        bill_type: "subscription",
        frequency,
        anchor_date: values.date,
        autopay_account_id: selectedAccount.id,
        autopay: false,
        description: note === "" ? null : note,
        source_merchant_key: null,
        category_id: values.categoryId === "" ? null : values.categoryId,
        tag_ids: values.tagIds,
        idempotency_key: mintIdempotencyKey(),
      });
      if (failure) setError(failure);
      return;
    }

    const signed = values.kind === "expense" ? -magnitude : magnitude;
    const input: RecordTransactionInput = {
      account_id: selectedAccount.id,
      amount: {
        minor_units: signed,
        currency: selectedAccount.balance.currency,
      },
      occurred_at: `${values.date}T00:00:00Z`,
      idempotency_key: mintIdempotencyKey(),
    };
    const meta: TransactionMeta = {
      categoryId: values.categoryId === "" ? null : values.categoryId,
      tagIds: values.tagIds,
      note: values.note.trim() === "" ? null : values.note.trim(),
    };
    const failure = await onCreate(input, meta, stagedFiles);
    if (failure) setError(failure);
  });

  return (
    <Card>
      <CardContent className="pt-6">
        <form onSubmit={submit} className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="txn-account">Account</Label>
            <select
              id="txn-account"
              className={SELECT_CLASS}
              {...register("account_id")}
            >
              {accounts.map((account) => (
                <option key={account.id} value={account.id}>
                  {account.name}
                </option>
              ))}
            </select>
          </div>

          <label className="flex items-center gap-2 self-start text-sm font-medium">
            <input
              type="checkbox"
              className="size-4 accent-primary"
              checked={recurring}
              onChange={(event) => setRecurring(event.target.checked)}
            />
            Set up as a recurring bill
          </label>

          {recurring ? (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="txn-bill-name">Bill name</Label>
              <Input
                id="txn-bill-name"
                {...register("recurringName")}
                placeholder="e.g. Rent, Spotify"
              />
            </div>
          ) : (
            <div className="flex flex-col gap-1.5">
              <Label>Type</Label>
              <div className="grid grid-cols-2 gap-2">
                <Button
                  type="button"
                  variant={kind === "expense" ? "default" : "outline"}
                  onClick={() => setValue("kind", "expense")}
                >
                  Expense
                </Button>
                <Button
                  type="button"
                  variant={kind === "income" ? "default" : "outline"}
                  onClick={() => setValue("kind", "income")}
                >
                  Income
                </Button>
              </div>
            </div>
          )}

          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="txn-amount">Amount</Label>
              <Input
                id="txn-amount"
                inputMode="decimal"
                autoFocus
                {...register("amount")}
                placeholder="0.00"
                aria-invalid={!!errors.amount}
              />
              {errors.amount && (
                <p className="text-xs text-loss">{errors.amount.message}</p>
              )}
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="txn-date">
                {recurring ? "First due date" : "Date"}
              </Label>
              <Input id="txn-date" type="date" {...register("date")} />
            </div>
          </div>

          {recurring && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="txn-frequency">Frequency</Label>
              <select
                id="txn-frequency"
                className={SELECT_CLASS}
                {...register("frequency")}
              >
                {PAY_FREQUENCIES.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              {watch("frequency") === "custom" && (
                <div className="flex items-center gap-1.5">
                  <span className="text-sm text-muted-foreground">Every</span>
                  <Input
                    aria-label="Interval count"
                    inputMode="numeric"
                    className="h-9 w-16"
                    {...register("interval_count")}
                  />
                  <select
                    aria-label="Interval unit"
                    className={SELECT_CLASS + " w-auto"}
                    {...register("interval_unit")}
                  >
                    {INTERVAL_UNITS.map((unit) => (
                      <option key={unit.value} value={unit.value}>
                        {unit.label}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </div>
          )}

          {/* Category / tags / note, set right here at add time (personal-cfo-4d8.24.2)
              instead of only after in the drawer. Behind a disclosure so the fast path
              (amount + date) stays uncluttered; collapsed values are still submitted. */}
          <div className="flex flex-col gap-3 border-t pt-3">
            <button
              type="button"
              onClick={() => setShowDetails((open) => !open)}
              aria-expanded={showDetails}
              className="flex items-center gap-1.5 self-start text-sm font-medium text-muted-foreground hover:text-foreground"
            >
              <ChevronDown
                className={cn(
                  "size-4 transition-transform",
                  showDetails && "rotate-180",
                )}
                aria-hidden
              />
              Add details
              {(watch("categoryId") !== "" ||
                tagIds.length > 0 ||
                watch("note").trim() !== "" ||
                stagedFiles.length > 0) && (
                <span className="rounded-full bg-secondary px-2 py-0.5 text-xs text-secondary-foreground">
                  Set
                </span>
              )}
            </button>

            {showDetails && (
              <div className="flex flex-col gap-3">
                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="txn-category">Category</Label>
                  <select
                    id="txn-category"
                    className={SELECT_CLASS}
                    {...register("categoryId")}
                  >
                    <option value="">Uncategorized</option>
                    {categoryOptions.map((option) => (
                      <option key={option.id} value={option.id}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </div>

                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="txn-tags">Tags</Label>
                  {tagIds.length > 0 && (
                    <div className="flex flex-wrap gap-1.5">
                      {tagIds.map((id) => {
                        const tag = tags?.find((candidate) => candidate.id === id);
                        return tag ? (
                          <TagChip
                            key={id}
                            tag={tag}
                            onRemove={() => removeTag(id)}
                          />
                        ) : null;
                      })}
                    </div>
                  )}
                  <input
                    id="txn-tags"
                    list="add-txn-tag-options"
                    value={tagInput}
                    onChange={(event) => setTagInput(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") {
                        event.preventDefault();
                        void addTagFromInput();
                      }
                    }}
                    placeholder="Add a tag…"
                    aria-label="Add a tag"
                    className={SELECT_CLASS}
                  />
                  <datalist id="add-txn-tag-options">
                    {(tags ?? [])
                      .filter(
                        (tag) => !tag.archived && !tagIds.includes(tag.id),
                      )
                      .map((tag) => (
                        <option key={tag.id} value={tag.name} />
                      ))}
                  </datalist>
                  {tagError && (
                    <p role="alert" className="text-sm text-loss">
                      {tagError}
                    </p>
                  )}
                </div>

                <div className="flex flex-col gap-1.5">
                  <Label htmlFor="txn-note">Note</Label>
                  <textarea
                    id="txn-note"
                    {...register("note")}
                    maxLength={4096}
                    rows={3}
                    placeholder="Add a note…"
                    className={cn(SELECT_CLASS, "h-auto resize-y")}
                  />
                </div>

                {/* Attach documents at add time (personal-cfo-4d8.24.2.3): staged locally,
                    copied into the encrypted vault (ADR 0023) after the record. Only for a
                    one-off txn — a recurring bill has no transaction id to attach to. */}
                {!recurring && (
                  <div className="flex flex-col gap-1.5">
                    <Label>Attachments</Label>
                    {stagedFiles.length > 0 && (
                      <ul className="flex flex-col gap-1">
                        {stagedFiles.map((file, index) => (
                          <li
                            key={`${file.name}-${index}`}
                            className="flex items-center justify-between rounded-md border px-3 py-2"
                          >
                            <div className="flex min-w-0 items-center gap-2">
                              <FileText
                                className="size-4 shrink-0 text-muted-foreground"
                                aria-hidden
                              />
                              <span className="truncate text-sm">{file.name}</span>
                              <span className="shrink-0 text-xs text-muted-foreground">
                                {formatBytes(file.size)}
                              </span>
                            </div>
                            <button
                              type="button"
                              onClick={() => removeStaged(index)}
                              aria-label={`Remove ${file.name}`}
                              className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-loss"
                            >
                              <Trash2 className="size-4" aria-hidden />
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                    <input
                      ref={filePicker}
                      type="file"
                      multiple
                      className="sr-only"
                      aria-label="Attach documents"
                      onChange={onPickFiles}
                    />
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="self-start"
                      onClick={() => filePicker.current?.click()}
                    >
                      <Paperclip aria-hidden />
                      Attach documents
                    </Button>
                  </div>
                )}
              </div>
            )}
          </div>

          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}

          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting}>
              {isSubmitting
                ? recurring
                  ? "Creating…"
                  : "Adding…"
                : recurring
                  ? "Create recurring bill"
                  : "Add transaction"}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}

// Rust remains authoritative (ADR 0003); this schema gates the form UI only.
// The pay-frequency options: the classic tokens plus the custom-interval escape
// hatch (ADR 0048) — "custom" is form-only, resolved to an every_<n>_<unit> token on
// submit. Shared by the recurring-transfer and recurring-bill (4d8.24.2.2) forms.
const PAY_FREQUENCIES: { value: string; label: string }[] = [
  ...CLASSIC_FREQUENCIES,
  { value: "custom", label: "Custom interval…" },
];

const transferFormSchema = z
  .object({
    source_account_id: z.string().min(1, "Choose an account."),
    dest_account_id: z.string().min(1, "Choose an account."),
    amount: z.string().refine((value) => {
      const minor = dollarsToMinorUnits(value);
      return minor !== null && minor > 0;
    }, "Enter an amount above zero."),
    date: z.string().min(1, "Pick a date."),
    frequency: z.string(),
    // Parts of a custom interval (ADR 0048); only read when frequency === "custom".
    interval_count: z.string(),
    interval_unit: z.string(),
  })
  .refine((values) => values.source_account_id !== values.dest_account_id, {
    message: "Pick two different accounts.",
    path: ["dest_account_id"],
  });
type TransferFormValues = z.infer<typeof transferFormSchema>;

// Move cash between two of the user's liquid accounts (personal-cfo-npoe), as a
// one-off or on a recurring schedule. The amount's currency follows the source
// account; Rust validates the accounts match. Exported so the Recurring tab offers
// "Add recurring transfer" without a second form (personal-cfo-4d8.25.12): that
// entry point locks the recurring mode (`lockRecurring`) and omits `onCreate`.
export function AddTransferForm({
  accounts,
  onCancel,
  onCreate,
  onCreateRecurring,
  defaultRecurring = false,
  lockRecurring = false,
}: {
  accounts: AccountViewDto[];
  onCancel: () => void;
  onCreate?: (input: RecordTransferInput) => Promise<string | null>;
  onCreateRecurring: (
    input: CreateRecurringTransferInput,
  ) => Promise<string | null>;
  defaultRecurring?: boolean;
  lockRecurring?: boolean;
}) {
  const firstSource = accounts.find((a) => SOURCE_ROLES.includes(a.cashflow_role));
  const firstDest = accounts.find(
    (a) => a.id !== firstSource?.id && a.balance.currency === firstSource?.balance.currency,
  );
  const {
    register,
    handleSubmit,
    watch,
    setValue,
    formState: { errors, isSubmitting },
  } = useForm<TransferFormValues>({
    resolver: zodResolver(transferFormSchema),
    defaultValues: {
      source_account_id: firstSource?.id ?? "",
      dest_account_id: firstDest?.id ?? "",
      amount: "",
      date: today(),
      frequency: "monthly",
      interval_count: "",
      interval_unit: "months",
    },
  });
  const [error, setError] = useState<string | null>(null);
  const [recurring, setRecurring] = useState(defaultRecurring);
  // A recurring transfer must be funded from cash (CreateRecurringTransfer requires a liquid
  // source), so it's only offered when at least one cash account exists — otherwise toggling to
  // Recurring would strip every source option and dead-end the form.
  const hasLiquidSource = accounts.some((a) => a.cashflow_role === "liquid_cash");
  const sourceId = watch("source_account_id");
  const destId = watch("dest_account_id");
  const source = accounts.find((a) => a.id === sourceId);
  const dest = accounts.find((a) => a.id === destId);

  // Source: cash always; an investment only for a one-off (a recurring transfer must be funded from
  // cash — CreateRecurringTransfer requires a liquid source, 9h0.1).
  const sourceAccounts = accounts.filter(
    (a) =>
      a.cashflow_role === "liquid_cash" ||
      (!recurring && a.cashflow_role === "investment_asset"),
  );
  // Destination: same currency, not the source, and a role the backend accepts given the source and
  // mode. An investment source withdraws to cash only (j0cg.2); a cash source can go to cash, an
  // investment (a contribution, 9h0.1), and — one-off only — a liability (a debt payment, r7sb).
  const destAccounts = accounts.filter((a) => {
    if (a.id === sourceId || a.balance.currency !== source?.balance.currency) return false;
    if (source?.cashflow_role === "investment_asset") return a.cashflow_role === "liquid_cash";
    if (recurring) {
      return a.cashflow_role === "liquid_cash" || a.cashflow_role === "investment_asset";
    }
    return TRANSFER_ROLES.has(a.cashflow_role);
  });

  // Keep the current selections valid as the mode/source changes (e.g. switching to Recurring while
  // an investment source is picked, or a destination that's no longer eligible).
  const sourceIds = sourceAccounts.map((a) => a.id).join(",");
  const destIds = destAccounts.map((a) => a.id).join(",");
  useEffect(() => {
    if (!sourceAccounts.some((a) => a.id === sourceId)) {
      setValue("source_account_id", sourceAccounts[0]?.id ?? "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- sourceIds captures the candidate set
  }, [sourceIds]);
  useEffect(() => {
    if (!destAccounts.some((a) => a.id === destId)) {
      setValue("dest_account_id", destAccounts[0]?.id ?? "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- destIds captures the candidate set
  }, [destIds]);
  const kindHint = transferKindHint(source, dest, recurring);

  const submit = handleSubmit(async (values) => {
    setError(null);
    const source = accounts.find((account) => account.id === values.source_account_id);
    const magnitude = dollarsToMinorUnits(values.amount);
    if (source === undefined || magnitude === null) return;
    const amount = { minor_units: magnitude, currency: source.balance.currency };

    // Resolve the form-only "custom" selection into the wire token (ADR 0048),
    // mirroring the backend bounds so rejection happens before the IPC call.
    let frequency = values.frequency;
    if (recurring && frequency === "custom") {
      const n = Number(values.interval_count);
      const unit = values.interval_unit as IntervalUnit;
      if (!isValidIntervalCount(n, unit)) {
        const max = INTERVAL_UNITS.find((u) => u.value === unit)?.max;
        setError(`Enter a whole number of ${unit} between 1 and ${max}.`);
        return;
      }
      frequency = buildIntervalToken(n, unit);
    }
    if (!recurring && !onCreate) return;
    const failure = recurring
      ? await onCreateRecurring({
          source_account_id: values.source_account_id,
          dest_account_id: values.dest_account_id,
          amount,
          frequency,
          anchor_date: values.date,
          idempotency_key: mintIdempotencyKey(),
        })
      : await onCreate!({
          source_account_id: values.source_account_id,
          dest_account_id: values.dest_account_id,
          amount,
          occurred_at: `${values.date}T00:00:00Z`,
          idempotency_key: mintIdempotencyKey(),
        });
    if (failure) setError(failure);
  });

  return (
    <Card>
      <CardContent className="pt-6">
        <form onSubmit={submit} className="flex flex-col gap-4">
          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="transfer-source">From</Label>
              <select
                id="transfer-source"
                className={SELECT_CLASS}
                {...register("source_account_id")}
              >
                {sourceAccounts.map((account) => (
                  <option key={account.id} value={account.id}>
                    {account.name}
                    {roleHint(account.cashflow_role)}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="transfer-dest">To</Label>
              <select
                id="transfer-dest"
                className={SELECT_CLASS}
                {...register("dest_account_id")}
              >
                {destAccounts.map((account) => (
                  <option key={account.id} value={account.id}>
                    {account.name}
                    {roleHint(account.cashflow_role)}
                  </option>
                ))}
              </select>
              {errors.dest_account_id && (
                <p className="text-xs text-loss">
                  {errors.dest_account_id.message}
                </p>
              )}
            </div>
          </div>

          {destAccounts.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              No account this one can move money to. Add a cash account to transfer, pay, or
              withdraw.
            </p>
          ) : (
            kindHint && <p className="text-xs text-muted-foreground">{kindHint}</p>
          )}

          {!lockRecurring && (
          <div className="flex flex-col gap-1.5">
            <Label>Repeat</Label>
            <div className="grid grid-cols-2 gap-2">
              <Button
                type="button"
                variant={recurring ? "outline" : "default"}
                onClick={() => setRecurring(false)}
              >
                One-off
              </Button>
              <Button
                type="button"
                variant={recurring ? "default" : "outline"}
                disabled={!hasLiquidSource}
                title={hasLiquidSource ? undefined : "Recurring transfers need a cash account"}
                onClick={() => setRecurring(true)}
              >
                Recurring
              </Button>
            </div>
          </div>
          )}

          <div className="grid grid-cols-2 gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="transfer-amount">Amount</Label>
              <Input
                id="transfer-amount"
                inputMode="decimal"
                autoFocus
                {...register("amount")}
                placeholder="0.00"
                aria-invalid={!!errors.amount}
              />
              {errors.amount && (
                <p className="text-xs text-loss">{errors.amount.message}</p>
              )}
            </div>
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="transfer-date">
                {recurring ? "Starting" : "Date"}
              </Label>
              <Input id="transfer-date" type="date" {...register("date")} />
            </div>
          </div>

          {recurring && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="transfer-frequency">Frequency</Label>
              <select
                id="transfer-frequency"
                className={SELECT_CLASS}
                {...register("frequency")}
              >
                {PAY_FREQUENCIES.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              {watch("frequency") === "custom" && (
                <div className="flex items-center gap-1.5">
                  <span className="text-sm text-muted-foreground">Every</span>
                  <Input
                    aria-label="Interval count"
                    inputMode="numeric"
                    className="h-9 w-16"
                    {...register("interval_count")}
                  />
                  <select
                    aria-label="Interval unit"
                    className={SELECT_CLASS + " w-auto"}
                    {...register("interval_unit")}
                  >
                    {INTERVAL_UNITS.map((unit) => (
                      <option key={unit.value} value={unit.value}>
                        {unit.label}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </div>
          )}

          {error && (
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
          )}

          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting || destAccounts.length === 0}>
              {isSubmitting
                ? "Adding…"
                : recurring
                  ? dest?.cashflow_role === "investment_asset"
                    ? "Schedule contribution"
                    : "Schedule transfer"
                  : "Add transfer"}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  );
}
