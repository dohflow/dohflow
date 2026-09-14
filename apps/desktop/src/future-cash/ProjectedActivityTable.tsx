import { useState } from "react";
import { AlertTriangle, ChevronRight, Columns3, Search } from "lucide-react";

import type { ForecastEventDto, MultiSeriesForecastDto } from "@/bindings";
import { MarkObligationPaid } from "./MarkObligationPaid";
import { AdjustOccurrenceAmount } from "./AdjustOccurrenceAmount";
import { Input } from "@/components/ui/input";
import { NativeSelect } from "@/components/ui/native-select";
import { DataTable, type DataTableColumn } from "@/components/ui/data-table";
import {
  formatIsoDate,
  formatMoney,
  formatSignedMoney,
  signedAmountClass,
} from "@/lib/format";
import { cn } from "@/lib/utils";
import { PaginationControls } from "@/components/ui/pagination";
import { PAGE_SIZE_OPTIONS, usePagination } from "@/lib/usePagination";
import { explainEvent } from "./explainEvent";
import { ForecastRowExplanation } from "./ForecastRowExplanation";

const TIER_LABEL: Record<string, string> = {
  spendable: "Spendable",
  reserve: "Reserve",
  unallocated: "Unallocated",
  net: "Net cash",
};

/// Deterministic order for the non-net tier columns.
const TIER_ORDER: Record<string, number> = {
  spendable: 0,
  reserve: 1,
  unallocated: 2,
};

/// A balance column: a cash tier, or an individual account (personal-cfo-4d8.27.7.2).
/// `key` indexes `Row.running` — tier tokens and account ids share that map and cannot
/// collide (tiers are fixed words, accounts are UUIDs).
type Column = {
  key: string;
  label: string;
  emphasize: boolean;
  kind: "tier" | "account";
};

/// One ledger row: TODAY (`event === null`), then each projected activity. `running`
/// is the per-tier balance AFTER this row's amount is applied (TODAY = the opening
/// balances, before any activity).
type Row = {
  key: string;
  date: string;
  event: ForecastEventDto | null;
  running: Record<string, number>;
  /// The liquid account this row's amount moves (null for TODAY / Unallocated).
  accountId: string | null;
  accountName: string | null;
  /// Set when this row pushes its own account below zero — the grouped columns
  /// average that away, hiding a real overdraft (personal-cfo-4d8.27.7.1).
  overdraft: { accountName: string; balanceMinor: number } | null;
};

/// The left-border accent per activity type (brand + semantic finance tokens).
function kindBorder(kind: string | undefined): string {
  if (kind === "income") return "border-l-gain";
  if (kind === "recurring_bill" || kind === "loan_payment") return "border-l-loss";
  if (kind === undefined) return "border-l-transparent"; // TODAY
  return "border-l-primary"; // manual_entry / transfer / other
}

/// Display labels for the activity-kind filter (mirrors explainEvent's KIND_LABELS;
/// unknown kinds fall back to the raw token so nothing is unfilterable).
const KIND_FILTER_LABEL: Record<string, string> = {
  income: "Income",
  recurring_bill: "Recurring bill",
  loan_payment: "Loan payment",
  transfer: "Transfer",
  manual_entry: "Manual entry",
};

/// FILTER ONLY — deliberately no re-sorting on this table (personal-cfo-yequ): the
/// balance columns are per-row running balances computed in date order, so reordering
/// rows would make those balances lie. Hiding rows is safe — each surviving row keeps
/// its true per-day balance.
function filterActivityRows(rows: Row[], query: string, kind: string): Row[] {
  const needle = query.trim().toLowerCase();
  return rows.filter((row) => {
    if (row.event === null) return true; // TODAY is never filtered out
    if (kind !== "" && row.event.kind !== kind) return false;
    return needle === "" || row.event.name.toLowerCase().includes(needle);
  });
}

