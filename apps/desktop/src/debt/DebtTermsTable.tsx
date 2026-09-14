import type { CardStatementForecastDto } from "@/bindings";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { DataTable, type DataTableColumn } from "@/components/ui/data-table";
import { formatMoney } from "@/lib/format";

import {
  monthlyPaymentMinor,
  termRemainingMonths,
  type ScopedDebt,
} from "./debtStats";

/// Rendered when a figure cannot be derived from what is on record.
///
/// Deliberately words, not a dash: on a table of rates and payments an em-dash reads as
/// zero, and "no rate recorded" and "0%" are different facts (the same distinction
/// `debt_terms_list` preserves by omitting termless accounts).
const UNKNOWN = <span className="text-muted-foreground">Not recorded</span>;

function months(n: number | null) {
  if (n === null) return null;
  if (n < 12) return `${n} mo`;
  const years = Math.floor(n / 12);
  const rest = n % 12;
  return rest === 0 ? `${years} yr` : `${years} yr ${rest} mo`;
}

/// Amounts owed, rates and dates for the debts in scope (personal-cfo-g43x, from the mock).
///
/// Consolidates what was previously split between the card-statement forecast and the
/// account editor — the terms that decide what a debt costs were only visible one account at
/// a time, in a drawer you had to open.
export function DebtTermsTable({
  debts,
  currency,
  cards,
}: {
  debts: ScopedDebt[];
  currency: string;
  /// Card statement forecasts, for the projected-statement column. A loan has no statement,
  /// and a card with no billing cycle recorded has none to project.
  cards?: CardStatementForecastDto[] | null;
}) {
  const money = (minor: number) => formatMoney({ minor_units: minor, currency });
  // The NEXT cycle's statement per card. Cycles arrive in date order, so the first is the
  // one about to close — a later one would answer a question nobody asked.
  const nextStatement = new Map<string, number>();
  for (const card of cards ?? []) {
    const next = card.cycles[0];
    if (next !== undefined) nextStatement.set(card.account_id, next.statement_balance_minor);
  }

  const columns: DataTableColumn<ScopedDebt>[] = [
    {
      key: "name",
      header: "Debt",
      cell: (d) => <span className="font-medium">{d.account.name}</span>,
    },
    {
      key: "owed",
      header: "Amount owed",
      align: "right",
      width: "min",
      // Positive amount owed, never a negative balance (ADR 0044 / balanceSign).
      cell: (d) => <span className="tabular-nums">{money(d.owedMinor)}</span>,
    },
    {
      key: "apr",
      header: "Rate",
      align: "right",
      width: "min",
      cell: (d) =>
        d.terms?.apr_bps == null ? (
          UNKNOWN
        ) : (
          <span className="tabular-nums">{(d.terms.apr_bps / 100).toFixed(2)}%</span>
        ),
    },
    {
      key: "close",
      header: "Statement closes",
      align: "right",
      width: "min",
      cell: (d) =>
        d.terms?.statement_close_day == null ? (
          UNKNOWN
        ) : (
          <span className="tabular-nums">Day {d.terms.statement_close_day}</span>
        ),
    },
    {
      key: "due",
      header: "Payment due",
      align: "right",
      width: "min",
      cell: (d) =>
        d.terms?.payment_due_day == null ? (
          UNKNOWN
        ) : (
          <span className="tabular-nums">Day {d.terms.payment_due_day}</span>
        ),
    },
    {
      key: "statement",
      header: "Projected statement",
      align: "right",
      width: "min",
      cell: (d) => {
        const projected = nextStatement.get(d.account.id);
        // A loan has no statement at all, which is different from a card whose cycle is not
        // recorded — but neither has a figure, and inventing one would be worse than
        // saying so.
        return projected === undefined ? (
          <span className="text-muted-foreground">—</span>
        ) : (
          <span className="tabular-nums">{money(projected)}</span>
        );
      },
    },
    {
      key: "payment",
      header: "Monthly payment",
      align: "right",
      width: "min",
      cell: (d) => {
        const payment = monthlyPaymentMinor(d);
        return payment === null ? UNKNOWN : <span className="tabular-nums">{money(payment)}</span>;
      },
    },
    {
      key: "term",
      header: "Term remaining",
      align: "right",
      width: "min",
      cell: (d) => {
        const label = months(termRemainingMonths(d));
        // "Not at this payment" rather than a number: when the payment does not exceed the
        // interest the debt never amortizes, and the formula's answer there is negative.
        return label === null ? (
          <span className="text-muted-foreground">Not at this payment</span>
        ) : (
          <span className="tabular-nums">{label}</span>
        );
      },
    },
  ];

  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-base">Balances and terms</CardTitle>
        <p className="text-sm text-muted-foreground">
          Amounts owed, rates and dates for the debts in scope.
        </p>
      </CardHeader>
      <CardContent className="p-0">
        <DataTable
          columns={columns}
          rows={debts}
          rowKey={(d) => d.account.id}
          label="Balances and terms for the debts in scope"
        />
      </CardContent>
    </Card>
  );
}
