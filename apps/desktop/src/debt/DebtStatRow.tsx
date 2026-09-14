import { Card, CardContent } from "@/components/ui/card";
import { formatMoney } from "@/lib/format";

import { debtStats, type ScopedDebt } from "./debtStats";

/// A stat and its one-line qualifier. `value` is already formatted; `note` says what the
/// figure rests on or leaves out, which is the difference between a number and a claim.
function Stat({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <span className="text-lg font-semibold tabular-nums">{value}</span>
      {note !== undefined && (
        <span className="text-xs text-muted-foreground">{note}</span>
      )}
    </div>
  );
}

/// What the debts in scope cost right now (personal-cfo-g43x, from the Debt Page mock).
///
/// Every figure is derived in `debtStats` and only formatted here — the arithmetic is where
/// a plausible-but-wrong number hides, so it lives in a tested pure module rather than in
/// JSX.
///
/// Descriptive per ADR 0018: it states rates, totals and dates. It does not rank a debt or
/// suggest what to pay.
export function DebtStatRow({
  debts,
  currency,
}: {
  debts: ScopedDebt[];
  currency: string;
}) {
  const stats = debtStats(debts, currency);
  if (debts.length === 0) return null;

  const money = (minor: number) => formatMoney({ minor_units: minor, currency });
  // An unknown total says so. A dash that could be mistaken for zero would be worse than
  // no figure at all on a surface about what debt costs.
  const UNKNOWN = "Not known";

  return (
    <Card>
      <CardContent className="grid grid-cols-2 gap-x-6 gap-y-4 p-4 sm:grid-cols-3 lg:grid-cols-5">
        <Stat
          label="Total owed"
          value={money(stats.totalOwedMinor)}
          note={
            stats.otherCurrency > 0
              ? `${stats.otherCurrency} debt${stats.otherCurrency === 1 ? "" : "s"} in another currency not counted`
              : undefined
          }
        />
        <Stat
          label="Average rate"
          value={stats.aprBps === null ? UNKNOWN : `${(stats.aprBps / 100).toFixed(2)}%`}
          note={
            stats.aprExcluded > 0
              ? `${stats.aprExcluded} without a rate on record, left out`
              : "Weighted by balance"
          }
        />
        <Stat
          label="Minimum payments"
          value={stats.minimumsMinor === null ? UNKNOWN : money(stats.minimumsMinor)}
          note={
            stats.minimumsMinor === null
              ? "A debt has no terms recorded"
              : "This month, in total"
          }
        />
        <Stat
          label="Interest accruing"
          value={stats.interestMinor === null ? UNKNOWN : money(stats.interestMinor)}
          note={
            stats.interestMinor === null
              ? "A debt has no rate recorded"
              : "This month, at current rates"
          }
        />
        <Stat
          label="Next payment due"
          value={stats.nextDue === null ? UNKNOWN : `Day ${stats.nextDue.day}`}
          note={
            stats.nextDue === null
              ? "No due dates recorded"
              : stats.nextDue.count > 1
                ? `${stats.nextDue.count} debts share this day`
                : "Of the month"
          }
        />
      </CardContent>
    </Card>
  );
}
