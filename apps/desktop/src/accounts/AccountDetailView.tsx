/// Account Detail (personal-cfo-4d8.27.5.7.4): a focused view of ONE spending account —
/// realized balance history behind a today divider, the projected median with its widening
/// uncertainty cone ahead of it (ADR 0050), type-aware stats, and past + upcoming activity.
/// Built to the approved Claude Design mock (Account Detail.dc.html); reached by clicking a
/// spending account in the Accounts list.

import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Pencil, Receipt, Wallet } from "lucide-react";

import type { AccountViewDto, TransactionRowDto } from "@/bindings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { DataTable, type DataTableColumn } from "@/components/ui/data-table";
import { EmptyState } from "@/components/ui/empty-state";
import { NativeSelect } from "@/components/ui/native-select";
import { PaginationControls } from "@/components/ui/pagination";
import { Skeleton } from "@/components/ui/skeleton";
import { TableCell, TableRow } from "@/components/ui/table";
import {
  formatIsoDate,
  formatMoney,
  formatSignedMoney,
  signedAmountClass,
} from "@/lib/format";
import { type Pagination } from "@/lib/usePagination";
import { EMPTY_FILTERS } from "@/transactions/filters";
import { usePagedTransactions } from "@/transactions/usePagedTransactions";
import { useFutureCashByAccount } from "@/future-cash/useFutureCash";

import { AccountBalanceChart, type ChartMarker } from "./AccountBalanceChart";
import {
  buildCardChartRows,
  buildLiquidChartRows,
  type DetailChartRow,
} from "./accountDetailSeries";
import { figureLabelForRole, storedToShownMinor } from "./balanceSign";
import { ReconcileBalanceModal } from "./ReconcileBalanceModal";
import { SetBalanceModal } from "./SetBalanceModal";
import { SUBTYPE_LABELS } from "./subtypes";
import { useAccounts } from "./useAccounts";
import { useCardStatementForecast } from "./useCardStatementForecast";
import { useCashFlowHistory } from "./useCashFlowHistory";

/// History lookback + forward horizon per range choice (days). Forward is shorter than
/// back, mirroring the approved mock's proportions.
const RANGES = {
  "3M": { lookback: 90, horizon: 45 },
  "6M": { lookback: 180, horizon: 90 },
  "12M": { lookback: 365, horizon: 120 },
} as const;
type RangeKey = keyof typeof RANGES;

/// The roles this view covers — accounts one spends from.
export function isSpendingRole(role: string): boolean {
  return role === "liquid_cash" || role === "credit_facility";
}

/// Badge fallback when an account has no subtype (only spending roles reach this view).
const SPENDING_ROLE_LABELS: Record<string, string> = {
  liquid_cash: "Cash",
  credit_facility: "Credit card",
};

/// One upcoming (projected) activity row.
type UpcomingRow = {
  key: string;
  date: string;
  label: string;
  detail: string | null;
  amountMinor: number | null;
};