/// The projected-activity table (personal-cfo-ygjs, rebuilt for personal-cfo-4d8.10, and
/// moved onto the shared `DataTable` primitive for personal-cfo-wxy7 / ADR 0053).
/// Columns: Activity · Date · Amount (the signed
/// delta) · the cash tiers (Spendable / Reserve / …) · Net cash, with the balance
/// columns showing a **per-row running balance** — two activities on the same day step
/// the balance twice instead of both showing the day-end total. The day-end row
/// reconciles to the backend's closing for that date (the chart keeps one end-of-day
/// point per date). Per-account columns are deferred to the column-picker cog
/// (personal-cfo-inaw); the feedback asked for accounts hidden by default. Rows expand
/// to the typed explanation (personal-cfo-vkge).
export function ProjectedActivityTable({
  projection,
}: {
  projection: MultiSeriesForecastDto;
}) {
  const { currency } = projection;
  const [openKey, setOpenKey] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState("");
  // Individual account columns the user has added (personal-cfo-4d8.27.7.2): the
  // default stays the grouped tiers, per the feedback that accounts are hidden by
  // default — this is the opt-in for "how does this transaction hit THIS account".
  const [accountColumns, setAccountColumns] = useState<string[]>([]);

  const balanceColumns = buildColumns(projection, accountColumns);
  const rows = buildRows(projection, balanceColumns);
  // TODAY (the opening-balance anchor) stays pinned; the activity rows below it
  // paginate (default 10, sticky size) so a long horizon is not an endless scroll
  // (personal-cfo-4d8.12). Running balances are computed over the full series, so each
  // page still shows correct absolute balances.
  const today = rows[0];
  const activities = rows.slice(1);
  // The kind options come from the rows actually present, so the picker never offers
  // a dead filter. Filtering happens AFTER buildRows: balances stay computed over the
  // full series, so a hidden row never distorts a visible row's running balance.
  const kindOptions = [
    ...new Set(activities.flatMap((row) => (row.event ? [row.event.kind] : []))),
  ];
  const filteredActivities = filterActivityRows(activities, query, kind);
  const activityPages = usePagination(
    filteredActivities,
    "pcfo.pageSize.projectedActivity",
  );
  const visibleRows = today
    ? [today, ...activityPages.pageItems]
    : activityPages.pageItems;
  const filteredOut = activities.length > 0 && filteredActivities.length === 0;

  // Column defs for the shared DataTable (ADR 0053): Activity + Date + Amount, then one
  // per balance column — so the expander row's span is `columns.length` by construction
  // rather than the `3 + columns.length` this file used to compute by hand.
  //
  // No column opts into `sortable`, and that is the point (ADR 0053 §4): the balance
  // columns are per-row running balances folded in date order, so re-sorting them would
  // make every one of them lie (personal-cfo-yequ).
  const columns: DataTableColumn<Row>[] = [
    {
      key: "activity",
      header: "Activity",
      className: "align-top",
      cell: (row) => {
        const open = openKey === row.key;
        return (
          <ActivityCell
            row={row}
            currency={currency}
            open={open}
            panelId={`explain-${row.key}`}
            onToggle={() => setOpenKey(open ? null : row.key)}
          />
        );
      },
    },
    {
      key: "date",
      header: "Date",
      className: "align-top text-muted-foreground",
      cell: (row) => formatIsoDate(row.date),
    },
    {
      key: "amount",
      header: "Amount",
      align: "right",
      className: "align-top tabular-nums",
      cell: (row) => {
        const amount = row.event?.amount ?? null;
        // The gain/loss colour is per ROW, so it goes on a span — `column.className` is
        // shared by every cell in the column.
        return (
          <span
            className={amount ? signedAmountClass(amount) : "text-muted-foreground"}
          >
            {amount ? formatSignedMoney(amount) : "—"}
          </span>
        );
      },
    },
    ...balanceColumns.map(
      (column): DataTableColumn<Row> => ({
        key: column.key,
        header: column.label,
        align: "right",
        headerClassName: column.emphasize ? "text-foreground" : undefined,
        className: cn(
          "align-top tabular-nums",
          column.emphasize ? "font-semibold" : "text-muted-foreground",
        ),
        cell: (row) => (
          <span
            className={cn(
              // A per-account column that has gone negative is the overdraft itself.
              column.kind === "account" &&
                (row.running[column.key] ?? 0) < 0 &&
                "text-loss",
            )}
          >
            {formatMoney({ minor_units: row.running[column.key] ?? 0, currency })}
          </span>
        ),
      }),
    ),
  ];

  return (
    <div className="flex flex-col gap-3">
      {activities.length > 0 && (
        <div className="flex items-center gap-2">
          <div className="relative flex-1">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              type="search"
              role="searchbox"
              aria-label="Filter projected activity"
              placeholder="Filter by activity name…"
              className="pl-9"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <NativeSelect
            aria-label="Filter by activity type"
            value={kind}
            onChange={(e) => setKind(e.target.value)}
            className="w-40 shrink-0"
          >
            <option value="">All types</option>
            {kindOptions.map((option) => (
              <option key={option} value={option}>
                {KIND_FILTER_LABEL[option] ?? option}
              </option>
            ))}
          </NativeSelect>
          <AccountColumnPicker
            accounts={projection.accounts}
            selected={accountColumns}
            onChange={setAccountColumns}
          />
        </div>
      )}
      <DataTable
        columns={columns}
        rows={visibleRows}
        rowKey={(row) => row.key}
        label="Projected activity"
        minWidth="min-w-[640px]"
        className="flex flex-col gap-3"
        rowProps={(row) => ({
          className: cn(
            "border-l-2",
            kindBorder(row.event?.kind),
            row.event === null && "bg-muted/40",
          ),
        })}
        expandedContent={(row) =>
          row.event !== null && openKey === row.key ? (
            <ExplanationPanel
              event={row.event}
              date={row.date}
              panelId={`explain-${row.key}`}
              impacted={impactedAccounts(projection, row)}
            />
          ) : null
        }
        // TODAY is pinned INSIDE the rows (its balance cells are the same running
        // balances every other row renders, so they have to come from the column defs),
        // which means this table is never `rows.length === 0` and `empty` can never
        // fire. The filtered-to-nothing notice is a trailing row instead
        // (personal-cfo-wxy7).
        trailingRow={
          filteredOut ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              No matching activity — clear the filter.
            </p>
          ) : null
        }
        footer={
          activityPages.total > PAGE_SIZE_OPTIONS[0] ? (
            <PaginationControls
              pagination={activityPages}
              noun="activities"
              className="px-1"
            />
          ) : null
        }
      />
    </div>
  );
}

