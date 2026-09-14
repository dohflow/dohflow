import { useMemo, useState } from "react";
import {
  Archive,
  ArchiveRestore,
  Banknote,
  Check,
  ChevronDown,
  CreditCard,
  Home,
  Landmark,
  Link2,
  Loader2,
  Pencil,
  Plus,
  Search,
  SlidersHorizontal,
  TrendingUp,
  Wallet,
} from "lucide-react";

import type { AccountViewDto } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import { NativeSelect } from "@/components/ui/native-select";
import { PageHeader } from "@/components/PageHeader";
import { cn } from "@/lib/utils";
import { formatMoney, signedAmountClass } from "@/lib/format";
import { useBaseCurrency } from "@/settings/useBaseCurrency";
import { useAccounts, useCashTiers } from "./useAccounts";
import { SUBTYPE_LABELS } from "./subtypes";
import { archiveMovesTheForecast } from "./archiveNotice";
import { storedToShownMinor } from "./balanceSign";
import { AccountDetailView, isSpendingRole } from "./AccountDetailView";
import { AccountEditorDrawer } from "./AccountEditorDrawer";
import { useBatchBalanceDraft, type BatchBalanceDraft } from "./useBatchBalanceDraft";
import { LoanDoubleCountWarning } from "./LoanDoubleCountWarning";

// `AccountViewDto.cashflow_role` tokens split across the balance sheet (ADR 0044).
const ASSET_ROLES = new Set(["liquid_cash", "investment_asset", "real_asset"]);
const LIABILITY_ROLES = new Set(["credit_facility", "loan_liability"]);

/// The per-kind sections within each column (ADR 0037 addendum, personal-cfo-4d8.23.8): each
/// renders as a collapsible accordion with an icon header, its own filter/sort, and per-row
/// edit + archive controls.
type SectionDef = { role: string; label: string; icon: typeof Banknote };
const ASSET_SECTIONS: SectionDef[] = [
  { role: "liquid_cash", label: "Cash", icon: Banknote },
  { role: "investment_asset", label: "Investments", icon: TrendingUp },
  { role: "real_asset", label: "Property & vehicles", icon: Home },
];
const LIABILITY_SECTIONS: SectionDef[] = [
  { role: "credit_facility", label: "Credit cards", icon: CreditCard },
  { role: "loan_liability", label: "Loans", icon: Landmark },
];

const ROLE_LABELS: Record<string, string> = {
  liquid_cash: "Cash",
  investment_asset: "Investment",
  real_asset: "Property / vehicle",
  credit_facility: "Credit card / line",
  loan_liability: "Loan / mortgage",
};

type AccountSort = "name" | "balance_desc" | "balance_asc";
const SORT_OPTIONS: { value: AccountSort; label: string }[] = [
  { value: "name", label: "Name A–Z" },
  { value: "balance_desc", label: "Amount high" },
  { value: "balance_asc", label: "Amount low" },
];

/// The shown figure (positive "amount owed" for liabilities, 4d8.23.3).
const shownMinor = (a: AccountViewDto) =>
  storedToShownMinor(a.cashflow_role, a.balance.minor_units);

/// The four summary figures (ADR 0044): net worth = assets − liabilities. Computed over
/// active accounts in a single display currency.
function summarize(
  accounts: AccountViewDto[],
  currency: string,
): { netWorth: number; assets: number; liabilities: number } {
  let assets = 0;
  let liabilities = 0;
  for (const a of accounts) {
    if (!a.active || a.balance.currency !== currency) continue;
    if (ASSET_ROLES.has(a.cashflow_role)) assets += a.balance.minor_units;
    else if (LIABILITY_ROLES.has(a.cashflow_role)) liabilities += a.balance.minor_units;
  }
  return { netWorth: assets + liabilities, assets, liabilities };
}

type Editor = { mode: "new" } | { mode: "edit"; account: AccountViewDto } | null;

