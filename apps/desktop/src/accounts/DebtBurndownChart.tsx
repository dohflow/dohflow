import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts";

import type { DebtPayoffPlanDto } from "@/bindings";
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";
import { compactMoney, monthsToDuration } from "@/lib/format";
import { STRATEGY_LABEL, STRATEGY_ORDER, type Strategy } from "./debtStrategies";

/// Per-strategy line colour — distinct brand chart tokens so the three paydown paths read apart.
const STRATEGY_COLOR: Record<Strategy, string> = {
  minimum_only: "var(--chart-4)",
  snowball: "var(--chart-2)",
  avalanche: "var(--chart-1)",
};

/// Chart until the slowest strategy that *clears* reaches zero (so every debt-free point shows),
/// capped at 30 years; if none clear, show a 10-year window.
const MAX_MONTHS = 360;
const DEFAULT_WINDOW = 120;

/// The debt-burndown chart (personal-cfo-6wk.5): total owed balance over time, one line per
/// paydown strategy, from the payoff engine's aggregate trajectory (6wk.16). Descriptive only
/// (ADR 0018) — no line is labelled best/recommended; the reader compares the shapes. Per-debt
/// stacking is a follow-on (6wk.17).
export function DebtBurndownChart({
  plans,
  currency,
}: {
  plans: DebtPayoffPlanDto[];
  currency: string;
}) {
  const present = STRATEGY_ORDER.filter((s) =>
    plans.some((p) => p.strategy === s),
  );
  const trajByStrategy = new Map(
    plans.map((p) => [p.strategy, p.monthly_total_owed_minor]),
  );
  const longest = Math.max(
    0,
    ...present.map((s) => trajByStrategy.get(s)?.length ?? 0),
  );

  if (present.length === 0 || longest < 2) {
    return (
      <p className="py-8 text-center text-sm text-muted-foreground">
        Not enough data to chart the paydown yet.
      </p>
    );
  }

  const clearing = plans
    .map((p) => p.debt_free_month)
    .filter((m): m is number => m !== null);
  const horizon = Math.min(
    MAX_MONTHS,
    clearing.length ? Math.max(...clearing) : DEFAULT_WINDOW,
  );

  // One row per month: { month, minimum_only, snowball, avalanche } in minor units. A strategy
  // that has cleared (its trajectory ended) holds at 0 for the remaining months.
  const data = Array.from({ length: horizon + 1 }, (_, month) => {
    const point: Record<string, number> = { month };
    for (const s of present) {
      point[s] = trajByStrategy.get(s)?.[month] ?? 0;
    }
    return point;
  });

  const config: ChartConfig = {};
  for (const s of present) {
    config[s] = { label: STRATEGY_LABEL[s], color: STRATEGY_COLOR[s] };
  }

  return (
    <figure
      className="flex flex-col gap-2"
      aria-label={`Total debt owed over time under ${present
        .map((s) => STRATEGY_LABEL[s])
        .join(", ")}`}
    >
      <ChartContainer config={config} className="h-64">
        <LineChart data={data} margin={{ left: 8, right: 12, top: 8, bottom: 0 }}>
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
          {present.map((s) => (
            <Line
              key={s}
              dataKey={s}
              type="monotone"
              stroke={`var(--color-${s})`}
              strokeWidth={1.75}
              dot={false}
              activeDot={{ r: 4 }}
              isAnimationActive={false}
            />
          ))}
        </LineChart>
      </ChartContainer>
      <figcaption className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
        {present.map((s) => (
          <span key={s} className="inline-flex items-center gap-1.5">
            <span
              className="inline-block size-2 rounded-full"
              style={{ backgroundColor: `var(--color-${s})` }}
              aria-hidden
            />
            {STRATEGY_LABEL[s]}
          </span>
        ))}
      </figcaption>
    </figure>
  );
}