/// Default columns: the cash tiers present in the data (Spendable + Reserve always, as
/// the headline tiers; Unallocated only when it carries a balance), then Net cash
/// fixed at the far right. So Spendable + Reserve [+ Unallocated] always reconciles to
/// Net.
/// Add/remove individual-account balance columns (personal-cfo-4d8.27.7.2). A plain
/// disclosure of checkboxes — the same shape as the chart's series picker, without
/// coupling the table's columns to the chart's persisted selection (they answer
/// different questions).
function AccountColumnPicker({
  accounts,
  selected,
  onChange,
}: {
  accounts: MultiSeriesForecastDto["accounts"];
  selected: string[];
  onChange: (next: string[]) => void;
}) {
  const [open, setOpen] = useState(false);
  const real = accounts.filter((a) => a.account_id !== null);
  if (real.length === 0) return null;
  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
      >
        <Columns3 aria-hidden className="size-3.5" />
        Accounts
        {selected.length > 0 && (
          <span className="rounded-full bg-primary/15 px-1.5 text-[10px] font-semibold text-primary">
            {selected.length}
          </span>
        )}
      </button>
      {open && (
        <div
          role="group"
          aria-label="Account columns"
          className="absolute right-0 z-20 mt-1 flex w-56 flex-col gap-1 rounded-md border bg-popover p-2 shadow-lg"
        >
          {real.map((account) => {
            const id = account.account_id!;
            const on = selected.includes(id);
            return (
              <label
                key={id}
                className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-sm hover:bg-muted"
              >
                <input
                  type="checkbox"
                  checked={on}
                  onChange={() =>
                    onChange(
                      on ? selected.filter((x) => x !== id) : [...selected, id],
                    )
                  }
                />
                <span className="truncate">{account.name}</span>
              </label>
            );
          })}
        </div>
      )}
    </div>
  );
}