/// The Accounts surface (Claude Design `Accounts.dc.html`, ADR 0037 addendum + ADR 0044): a
/// net-worth summary over a two-column Assets | Liabilities balance sheet, grouped into per-kind
/// accordion sections with per-row edit/archive controls and per-section filter/sort. No
/// sub-navigation — "Update balances" is an inline mode toggle, and the debt analytics live in a
/// collapsible "Debt insights" section rather than a tab (personal-cfo-4d8.23.6/.7/.8).
export function AccountsView() {
  const { accounts, error } = useAccounts();
  const { tiers } = useCashTiers();
  const { baseCurrency } = useBaseCurrency();
  const [editor, setEditor] = useState<Editor>(null);
  const [batchMode, setBatchMode] = useState(false);
  const [showArchived, setShowArchived] = useState(false);
  // Drill-in to one spending account's detail view (personal-cfo-4d8.27.5.7.4).
  const [detailId, setDetailId] = useState<string | null>(null);

  const displayCurrency =
    (accounts ?? []).find((a) => a.active)?.balance.currency ?? baseCurrency ?? "USD";
  const summary = useMemo(
    () => summarize(accounts ?? [], displayCurrency),
    [accounts, displayCurrency],
  );
  const money = (minor: number) =>
    formatMoney({ minor_units: minor, currency: displayCurrency });
  const openEditor = (account: AccountViewDto) => setEditor({ mode: "edit", account });
  // Clicking an ACTIVE spending account opens its detail view; everything else keeps
  // opening the editor (the pencil always edits). While batch balance editing is on,
  // a name click keeps its prior editor behavior instead of navigating away mid-edit.
  const openDetail = (account: AccountViewDto) => {
    if (!batchMode && account.active && isSpendingRole(account.cashflow_role)) {
      setDetailId(account.id);
    } else {
      openEditor(account);
    }
  };

  const hasAccounts = (accounts?.length ?? 0) > 0;
  // Inline batch balance editing (personal-cfo-4d8.25.24): the same grouped layout,
  // with each editable balance cell swapped for an input while `batchMode` is on.
  const batch = useBatchBalanceDraft();
  const editableAccounts = (accounts ?? []).filter(batch.isEditable);
  const editedCount = batch.editedCount(editableAccounts);

  if (detailId) {
    return (
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-4">
        <AccountDetailView accountId={detailId} onBack={() => setDetailId(null)} />
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-4xl flex-col gap-4">
      <PageHeader
        title="Accounts"
        subtitle="Track balances across everything you own and owe."
        actions={
          <Button size="sm" onClick={() => setEditor({ mode: "new" })}>
            <Plus aria-hidden />
            Add account
          </Button>
        }
      />

      <SummaryStrip
        netWorth={money(summary.netWorth)}
        netWorthClass={signedAmountClass({
          minor_units: summary.netWorth,
          currency: displayCurrency,
        })}
        assets={money(summary.assets)}
        liabilities={money(-summary.liabilities)}
        cashOnHand={tiers ? formatMoney(tiers.net) : money(0)}
      />

      <LoanDoubleCountWarning />

      {hasAccounts && (
        <div className="flex flex-wrap items-center justify-end gap-2">
          <label className="mr-auto flex h-8 cursor-pointer items-center gap-2 rounded-md border px-2.5 text-xs font-medium text-muted-foreground">
            <input
              type="checkbox"
              className="size-3.5 accent-primary"
              checked={showArchived}
              onChange={(e) => setShowArchived(e.target.checked)}
            />
            Show archived
          </label>
          <Button
            type="button"
            size="sm"
            variant={batchMode ? "secondary" : "outline"}
            aria-pressed={batchMode}
            onClick={() => {
              // Leaving batch mode discards any unsaved drafts (the flat surface
              // used to unmount) so they can't resurrect on re-entry (4d8.25.24 review).
              if (batchMode) batch.reset();
              setBatchMode((v) => !v);
            }}
          >
            <Pencil aria-hidden />
            {batchMode ? "Done updating" : "Update balances"}
          </Button>
        </div>
      )}

      {hasAccounts && batchMode && (
        <div className="flex flex-wrap items-center gap-3 rounded-md border bg-muted/30 px-3 py-2">
          <label htmlFor="batch-asof" className="text-xs font-medium text-muted-foreground">
            As of
          </label>
          <Input
            id="batch-asof"
            type="date"
            value={batch.date}
            onChange={(e) => batch.setDate(e.target.value)}
            className="h-8 w-40"
          />
          <span className="text-xs text-muted-foreground" role="status">
            {batch.savedCount > 0 && editedCount === 0
              ? `Updated ${batch.savedCount} ${batch.savedCount === 1 ? "account" : "accounts"}.`
              : `${editedCount} ${editedCount === 1 ? "change" : "changes"} to save`}
          </span>
          <Button
            size="sm"
            className="ml-auto"
            disabled={batch.saving || editedCount === 0}
            onClick={() => void batch.saveAll(editableAccounts)}
          >
            {batch.saving && <Loader2 className="size-4 animate-spin" aria-hidden />}
            Save {editedCount > 0 ? editedCount : "all"}
          </Button>
        </div>
      )}

      {error && (
        <p role="alert" className="text-sm text-loss">
          {error}
        </p>
      )}

      {accounts === null ? (
        <div className="flex items-center justify-center gap-2 py-10 text-muted-foreground">
          <Loader2 className="size-5 animate-spin" aria-hidden />
          Loading accounts…
        </div>
      ) : accounts.length === 0 ? (
        <Card>
          <CardContent className="flex flex-col items-center gap-2 py-10 text-center">
            <Wallet className="size-8 text-muted-foreground" aria-hidden />
            <p className="font-medium">No accounts yet</p>
            <p className="text-sm text-muted-foreground">
              Add everything that affects your cash — checking, cards, loans, investments —
              to start tracking.
            </p>
          </CardContent>
        </Card>
      ) : (
        <>
          <div className="grid gap-x-6 gap-y-5 md:grid-cols-2">
            <BalanceColumn
              title="Assets"
              total={money(summary.assets)}
              sections={ASSET_SECTIONS}
              accounts={accounts}
              showArchived={showArchived}
              money={money}
              onEdit={openEditor}
              onOpen={openDetail}
              batch={batchMode ? batch : null}
            />
            <BalanceColumn
              title="Liabilities"
              total={money(-summary.liabilities)}
              sections={LIABILITY_SECTIONS}
              accounts={accounts}
              showArchived={showArchived}
              money={money}
              onEdit={openEditor}
              onOpen={openDetail}
              batch={batchMode ? batch : null}
            />
          </div>
        </>
      )}

      {editor && (
        <AccountEditorDrawer
          account={editor.mode === "edit" ? editor.account : null}
          defaultCurrency={baseCurrency === "EUR" ? "EUR" : "USD"}
          onClose={() => setEditor(null)}
        />
      )}
    </div>
  );
}

function SummaryStrip({
  netWorth,
  netWorthClass,
  assets,
  liabilities,
  cashOnHand,
}: {
  netWorth: string;
  netWorthClass: string;
  assets: string;
  liabilities: string;
  cashOnHand: string;
}) {
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
      <Stat label="Net worth" value={netWorth} valueClass={netWorthClass} emphasis />
      <Stat label="Total assets" value={assets} />
      <Stat label="Total liabilities" value={liabilities} />
      <Stat label="Cash on hand" value={cashOnHand} />
    </div>
  );
}