export function AccountDetailView({
  accountId,
  onBack,
}: {
  accountId: string;
  onBack: () => void;
}) {
  const [selectedId, setSelectedId] = useState(accountId);
  const [range, setRange] = useState<RangeKey>("6M");
  const [modal, setModal] = useState<"none" | "set" | "reconcile">("none");
  const queryClient = useQueryClient();

  const { accounts } = useAccounts();
  const { lookback, horizon } = RANGES[range];
  const { history, error: historyError, loading: historyLoading } =
    useCashFlowHistory(lookback);
  const { projection, error: forecastError } = useFutureCashByAccount(horizon);
  const { cards, error: cardsError } = useCardStatementForecast();

  const spendingAccounts = (accounts ?? []).filter(
    (a) => a.active && isSpendingRole(a.cashflow_role),
  );
  const account = (accounts ?? []).find((a) => a.id === selectedId);
  const isCard = account?.cashflow_role === "credit_facility";
  const card = isCard ? (cards ?? []).find((c) => c.account_id === selectedId) : undefined;

  const filters = useMemo(
    () => ({ ...EMPTY_FILTERS, accountIds: [selectedId] }),
    [selectedId],
  );
  const {
    rows: postedRows,
    total: postedTotal,
    pagination,
    error: postedError,
  } = usePagedTransactions(filters, "newest", "pcfo.pageSize.accountDetail");

  const historySeries = history?.accounts.find((a) => a.account_id === selectedId);
  const forwardSeries = projection?.accounts.find((a) => a.account_id === selectedId);
  const todayIso = history?.end_date ?? "";

  const chartRows: DetailChartRow[] = useMemo(() => {
    if (!account) return [];
    return isCard
      ? buildCardChartRows(account.cashflow_role, historySeries, card, todayIso, horizon)
      : buildLiquidChartRows(account.cashflow_role, historySeries, forwardSeries);
  }, [account, isCard, historySeries, card, forwardSeries, todayIso, horizon]);

  // Upcoming activity: a card's statement closes + payments, or a liquid account's
  // attributed forecast events, soonest first.
  const upcoming: UpcomingRow[] = useMemo(() => {
    // Without the history's household "today" every date compares later than "",
    // which would list even closed cycles — wait for it instead.
    if (isCard && card && todayIso) {
      const rows: UpcomingRow[] = [];
      for (const cycle of card.cycles) {
        if (cycle.close_date > todayIso) {
          rows.push({
            key: `close-${cycle.close_date}`,
            date: cycle.close_date,
            label: cycle.statement_is_actual
              ? "Statement closes"
              : "Statement closes (est.)",
            detail: null,
            amountMinor: cycle.statement_balance_minor,
          });
        }
        if (cycle.due_date > todayIso && cycle.forecast_payment_minor > 0) {
          rows.push({
            key: `due-${cycle.due_date}`,
            date: cycle.due_date,
            label: "Payment",
            detail: "reduces the balance owed",
            amountMinor: -cycle.forecast_payment_minor,
          });
        }
      }
      return rows.sort((a, b) => a.date.localeCompare(b.date)).slice(0, 8);
    }
    return (forwardSeries?.days ?? [])
      .flatMap((day) =>
        day.events.map((e, i) => ({
          key: `${day.date}-${e.source_event_id}-${i}`,
          date: day.date,
          label: e.name,
          detail: e.kind.replace(/_/g, " "),
          amountMinor: e.amount.minor_units,
        })),
      )
      .slice(0, 8);
  }, [isCard, card, forwardSeries, todayIso]);

  // Chart markers: the next payment / statement close (cards), or the single largest
  // upcoming flow (liquid) — pinned to the projected median at that date.
  const markers: ChartMarker[] = useMemo(() => {
    const p50At = (date: string) =>
      chartRows.find((r) => r.date === date && r.p50 !== undefined)?.p50;
    const picks = isCard
      ? upcoming.slice(0, 2)
      : [...upcoming]
          .sort((a, b) => Math.abs(b.amountMinor ?? 0) - Math.abs(a.amountMinor ?? 0))
          .slice(0, 1);
    return picks.flatMap((u) => {
      const value = p50At(u.date);
      return value === undefined ? [] : [{ date: u.date, value, label: u.label }];
    });
  }, [chartRows, upcoming, isCard]);

  // Running balance for posted rows: walk backward from the current stored balance —
  // only exact on the newest page, so later pages omit the column's values. Keyed by
  // transaction id rather than position, because a DataTable column's `cell(row)` is
  // handed the row, not its index.
  const runningBalances: Map<string, number> = useMemo(() => {
    const balances = new Map<string, number>();
    if (!account || pagination.page !== 0) return balances;
    let after = account.balance.minor_units;
    for (const t of postedRows) {
      balances.set(t.transaction_id, after);
      after -= t.amount.minor_units;
    }
    return balances;
  }, [account, postedRows, pagination.page]);

  const error = historyError ?? forecastError ?? (isCard ? cardsError : null);
  const loading = historyLoading || (!history && !error);

  if (!loading && accounts !== null && !account) {
    return (
      <div className="space-y-4">
        <BackLink onBack={onBack} />
        <EmptyState
          icon={Wallet}
          title="Account not found"
          description="It may have been removed. Head back to your accounts."
        />
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0">
          <BackLink onBack={onBack} />
          <div className="mt-2 flex flex-wrap items-center gap-2.5">
            <h2 className="text-2xl font-semibold tracking-tight">
              {account?.name ?? "Account"}
            </h2>
            {account && (
              <Badge variant="secondary">
                {account.subtype
                  ? (SUBTYPE_LABELS[account.subtype] ?? account.subtype)
                  : (SPENDING_ROLE_LABELS[account.cashflow_role] ?? account.cashflow_role)}
              </Badge>
            )}
          </div>
        </div>
        {spendingAccounts.length > 1 && (
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            Account
            <NativeSelect
              aria-label="Switch account"
              value={selectedId}
              onChange={(e) => setSelectedId(e.target.value)}
            >
              {spendingAccounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                </option>
              ))}
            </NativeSelect>
          </label>
        )}
      </div>

      {error && (
        <Card>
          <CardContent className="flex flex-wrap items-center justify-between gap-3 py-4">
            <p role="alert" className="text-sm text-loss">
              {error}
            </p>
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                void queryClient.invalidateQueries({ queryKey: ["forecast"] })
              }
            >
              Try again
            </Button>
          </CardContent>
        </Card>
      )}

      {loading && !error && (
        <div className="space-y-4" aria-label="Loading account detail">
          <Skeleton className="h-10 w-64" />
          <Skeleton className="h-80 w-full" />
          <Skeleton className="h-48 w-full" />
        </div>
      )}

      {!loading && account && (
        <>
          <div className="flex flex-wrap items-end justify-between gap-4">
            <div>
              <p className="text-xs font-medium text-muted-foreground">
                {figureLabelForRole(account.cashflow_role)}
              </p>
              <div className="mt-1 flex items-center gap-3">
                <span className="text-4xl font-semibold tracking-tight tabular-nums">
                  {formatMoney({
                    minor_units: storedToShownMinor(
                      account.cashflow_role,
                      account.balance.minor_units,
                    ),
                    currency: account.balance.currency,
                  })}
                </span>
                <Button variant="outline" size="sm" onClick={() => setModal("set")}>
                  <Pencil className="size-3.5" aria-hidden /> Edit
                </Button>
              </div>
              {todayIso && (
                <p className="mt-1.5 text-xs text-muted-foreground">
                  as of {formatIsoDate(todayIso)} · manual account, you keep it current
                </p>
              )}
            </div>
            <div className="flex flex-col items-end gap-1.5">
              <span className="text-[11px] text-muted-foreground">History & horizon</span>
              <div
                role="group"
                aria-label="History and horizon range"
                className="flex overflow-hidden rounded-md border border-border"
              >
                {(Object.keys(RANGES) as RangeKey[]).map((key) => (
                  <button
                    key={key}
                    type="button"
                    aria-pressed={range === key}
                    onClick={() => setRange(key)}
                    className={
                      range === key
                        ? "bg-secondary px-3 py-1.5 text-xs font-semibold"
                        : "px-3 py-1.5 text-xs text-muted-foreground hover:bg-muted"
                    }
                  >
                    {key}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {isCard && card && (
            <CardStatsStrip account={account} card={card} todayIso={todayIso} />
          )}

          <Card>
            <CardHeader>
              <CardTitle>Balance &amp; projection</CardTitle>
              <p className="text-sm text-muted-foreground">
                {isCard
                  ? "What you've owed, and where the balance is headed as statements close and payments post — with the honest range around estimates."
                  : "Your realized balance, and where it's headed — the range widens the further out you look."}
              </p>
            </CardHeader>
            <CardContent>
              <AccountBalanceChart
                rows={chartRows}
                currency={account.balance.currency}
                todayIso={todayIso}
                markers={markers}
                figureLabel={figureLabelForRole(account.cashflow_role)}
              />
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Activity</CardTitle>
            </CardHeader>
            <CardContent>
              <ActivityTable
                account={account}
                upcoming={upcoming}
                postedRows={postedRows}
                postedTotal={postedTotal}
                postedError={postedError}
                pagination={pagination}
                runningBalances={runningBalances}
              />
            </CardContent>
          </Card>
        </>
      )}

      {modal === "set" && account && (
        <SetBalanceModal
          account={account}
          onClose={() => setModal("none")}
          onExplainWithTransactions={() => setModal("reconcile")}
        />
      )}
      {modal === "reconcile" && account && (
        <ReconcileBalanceModal account={account} onClose={() => setModal("none")} />
      )}
    </div>
  );
}

function BackLink({ onBack }: { onBack: () => void }) {
  return (
    <button
      type="button"
      onClick={onBack}
      className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
    >
      <ArrowLeft className="size-4" aria-hidden /> Accounts
    </button>
  );
}

/// The Activity table (ADR 0053, personal-cfo-wxy7): a pinned, projected "Upcoming"
/// block above the server-paged "Posted" rows.
///
/// The upcoming rows are `leadingRow` content — pinned, never paged, and projections
/// rather than rows the table pages over — while the posted rows go through the column
/// defs, so the primitive owns the loading / empty / error treatments and the section
/// rows' span. Before this, an error rendered ABOVE the table while the body still said
/// "No posted transactions on this account yet", which reads as "there are none" when
/// the truth is "we could not load them".
function ActivityTable({
  account,
  upcoming,
  postedRows,
  postedTotal,
  postedError,
  pagination,
  runningBalances,
}: {
  account: AccountViewDto;
  upcoming: UpcomingRow[];
  postedRows: TransactionRowDto[];
  /// `undefined` until the first page lands — the same loading signal Transactions uses.
  postedTotal: number | undefined;
  postedError: string | null;
  pagination: Pagination<never>;
  runningBalances: Map<string, number>;
}) {
  const currency = account.balance.currency;
  const columns: DataTableColumn<TransactionRowDto>[] = [
    {
      key: "date",
      header: "Date",
      className: "whitespace-nowrap tabular-nums text-muted-foreground",
      cell: (t) => formatIsoDate(t.occurred_at.slice(0, 10)),
    },
    {
      key: "description",
      header: "Description",
      className: "font-medium",
      cell: (t) => t.memo ?? t.counterparty ?? "Transaction",
    },
    {
      key: "amount",
      header: "Amount",
      align: "right",
      className: "tabular-nums font-medium",
      // The gain/loss colour is per row, so it goes on a span rather than the column's
      // (shared) cell class.
      cell: (t) => (
        <span className={signedAmountClass(t.amount)}>{formatSignedMoney(t.amount)}</span>
      ),
    },
    {
      key: "balance",
      header: "Balance",
      align: "right",
      className: "tabular-nums text-muted-foreground",
      cell: (t) => {
        const balance = runningBalances.get(t.transaction_id);
        return balance === undefined
          ? "—"
          : formatMoney({
              minor_units: storedToShownMinor(account.cashflow_role, balance),
              currency,
            });
      },
    },
  ];

  return (
    <DataTable
      columns={columns}
      rows={postedRows}
      rowKey={(t) => t.transaction_id}
      label={`Activity on ${account.name}`}
      minWidth="min-w-[560px]"
      status={postedError ? "error" : postedTotal === undefined ? "loading" : "ready"}
      error={postedError}
      empty={{
        icon: Receipt,
        title: "No posted transactions on this account yet.",
      }}
      leadingRow={
        <>
          {upcoming.length > 0 && (
            <SectionRow label="Upcoming" note="projected" span={columns.length} />
          )}
          {upcoming.map((u) => (
            <TableRow key={u.key} className="bg-muted/40">
              <TableCell className="whitespace-nowrap tabular-nums text-muted-foreground">
                {formatIsoDate(u.date)}
              </TableCell>
              <TableCell>
                <span className="flex flex-wrap items-center gap-2">
                  <span className="font-medium text-muted-foreground">{u.label}</span>
                  <Badge variant="outline" className="text-[10px]">
                    Projected
                  </Badge>
                  {u.detail && (
                    <span className="text-xs text-muted-foreground">{u.detail}</span>
                  )}
                </span>
              </TableCell>
              <TableCell className="text-right tabular-nums text-muted-foreground">
                {u.amountMinor !== null
                  ? formatSignedMoney({ minor_units: u.amountMinor, currency })
                  : "—"}
              </TableCell>
              <TableCell className="text-right text-muted-foreground">—</TableCell>
            </TableRow>
          ))}
          <SectionRow label="Posted" note={null} span={columns.length} />
        </>
      }
      footer={<PaginationControls pagination={pagination} noun="transactions" />}
    />
  );
}

function SectionRow({
  label,
  note,
  span,
}: {
  label: string;
  note: string | null;
  /// Derived from the column list rather than written out, so adding a column can never
  /// leave a section header misaligned.
  span: number;
}) {
  return (
    <TableRow>
      <TableCell colSpan={span} className="border-b-0 pb-1 pt-4">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
          {label}
        </span>
        {note && <span className="ml-2 text-[11px] text-muted-foreground">· {note}</span>}
      </TableCell>
    </TableRow>
  );
}

/// The card stats strip: statement balance, payment due (with a days-away badge), and the
/// credit-limit utilization bar — from the same projection the chart draws.
function CardStatsStrip({
  account,
  card,
  todayIso,
}: {
  account: AccountViewDto;
  card: NonNullable<ReturnType<typeof useCardStatementForecast>["cards"]>[number];
  todayIso: string;
}) {
  const cycle = card.cycles[0];
  const owedShown = storedToShownMinor(account.cashflow_role, account.balance.minor_units);
  const limit = card.credit_limit_minor;
  const utilization = limit > 0 ? Math.min(100, Math.max(0, (owedShown / limit) * 100)) : null;
  const dueDays =
    cycle && todayIso
      ? Math.round(
          (Date.parse(cycle.due_date) - Date.parse(todayIso)) / 86_400_000,
        )
      : null;
  return (
    <div className="flex flex-wrap items-center gap-x-8 gap-y-3 rounded-lg border border-border bg-card px-5 py-4">
      {cycle && (
        <div>
          <p className="text-[11px] text-muted-foreground">
            Statement balance{cycle.statement_is_actual ? "" : " (est.)"}
          </p>
          <p className="text-sm font-semibold tabular-nums">
            {formatMoney({
              minor_units: cycle.statement_balance_minor,
              currency: account.balance.currency,
            })}
          </p>
        </div>
      )}
      {cycle && (
        <div>
          <p className="text-[11px] text-muted-foreground">Payment due</p>
          <p className="flex items-center gap-2 text-sm font-semibold">
            {formatIsoDate(cycle.due_date)}
            {dueDays !== null && dueDays >= 0 && (
              <Badge variant={dueDays <= 5 ? "warning" : "secondary"}>
                in {dueDays} {dueDays === 1 ? "day" : "days"}
              </Badge>
            )}
          </p>
        </div>
      )}
      {utilization !== null && (
        <div className="min-w-52 flex-1">
          <div className="flex items-baseline justify-between">
            <p className="text-[11px] text-muted-foreground">
              Credit limit{" "}
              {formatMoney({ minor_units: limit, currency: account.balance.currency })}
            </p>
            <p className="text-[11px] font-semibold">{utilization.toFixed(0)}% used</p>
          </div>
          <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-secondary">
            <div
              className="h-full rounded-full bg-[var(--chart-1)]"
              style={{ width: `${utilization}%` }}
            />
          </div>
          <p className="mt-1.5 text-[11px] text-muted-foreground">
            {formatMoney({
              minor_units: Math.max(0, limit - owedShown),
              currency: account.balance.currency,
            })}{" "}
            available
          </p>
        </div>
      )}
    </div>
  );
}