/// Where a row's money moves (personal-cfo-4d8.27.7.5), derived from the projection.
///
/// A recurring transfer is emitted as TWO legs sharing one `source_event_id` and date —
/// negative on the source account, positive on the destination (ADR 0026 §14) — so both
/// ends are recoverable client-side. Everything else touches exactly one account: an
/// outflow leaves it, an inflow lands in it.
function impactedAccounts(
  projection: MultiSeriesForecastDto,
  row: Row,
): { from: string | null; to: string | null } {
  if (row.event === null) return { from: null, to: null };
  const legs = projection.accounts
    .filter((a) => a.account_id !== null)
    .flatMap((a) =>
      a.days
        .filter((d) => d.date === row.date)
        .flatMap((d) =>
          d.events
            .filter((e) => e.source_event_id === row.event!.source_event_id)
            .map((e) => ({ name: a.name, minor: e.amount.minor_units })),
        ),
    );
  const source = legs.find((l) => l.minor < 0)?.name ?? null;
  const dest = legs.find((l) => l.minor > 0)?.name ?? null;
  if (source && dest) return { from: source, to: dest };
  // Single-sided: fall back to this row's own account so income/bills still say where.
  return row.event.amount.minor_units < 0
    ? { from: row.accountName, to: null }
    : { from: null, to: row.accountName };
}

function buildColumns(
  projection: MultiSeriesForecastDto,
  accountColumns: string[],
): Column[] {
  const { groups, accounts } = projection;
  const tierColumns: Column[] = groups
    .filter((g) => g.tier !== "net")
    .filter(
      (g) =>
        g.tier === "spendable" ||
        g.tier === "reserve" ||
        g.closings.some((c) => c.closing.p50.minor_units !== 0),
    )
    .map((g) => g.tier)
    .sort((a, b) => (TIER_ORDER[a] ?? 99) - (TIER_ORDER[b] ?? 99))
    .map((tier) => ({
      key: tier,
      label: TIER_LABEL[tier] ?? tier,
      emphasize: false,
      kind: "tier" as const,
    }));
  // Individual accounts the user asked to see, in the projection's own order so the
  // columns are stable (personal-cfo-4d8.27.7.2).
  const picked: Column[] = accounts
    .filter((a) => a.account_id !== null && accountColumns.includes(a.account_id))
    .map((a) => ({
      key: a.account_id!,
      label: a.name,
      emphasize: false,
      kind: "account" as const,
    }));
  const hasNet = groups.some((g) => g.tier === "net");
  return hasNet
    ? [
        ...tierColumns,
        ...picked,
        { key: "net", label: TIER_LABEL.net!, emphasize: true, kind: "tier" as const },
      ]
    : [...tierColumns, ...picked];
}

