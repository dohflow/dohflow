import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts";

import type { DebtPayoffPlanDto } from "@/bindings";
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";
import { compactMoney, monthsToDuration } from "@/lib/format";
import { foldDebts } from "./debtBands";

/// The four categorical chart slots, in order (ADR 0054). Assigned per debt, never
/// cycled — see `colorFor` below.
const DEBT_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
];
const MAX_MONTHS = 360;
const DEFAULT_WINDOW = 120;

/// A CSS-var-safe series key (account labels may contain spaces).
const seriesKey = (index: number) => `debt${index}`;

/// The per-debt burndown (personal-cfo-6wk.17): a stacked area of each debt's owed balance over
/// time for one paydown plan — you can see which debt clears when, and the bands sum to the total.
/// Descriptive only (ADR 0018). Consumes the plan's per-debt series from the payoff comparison.
export function DebtPerDebtChart({
  plan,
  currency,
}: {
  plan: DebtPayoffPlanDto;
  currency: string;
}) {
  const debts = foldDebts(plan.per_debt, DEBT_COLORS.length);
  const months = plan.monthly_total_owed_minor.length;
  if (debts.length === 0 || months < 2) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        Not enough data to break the paydown down per debt yet.
      </p>
    );
  }

  const horizon = Math.min(
    MAX_MONTHS,
    plan.debt_free_month ?? Math.min(months - 1, DEFAULT_WINDOW),
  );
  // One row per month: { month, debt0, debt1, … } in minor units (a cleared debt holds at 0).
  const data = Array.from({ length: horizon + 1 }, (_, month) => {
    const point: Record<string, number> = { month };
    debts.forEach((d, i) => {
      point[seriesKey(i)] = d.owed[month] ?? 0;
    });
    return point;
  });

  const config: ChartConfig = {};
  debts.forEach((d, i) => {
    // In order, never cycled — `foldDebts` guarantees there are never more bands than
    // slots, so this index is always in range (ADR 0054).
    config[seriesKey(i)] = { label: d.label, color: DEBT_COLORS[i] };
  });

  return (
    <figure
      className="flex flex-col gap-2"
      aria-label={`Owed balance per debt over time: ${debts.map((d) => d.label).join(", ")}`}
    >
      <ChartContainer config={config} className="h-56">
        <AreaChart data={data} margin={{ left: 8, right: 12, top: 8, bottom: 0 }}>
          <CartesianGrid vertical={false} stroke="var(--border)" strokeOpacity={0.6} />
          <XAxis
            dataKey="month"
            type="number"
            domain={[0, horizon]}
            allowDecimals={false}
            tickLine={false}
            axisLine={false}
            tickMargin={8}
            minTickGap={40}
            tick={{ fontSize: 11 }}
            tickFormatter={(value) => monthsToDuration(Number(value))}
          />
          <YAxis
            tickLine={false}
            axisLine={false}
            width={56}
            tick={{ fontSize: 11 }}
            tickFormatter={(value) => compactMoney(Number(value), currency)}
          />
          <ChartTooltip
            cursor={{ stroke: "var(--border)" }}
            content={
              <ChartTooltipContent
                currency={currency}
                labelFormatter={(label) => monthsToDuration(Number(label))}
              />
            }
          />
          {debts.map((_, i) => (
            <Area
              key={seriesKey(i)}
              dataKey={seriesKey(i)}
              type="monotone"
              stackId="debts"
              stroke={`var(--color-${seriesKey(i)})`}
              fill={`var(--color-${seriesKey(i)})`}
              fillOpacity={0.5}
              isAnimationActive={false}
            />
          ))}
        </AreaChart>
      </ChartContainer>
      <figcaption className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {debts.map((d, i) => (
          <span key={seriesKey(i)} className="inline-flex items-center gap-1.5">
            <span
              className="inline-block size-2 rounded-full"
              style={{ backgroundColor: `var(--color-${seriesKey(i)})` }}
              aria-hidden
            />
            {d.label}
          </span>
        ))}
      </figcaption>
    </figure>
  );
}