function Stat({
  label,
  value,
  valueClass,
  emphasis = false,
}: {
  label: string;
  value: string;
  valueClass?: string;
  emphasis?: boolean;
}) {
  return (
    <Card>
      <CardContent className="flex flex-col gap-1 p-3">
        <span className="text-xs text-muted-foreground">{label}</span>
        <span
          className={cn(
            "font-semibold tabular-nums",
            emphasis ? "text-lg" : "text-base",
            valueClass,
          )}
        >
          {value}
        </span>
      </CardContent>
    </Card>
  );
}

/// One balance-sheet column (Assets or Liabilities): a header total + the per-kind sections that
/// have any accounts.
function BalanceColumn({
  title,
  total,
  sections,
  accounts,
  showArchived,
  money,
  onEdit,
  onOpen,
  batch,
}: {
  title: string;
  total: string;
  sections: SectionDef[];
  accounts: AccountViewDto[];
  showArchived: boolean;
  money: (minor: number) => string;
  onEdit: (account: AccountViewDto) => void;
  onOpen: (account: AccountViewDto) => void;
  batch: BatchBalanceDraft | null;
}) {
  const populated = sections
    .map((def) => ({
      def,
      members: accounts.filter(
        (a) => a.cashflow_role === def.role && (showArchived || a.active),
      ),
    }))
    .filter((s) => s.members.length > 0);

  return (
    <section className="flex flex-col gap-2">
      <div className="flex items-center justify-between px-1">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          {title}
        </h3>
        <span className="text-xs font-medium tabular-nums text-muted-foreground">
          {total}
        </span>
      </div>
      {populated.length === 0 ? (
        <Card>
          <CardContent className="px-4 py-6 text-center text-sm text-muted-foreground">
            Nothing here yet.
          </CardContent>
        </Card>
      ) : (
        populated.map(({ def, members }) => (
          <AccountSection
            key={def.role}
            def={def}
            members={members}
            money={money}
            onEdit={onEdit}
            onOpen={onOpen}
            batch={batch}
          />
        ))
      )}
    </section>
  );
}