/// Fold the events into a per-row running balance per tier column. TODAY = the opening
/// balances (each tier's first-day closing minus that day's events); each activity row
/// then adds its amount to the activity's own tier + Net. The fold is linear, so the
/// last row of any date reconciles to that date's backend closing.
function buildRows(
  projection: MultiSeriesForecastDto,
  columns: Column[],
): Row[] {
  const { start_date, accounts, groups } = projection;

  const tierClosingOnFirst = (tier: string): number =>
    groups
      .find((g) => g.tier === tier)
      ?.closings.find((c) => c.date === start_date)?.closing.p50.minor_units ?? 0;

  // Sum of a tier's event amounts on a date (Net = every account; otherwise the
  // accounts whose tier matches).
  const tierEventsOnDate = (tier: string, date: string): number =>
    accounts
      .filter((a) => tier === "net" || a.tier === tier)
      .flatMap((a) => a.days.filter((d) => d.date === date))
      .flatMap((d) => d.events)
      .reduce((sum, e) => sum + e.amount.minor_units, 0);

  const running: Record<string, number> = {};
  for (const col of columns.filter((c) => c.kind === "tier")) {
    running[col.key] =
      tierClosingOnFirst(col.key) - tierEventsOnDate(col.key, start_date);
  }
  // EVERY real account is folded, not just the displayed ones: the overdraft warning
  // exists precisely for accounts the grouped columns hide
  // (personal-cfo-4d8.27.7.1/.7.2).
  for (const account of accounts) {
    if (account.account_id === null) continue;
    const opening = account.days.find((d) => d.date === start_date);
    const openingEvents = (opening?.events ?? []).reduce(
      (sum, e) => sum + e.amount.minor_units,
      0,
    );
    running[account.account_id] =
      (opening?.closing.p50.minor_units ?? 0) - openingEvents;
  }

  const rows: Row[] = [
    {
      key: "today",
      date: start_date,
      event: null,
      running: { ...running },
      accountId: null,
      accountName: null,
      overdraft: null,
    },
  ];

  // Every activity in global date order; a stable sort preserves each account's
  // canonical within-day order. Each carries the tier + account its amount moves.
  const activities = accounts
    .flatMap((a) =>
      a.days.flatMap((d) =>
        d.events.map((event, i) => ({
          key: `${event.source_event_id}-${d.date}-${a.account_id ?? "unalloc"}-${i}`,
          date: d.date,
          event,
          tier: a.tier,
          accountId: a.account_id,
          accountName: a.account_id === null ? null : a.name,
        })),
      ),
    )
    .sort((x, y) => x.date.localeCompare(y.date));

  // Authoritative day-end values, so the running fold cannot drift from the chart: the
  // Layer-2 spend model lowers an account's p50 WITHOUT emitting events (ADR 0026 §7),
  // so an event-only fold would slowly diverge — and the overdraft warning built on it
  // would under-fire (adversarial review of 4d8.27.7.1).
  const closingOnDate = (key: string, date: string): number | undefined => {
    const tier = groups.find((g) => g.tier === key);
    if (tier) {
      return tier.closings.find((c) => c.date === date)?.closing.p50.minor_units;
    }
    return accounts
      .find((a) => a.account_id === key)
      ?.days.find((d) => d.date === date)?.closing.p50.minor_units;
  };
  /// Snap every tracked series to its day-end closing and, if that reveals an account
  /// going under (spend-driven, no event to blame), flag the date's last row.
  const pinDate = (date: string) => {
    for (const key of Object.keys(running)) {
      const closing = closingOnDate(key, date);
      if (closing === undefined) continue;
      const before = running[key] ?? 0;
      running[key] = closing;
      const account = accounts.find((a) => a.account_id === key);
      if (account && before >= 0 && closing < 0) {
        const last = rows[rows.length - 1];
        if (last && last.overdraft === null) {
          last.overdraft = { accountName: account.name, balanceMinor: closing };
        }
      }
    }
    const last = rows[rows.length - 1];
    if (last) last.running = { ...running };
  };

  let pendingDate: string | null = null;
  for (const act of activities) {
    if (pendingDate !== null && act.date !== pendingDate) pinDate(pendingDate);
    pendingDate = act.date;
    const amt = act.event.amount.minor_units;
    // Apply to the activity's own tier (if shown) + Net. Locals keep the narrowing
    // across the assignment under noUncheckedIndexedAccess.
    const own = running[act.tier];
    if (own !== undefined) running[act.tier] = own + amt;
    if (running.net !== undefined) running.net += amt;
    // …and to the account itself, which is what reveals an overdraft the grouped
    // columns would otherwise average away.
    let overdraft: Row["overdraft"] = null;
    if (act.accountId !== null) {
      const before = running[act.accountId] ?? 0;
      const after = before + amt;
      running[act.accountId] = after;
      // Flag the CROSSING only — "this transaction will drop XYZ below $0". Once an
      // account is already negative, every later row would otherwise re-alarm; the
      // per-account column stays red for as long as it is under, which carries
      // "still negative" without a warning on every row.
      if (before >= 0 && after < 0 && act.accountName !== null) {
        overdraft = { accountName: act.accountName, balanceMinor: after };
      }
    }
    rows.push({
      key: act.key,
      date: act.date,
      event: act.event,
      running: { ...running },
      accountId: act.accountId,
      accountName: act.accountName,
      overdraft,
    });
  }
  if (pendingDate !== null) pinDate(pendingDate);

  return rows;
}

