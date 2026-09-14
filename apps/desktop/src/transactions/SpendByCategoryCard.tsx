import { ChevronLeft, PieChart } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { RankedBars, type RankedBar } from "@/components/ui/ranked-bars";
import { formatIsoDate, formatMoney } from "@/lib/format";
import { useBaseCurrency } from "@/settings/useBaseCurrency";

import type { TransactionFilters } from "./filters";
import { DEFAULT_SPEND_DAYS, useSpendByCategory } from "./useSpendByCategory";

/// The spend-by-category breakdown above the transaction list (ADR 0052,
/// personal-cfo-4d8.27.8.4).
///
/// Descriptive only (ADR 0018): it states magnitudes and the range they cover, and never
/// judges them. `copy-review.test.ts` scans this directory precisely because spend copy is
/// where a verdict would creep in.
export function SpendByCategoryCard({
  filters,
  trail,
  onDrill,
  onBack,
}: {
  filters: TransactionFilters;
  /// The drill path; the deepest entry is also the list's category filter.
  trail: { id: string; name: string }[];
  onDrill: (id: string, name: string) => void;
  onBack: () => void;
}) {
  const parentId = trail[trail.length - 1]?.id ?? null;
  const { rows, excluded, error, range } = useSpendByCategory(filters, parentId);
  const { baseCurrency } = useBaseCurrency();

  // "Uncategorized" is a list sentinel, not a category. The aggregate counts only
  // EXPENSE-categorized amounts (ADR 0052 §4), so it has nothing to say about these rows
  // — and drawing the full breakdown beside a list narrowed to uncategorized would be the
  // chart and the list describing different sets, which §2 forbids. Say so instead.
  if (filters.categoryId === "uncategorized") {
    return (
      <Card>
        <CardContent className="py-4">
          <p className="text-sm text-muted-foreground">
            Uncategorized transactions have no category breakdown. Categorize some to see
            them here.
          </p>
        </CardContent>
      </Card>
    );
  }

  // Categories with nothing against them in this range would be bars of length zero —
  // noise that pushes the real ones down. The list still shows their transactions if any
  // exist outside the range; this chart is about where money went inside it.
  const bars: RankedBar[] = (rows ?? [])
    .filter((row) => row.total_minor !== 0)
    .map((row) => ({
      key: row.category_id,
      label: row.name,
      value: row.total_minor,
      drillable: row.has_children,
    }));
  const total = bars.reduce((sum, bar) => sum + bar.value, 0);

  const status = error ? "error" : rows === null ? "loading" : "ready";
  const here = trail[trail.length - 1];

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between gap-4 pb-2">
        <div className="min-w-0">
          <CardTitle className="text-base">
            {here ? `Spending in ${here.name}` : "Where the money went"}
          </CardTitle>
          {/* The range is stated, always. The list defaults to no date bound, so without
              this the user would be reading a 90-day total with nothing saying so
              (ADR 0052 §4). */}
          <p className="mt-0.5 text-xs text-muted-foreground">
            {range.isDefault
              ? `The last ${DEFAULT_SPEND_DAYS} days, through ${formatIsoDate(range.to)}`
              : `${formatIsoDate(range.from)} to ${formatIsoDate(range.to)}`}
            {" · "}
            {/* There is no offline FX, so the aggregate is scoped to one currency
                (ADR 0052 §4) — say which, or a foreign-currency row in the list below
                silently contributes nothing here. */}
            {`amounts in ${baseCurrency}`}
            {status === "ready" && bars.length > 0 && (
              <>
                {" · "}
                {formatMoney({ minor_units: total, currency: baseCurrency })} total
              </>
            )}
          </p>
        </div>
        {trail.length > 0 && (
          <Button variant="ghost" size="sm" onClick={onBack}>
            <ChevronLeft aria-hidden className="size-4" />
            {trail.length === 1 ? "All categories" : trail[trail.length - 2]!.name}
          </Button>
        )}
      </CardHeader>
      <CardContent>
        <RankedBars
          bars={bars}
          currency={baseCurrency}
          status={status}
          // Names WHICH thing failed. The aggregate is a separate read from the list, so it
          // can fail while the vault is perfectly readable — and on a local-first finance
          // app "your data might be damaged" and "one computation failed" call for
          // completely different reactions from the user. The raw message follows so a
          // report still carries the detail.
          error={
            error
              ? `Couldn’t total this range. The vault is readable — this aggregate failed. (${error})`
              : null
          }
          label={
            here
              ? `Spending by subcategory of ${here.name}, ${formatIsoDate(range.from)} to ${formatIsoDate(range.to)}`
              : `Spending by category, ${formatIsoDate(range.from)} to ${formatIsoDate(range.to)}`
          }
          empty={{
            icon: PieChart,
            title: "No spending in this range",
            description:
              "Categorized expenses in the selected period appear here, largest first.",
          }}
          // Selecting a bar narrows the list to that category's subtree AND drills the
          // chart into its children — one gesture, both views (ADR 0052 §2).
          onSelect={(bar) => onDrill(bar.key, bar.label)}
        />
        {/* Why this total is not the list's total. The chart is expenses-only and cannot
            place an uncategorized row or a transfer between the household's own accounts —
            so the two legitimately differ, and without saying so the reader has no way to
            tell a rule from a bug (ADR 0052 §4). The figures come from the SAME query that
            excluded them, so the explanation cannot drift from the chart. */}
        {status === "ready" && excluded !== null && (
          <p className="mt-3 text-xs text-muted-foreground">
            {here
              ? `Expenses only, inside ${here.name}. Split transactions count only the lines that fall here.`
              : excluded.uncategorizedMinor === 0 && excluded.transfersMinor === 0
                ? "Expenses only."
                : `Expenses only. Excludes ${[
                    excluded.uncategorizedMinor > 0
                      ? `${formatMoney({ minor_units: excluded.uncategorizedMinor, currency: baseCurrency })} uncategorized`
                      : null,
                    excluded.transfersMinor > 0
                      ? `${formatMoney({ minor_units: excluded.transfersMinor, currency: baseCurrency })} moved between your own accounts`
                      : null,
                  ]
                    .filter(Boolean)
                    .join(" and ")}.`}
          </p>
        )}
      </CardContent>
    </Card>
  );
}