/// A single collapsible account section (one `cashflow_role`) with an icon header, a subtotal, its
/// own search/sort toolbar, and compact rows with per-row edit + archive controls.
function AccountSection({
  def,
  members,
  money,
  onEdit,
  onOpen,
  batch,
}: {
  def: SectionDef;
  members: AccountViewDto[];
  money: (minor: number) => string;
  onEdit: (account: AccountViewDto) => void;
  onOpen: (account: AccountViewDto) => void;
  batch: BatchBalanceDraft | null;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const [showControls, setShowControls] = useState(false);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<AccountSort>("name");
  const Icon = def.icon;

  const subtotal = members
    .filter((a) => a.active)
    .reduce((sum, a) => sum + a.balance.minor_units, 0);

  const rows = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const filtered = members.filter(
      (a) => needle === "" || a.name.toLowerCase().includes(needle),
    );
    filtered.sort((a, b) => {
      if (a.active !== b.active) return a.active ? -1 : 1;
      switch (sort) {
        case "name":
          return a.name.localeCompare(b.name);
        case "balance_desc":
          return shownMinor(b) - shownMinor(a);
        case "balance_asc":
          return shownMinor(a) - shownMinor(b);
      }
    });
    return filtered;
  }, [members, query, sort]);

  return (
    <Card>
      <CardContent className="p-0">
        <div className="flex items-center gap-2 px-3 py-2">
          <button
            type="button"
            onClick={() => setCollapsed((v) => !v)}
            aria-expanded={!collapsed}
            aria-label={`${def.label} section`}
            className="flex min-w-0 flex-1 items-center gap-2 text-left"
          >
            <ChevronDown
              className={cn(
                "size-4 shrink-0 text-muted-foreground transition-transform",
                collapsed && "-rotate-90",
              )}
              aria-hidden
            />
            <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
            <span className="truncate text-sm font-medium">{def.label}</span>
            <span className="text-xs text-muted-foreground">({members.length})</span>
          </button>
          <span className="shrink-0 text-sm font-medium tabular-nums text-muted-foreground">
            {money(storedToShownMinor(def.role, subtotal))}
          </span>
          <button
            type="button"
            onClick={() => setShowControls((v) => !v)}
            aria-label={`Filter and sort ${def.label}`}
            aria-pressed={showControls}
            className={cn(
              "shrink-0 rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground",
              (showControls || query !== "") && "text-foreground",
            )}
          >
            <SlidersHorizontal className="size-4" aria-hidden />
          </button>
        </div>

        {!collapsed && showControls && (
          <div className="flex flex-wrap items-center gap-2 border-t bg-muted/30 px-3 py-2">
            <div className="relative min-w-40 flex-1">
              <Search
                className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
                aria-hidden
              />
              <Input
                type="search"
                role="searchbox"
                aria-label={`Search ${def.label}`}
                placeholder="Search…"
                className="h-8 pl-8 text-sm"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <NativeSelect
              aria-label={`Sort ${def.label}`}
              value={sort}
              onChange={(e) => setSort(e.target.value as AccountSort)}
              className="h-8 w-36 shrink-0 text-sm"
            >
              {SORT_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </NativeSelect>
          </div>
        )}

        {!collapsed &&
          (rows.length === 0 ? (
            <p className="border-t px-4 py-4 text-center text-sm text-muted-foreground">
              No matching accounts.
            </p>
          ) : (
            <ul className="divide-y border-t">
              {rows.map((account) => (
                <AccountRow
                  key={account.id}
                  account={account}
                  money={money}
                  onEdit={onEdit}
                  onOpen={onOpen}
                  batch={batch}
                />
              ))}
            </ul>
          ))}
      </CardContent>
    </Card>
  );
}