/// The Activity column's cell: the disclosure control (or the TODAY label), plus the
/// per-account overdraft warning this row triggers.
function ActivityCell({
  row,
  currency,
  open,
  panelId,
  onToggle,
}: {
  row: Row;
  currency: string;
  open: boolean;
  panelId: string;
  onToggle: () => void;
}) {
  const { event } = row;
  return (
    <>
      {event ? (
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          aria-controls={panelId}
          className="flex items-start gap-1.5 text-left font-medium transition-colors hover:text-primary"
        >
          {/* Rows have always expanded to their explanation, but nothing said so
              (personal-cfo-4d8.27.7.3) — a rotating chevron makes it discoverable. */}
          <ChevronRight
            aria-hidden
            className={cn(
              "mt-0.5 size-3.5 shrink-0 text-muted-foreground transition-transform",
              open && "rotate-90",
            )}
          />
          <span>{event.name || "—"}</span>
        </button>
      ) : (
        // TODAY is the opening anchor, not an activity — nothing to expand.
        <span className="pl-5 font-medium">Today</span>
      )}
      {/* The grouped columns can net an individual account's overdraft away — say
          it plainly, since that is a real overdraft fee (personal-cfo-4d8.27.7.1). */}
      {/* Not a live region: these render WITH the table, so `role="alert"` would make
          a screen reader announce every one on load. The text is descriptive and is
          read in document order. */}
      {row.overdraft && (
        <span className="mt-1 flex items-center gap-1 pl-5 text-xs text-loss">
          <AlertTriangle aria-hidden className="size-3 shrink-0" />
          Overdraws {row.overdraft.accountName} to{" "}
          {formatMoney({ minor_units: row.overdraft.balanceMinor, currency })}
        </span>
      )}
    </>
  );
}

/// A row's expanded detail: the typed explanation, where the money moves, and the
/// per-occurrence controls. The DataTable owns the row and its span.
function ExplanationPanel({
  event,
  date,
  panelId,
  impacted,
}: {
  event: ForecastEventDto;
  date: string;
  panelId: string;
  /// Where this row's money moves (personal-cfo-4d8.27.7.5).
  impacted: { from: string | null; to: string | null };
}) {
  return (
    <>
      <ForecastRowExplanation id={panelId} explanation={explainEvent(event, date)} />
      {/* Where the money actually moves (personal-cfo-4d8.27.7.5): a transfer
          shows both ends, everything else the single account it hits. */}
      {(impacted.from || impacted.to) && (
        <p className="px-4 pb-3 text-sm text-muted-foreground">
          {impacted.from && impacted.to ? (
            <>
              Moves money from{" "}
              <span className="font-medium text-foreground">{impacted.from}</span> to{" "}
              <span className="font-medium text-foreground">{impacted.to}</span>
            </>
          ) : impacted.from ? (
            <>
              Comes out of{" "}
              <span className="font-medium text-foreground">{impacted.from}</span>
            </>
          ) : (
            <>
              Goes into{" "}
              <span className="font-medium text-foreground">{impacted.to}</span>
            </>
          )}
        </p>
      )}
      <AdjustOccurrenceAmount event={event} scheduledDate={date} />
      {(event.kind === "recurring_bill" || event.kind === "loan_payment") && (
        <MarkObligationPaid event={event} scheduledDate={date} />
      )}
    </>
  );
}