/// A compact account row: name + subtype + optional link chip + figure, with an explicit edit
/// button and an archive / unarchive control (personal-cfo-4d8.23.8).
function AccountRow({
  account,
  money,
  onEdit,
  onOpen,
  batch,
}: {
  account: AccountViewDto;
  money: (minor: number) => string;
  onEdit: (account: AccountViewDto) => void;
  onOpen: (account: AccountViewDto) => void;
  batch: BatchBalanceDraft | null;
}) {
  const { archiveAccount, reinstateAccount } = useAccounts();
  const [confirmingArchive, setConfirmingArchive] = useState(false);
  // In batch mode an editable account's figure becomes an inline input; everything
  // else in the row (name, subtype, controls) stays put (personal-cfo-4d8.25.24).
  const editing = batch !== null && batch.isEditable(account);
  const result = batch?.resultOf(account.id);
  return (
    <li
      className={cn(
        "group px-3 py-2 transition-colors hover:bg-muted/40",
        !account.active && "opacity-60",
      )}
    >
      <div className="flex items-center gap-2">
      <button
        type="button"
        onClick={() => onOpen(account)}
        className="flex min-w-0 flex-1 flex-col text-left"
      >
        <span className="truncate text-sm font-medium">
          {account.name}
          {!account.active && (
            <span className="ml-1.5 text-xs font-normal text-muted-foreground">
              (archived)
            </span>
          )}
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {account.subtype
            ? (SUBTYPE_LABELS[account.subtype] ?? account.subtype)
            : (ROLE_LABELS[account.cashflow_role] ?? account.cashflow_role)}
        </span>
        {account.linked_account_name && (
          <span className="mt-0.5 flex items-center gap-1 truncate text-xs text-muted-foreground">
            <Link2 className="size-3 shrink-0" aria-hidden />
            Linked to {account.linked_account_name}
          </span>
        )}
      </button>
      {editing && batch ? (
        <div className="flex shrink-0 items-center gap-1.5">
          {result?.kind === "ok" && (
            <Check className="size-4 text-gain" aria-label="Updated" />
          )}
          {result?.kind === "error" && (
            <span role="alert" className="max-w-32 truncate text-xs text-loss">
              {result.message}
            </span>
          )}
          <Input
            aria-label={`New balance for ${account.name}`}
            inputMode="decimal"
            className={cn(
              "h-8 w-32 tabular-nums",
              batch.isEdited(account) && "border-primary",
            )}
            value={batch.valueOf(account)}
            onChange={(e) => batch.setDraft(account.id, e.target.value)}
          />
        </div>
      ) : (
        <span
          className={cn(
            "shrink-0 text-sm font-medium tabular-nums",
            signedAmountClass(account.balance),
          )}
        >
          {money(storedToShownMinor(account.cashflow_role, account.balance.minor_units))}
        </span>
      )}
      <div className="flex shrink-0 items-center">
        <button
          type="button"
          onClick={() => onEdit(account)}
          aria-label={`Edit ${account.name}`}
          className="rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <Pencil className="size-4" aria-hidden />
        </button>
        {account.active ? (
          <button
            type="button"
            onClick={() => {
              // A balance that anchors the forecast must not vanish from it silently
              // (personal-cfo-tiqf, ADR 0056). A zero balance moves nothing, so it keeps
              // the one-click path.
              if (archiveMovesTheForecast(account)) setConfirmingArchive(true);
              else void archiveAccount(account.id);
            }}
            aria-label={`Archive ${account.name}`}
            className="rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <Archive className="size-4" aria-hidden />
          </button>
        ) : (
          <button
            type="button"
            onClick={() => void reinstateAccount(account.id)}
            aria-label={`Restore ${account.name}`}
            className="rounded-md p-1.5 text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <ArchiveRestore className="size-4" aria-hidden />
          </button>
        )}
      </div>
      </div>
      {/* Sits UNDER the row rather than inside it: the row's own name is a button that
          opens the account, and a button inside a button is invalid HTML. */}
      {confirmingArchive && (
        <div
          role="alertdialog"
          aria-label={`Archive ${account.name}`}
          className="mt-2 rounded-md border bg-muted/40 p-3 text-sm"
        >
          <p>
            {`Archiving ${account.name} stops counting its ${money(
              storedToShownMinor(account.cashflow_role, account.balance.minor_units),
            )} toward your forecast, so your projected cash will drop by that much. Restoring the account brings it back.`}
          </p>
          <div className="mt-2 flex gap-1">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setConfirmingArchive(false);
                void archiveAccount(account.id);
              }}
              aria-label={`Confirm archive ${account.name}`}
            >
              Archive
            </Button>
            <Button
              size="sm"
              variant="ghost"
              type="button"
              onClick={() => setConfirmingArchive(false)}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}
    </li>
  );
}

